import { test } from "node:test";
import assert from "node:assert/strict";
import { listUsers } from "../src/query.ts";

const users = Array.from({ length: 5 }, (_, i) => ({ id: i + 1, name: `u${i + 1}` }));

test("invalid page parameters fall back to page 1", () => {
  const page = listUsers(users, { page: "abc", perPage: "2" });
  assert.equal(page.page, 1);
  assert.deepEqual(page.items.map((u) => u.id), [1, 2]);
});

test("perPage is capped at 100 and at least 1", () => {
  assert.equal(listUsers(users, { perPage: "1000" }).perPage, 100);
  assert.equal(listUsers(users, { perPage: "0" }).perPage, 1);
});
