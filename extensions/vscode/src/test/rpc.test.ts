import { test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { RpcClient, RpcError } from "../rpc";
import { archiveName, parseSha256Sums, releaseTarget, sha256File, which } from "../platform";

function pair() {
  const sent: any[] = [];
  const client = new RpcClient((line) => sent.push(JSON.parse(line)));
  return { client, sent };
}

test("requests resolve with matching responses, even when split across chunks", async () => {
  const { client, sent } = pair();
  const p = client.request("session/status");
  assert.equal(sent[0].method, "session/status");
  const reply = JSON.stringify({ jsonrpc: "2.0", id: sent[0].id, result: { ok: true } }) + "\n";
  client.receive(reply.slice(0, 10));
  client.receive(reply.slice(10));
  assert.deepEqual(await p, { ok: true });
});

test("errors reject with RpcError and code", async () => {
  const { client, sent } = pair();
  const p = client.request("session/prompt", { text: "x" });
  client.receive(JSON.stringify({ jsonrpc: "2.0", id: sent[0].id, error: { code: -32001, message: "busy" } }) + "\n");
  await assert.rejects(p, (e: unknown) => e instanceof RpcError && e.code === -32001);
});

test("notifications are emitted; multiple messages per chunk", () => {
  const { client } = pair();
  const got: string[] = [];
  client.on("notification", (method: string, params: any) => got.push(`${method}:${params.type}`));
  client.receive(
    JSON.stringify({ jsonrpc: "2.0", method: "event", params: { type: "phase" } }) +
      "\n" +
      JSON.stringify({ jsonrpc: "2.0", method: "event", params: { type: "assistant_delta" } }) +
      "\n",
  );
  assert.deepEqual(got, ["event:phase", "event:assistant_delta"]);
});

test("server-initiated requests are answered through the handler", async () => {
  const { client, sent } = pair();
  client.onRequest(async (method, params: any) => {
    assert.equal(method, "permission/request");
    return { decision: params.kinds.includes("git_push") ? "deny" : "allow_once" };
  });
  client.receive(JSON.stringify({ jsonrpc: "2.0", id: 7, method: "permission/request", params: { kinds: ["git_push"] } }) + "\n");
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(sent[0], { jsonrpc: "2.0", id: 7, result: { decision: "deny" } });
});

test("close rejects pending requests and refuses new ones", async () => {
  const { client } = pair();
  const p = client.request("x");
  client.close("gone");
  await assert.rejects(p, /gone/);
  await assert.rejects(client.request("y"), /not running/);
});

test("garbage lines are reported, not thrown", () => {
  const { client } = pair();
  const errs: string[] = [];
  client.on("protocolError", (e: string) => errs.push(e));
  client.receive("not json\n{\"id\":1}\n");
  assert.equal(errs.length, 2);
});

test("platform mapping and asset names", () => {
  assert.equal(releaseTarget("darwin", "arm64"), "aarch64-apple-darwin");
  assert.equal(releaseTarget("win32", "x64"), "x86_64-pc-windows-msvc");
  assert.equal(releaseTarget("freebsd", "x64"), undefined);
  assert.equal(archiveName("0.1.0", "x86_64-pc-windows-msvc"), "kara-v0.1.0-x86_64-pc-windows-msvc.zip");
  assert.equal(archiveName("0.1.0", "aarch64-apple-darwin"), "kara-v0.1.0-aarch64-apple-darwin.tar.gz");
});

test("SHA256SUMS parsing and hashing", async () => {
  const sums = parseSha256Sums(
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  kara-v0.1.0-x.tar.gz\nbogus line\n",
  );
  assert.equal(sums.get("kara-v0.1.0-x.tar.gz"), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
  const f = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "kara-")), "empty");
  fs.writeFileSync(f, "");
  assert.equal(await sha256File(f), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
  assert.equal(which(path.basename(f), path.dirname(f)), f);
  assert.equal(which("definitely-not-a-binary", path.dirname(f)), undefined);
});
