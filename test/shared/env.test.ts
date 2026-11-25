import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { parseEnvFile, readEnvFile } from "../../src/shared/env.ts";

void test("parseEnvFile skips comments and splits on the first =", () => {
  assert.deepEqual(
    parseEnvFile("# note\n\nA=1\nB = two words \nC=a=b\nD='quoted'\nE=\"quoted\"\n"),
    { A: "1", B: "two words", C: "a=b", D: "quoted", E: "quoted" },
  );
});

void test("parseEnvFile keeps a __proto__ key as data", () => {
  const values = parseEnvFile("__proto__=x\n");
  assert.equal(Object.getPrototypeOf(values), Object.prototype);
  assert.equal(values.__proto__, "x");
});

void test("readEnvFile returns an empty map when the file is missing", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-env-"));
  try {
    assert.deepEqual(await readEnvFile(join(directory, "absent.env")), {});
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("readEnvFile strips a UTF-8 BOM", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-env-"));
  try {
    const path = join(directory, "machine.env");
    await writeFile(path, "\uFEFFFFR_APP_ID=123\n", "utf8");
    assert.deepEqual(await readEnvFile(path), { FFR_APP_ID: "123" });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
