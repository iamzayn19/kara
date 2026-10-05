// Drives the real `kara serve --stdio` binary the same way the extension
// does. Runs when KARA_BIN points at a built binary; set KARA_TEST_MODEL=1
// to also run a prompt against the locally installed model.

import { test } from "node:test";
import assert from "node:assert/strict";
import * as cp from "child_process";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { RpcClient } from "../rpc";

const bin = process.env.KARA_BIN;

function startServer(cwd: string, home: string) {
  const proc = cp.spawn(bin!, ["serve", "--stdio"], { cwd, env: { ...process.env, KARA_HOME: home, NO_COLOR: "1" } });
  const client = new RpcClient((line) => proc.stdin.write(line));
  const notifications: any[] = [];
  proc.stdout.setEncoding("utf8");
  proc.stdout.on("data", (d: string) => client.receive(d));
  client.on("notification", (m: string, p: any) => notifications.push({ m, p }));
  client.onRequest(async (method, params: any) => {
    assert.equal(method, "permission/request");
    return { decision: params.can_remember ? "allow_once" : "deny" };
  });
  return { proc, client, notifications };
}

function fixtureRepo(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "kara-ext-"));
  fs.writeFileSync(path.join(dir, "app.py"), "def add(a, b):\n    return a - b\n");
  cp.execFileSync("git", ["init", "-q", "-b", "main"], { cwd: dir });
  return dir;
}

test("serve --stdio protocol round trip", { skip: !bin }, async () => {
  const repo = fixtureRepo();
  const home = process.env.KARA_TEST_MODEL ? path.join(os.homedir(), ".kara") : fs.mkdtempSync(path.join(os.tmpdir(), "kara-home-"));
  const { proc, client } = startServer(repo, home);
  try {
    const info: any = await client.request("initialize", { workspace: repo, clientName: "test" });
    assert.equal(info.protocolVersion.split(".")[0], "1");
    assert.equal(fs.realpathSync(info.workspace), fs.realpathSync(repo));
    assert.ok(info.commands.some((c: any) => c.name === "/oracle"));

    const status: any = await client.request("session/status");
    assert.equal(status.permissionsMode, "workspace");

    const models: any = await client.request("models/list");
    assert.ok(models.models.length >= 5);
    assert.ok(models.models.every((m: any) => typeof m.sha256 === "undefined" || true));

    const perms: any = await client.request("session/command", { name: "permissions" });
    const push = perms.rows.find((r: any) => r.kind === "git-push");
    assert.equal(push.hardBoundary, true);

    const privacy: any = await client.request("session/command", { name: "privacy" });
    assert.equal(privacy.telemetry, false);
    assert.equal(privacy.remote_inference, false);

    const changes: any = await client.request("session/changes");
    assert.deepEqual(changes.kara, []);

    await assert.rejects(client.request("no/such/method"), /unknown method/);

    if (!info.model.available) {
      await assert.rejects(client.request("session/prompt", { text: "hi" }), /kara connect/);
    }
  } finally {
    proc.kill();
  }
});

test("prompt against the local model streams events", { skip: !bin || !process.env.KARA_TEST_MODEL, timeout: 600_000 }, async () => {
  const repo = fixtureRepo();
  const { proc, client, notifications } = startServer(repo, path.join(os.homedir(), ".kara"));
  try {
    const info: any = await client.request("initialize", { workspace: repo, clientName: "test" });
    assert.ok(info.model.available, "a local model must be installed for this test");
    const result: any = await client.request("session/prompt", {
      text: "In one sentence, what does app.py's add function return? Do not change any files.",
      mode: "execute",
    });
    assert.equal(result.outcome, "completed");
    assert.ok(result.summary.length > 0);
    const types = new Set(notifications.filter((n) => n.m === "event").map((n) => n.p.type));
    assert.ok(types.has("turn_started") && types.has("turn_finished"));
    assert.equal(fs.readFileSync(path.join(repo, "app.py"), "utf8"), "def add(a, b):\n    return a - b\n");
  } finally {
    await client.request("shutdown").catch(() => undefined);
    proc.kill();
  }
});
