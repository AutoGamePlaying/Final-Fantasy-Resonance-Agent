import assert from "node:assert/strict";
import { test } from "node:test";
import { describeError, isMissingFile } from "../../src/shared/errors.ts";

void test("describeError keeps the message and stringifies other values", () => {
  assert.equal(describeError(new Error("读不了")), "读不了");
  assert.equal(describeError("出错了"), "出错了");
  assert.equal(describeError(42), "42");
});

void test("isMissingFile only accepts ENOENT errors", () => {
  const missing = Object.assign(new Error("gone"), { code: "ENOENT" });
  const denied = Object.assign(new Error("no"), { code: "EACCES" });
  assert.equal(isMissingFile(missing), true);
  assert.equal(isMissingFile(denied), false);
  assert.equal(isMissingFile(new Error("plain")), false);
  assert.equal(isMissingFile("x"), false);
});
