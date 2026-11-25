import assert from "node:assert/strict";
import { join, resolve } from "node:path";
import test from "node:test";
import { MACHINE_ENV, REPO, depsDir, r1Dir, runDir, ue4ssDir } from "../../src/shared/paths.ts";

void test("the repo is two levels above src/shared", () => {
  assert.equal(REPO, resolve(import.meta.dirname, "..", ".."));
});

void test("depsDir defaults to <repo>/.deps and honors FFR_DEPS", () => {
  assert.equal(depsDir({}), join(REPO, ".deps"));
  assert.equal(depsDir({ FFR_DEPS: "/custom-deps" }), "/custom-deps");
});

void test("ue4ssDir is <deps>/ue4ss", () => {
  assert.equal(ue4ssDir({}), join(REPO, ".deps", "ue4ss"));
  assert.equal(ue4ssDir({ FFR_DEPS: "/custom-deps" }), "/custom-deps/ue4ss");
});

void test("runDir is <deps>/run", () => {
  assert.equal(runDir({}), join(REPO, ".deps", "run"));
  assert.equal(runDir({ FFR_DEPS: "/custom-deps" }), "/custom-deps/run");
});

void test("r1Dir is <deps>/r1", () => {
  assert.equal(r1Dir({}), join(REPO, ".deps", "r1"));
  assert.equal(r1Dir({ FFR_DEPS: "/custom-deps" }), "/custom-deps/r1");
});

void test("MACHINE_ENV points at the uncommitted machine settings", () => {
  assert.equal(MACHINE_ENV, join(REPO, "machine", "machine.env"));
});
