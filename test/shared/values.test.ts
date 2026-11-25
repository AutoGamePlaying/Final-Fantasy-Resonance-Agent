import assert from "node:assert/strict";
import { test } from "node:test";
import { isRecord, list, num, record, records, text, truthy } from "../../src/shared/values.ts";

void test("isRecord accepts objects only", () => {
  assert.equal(isRecord({}), true);
  assert.equal(isRecord({ a: 1 }), true);
  assert.equal(isRecord([]), false);
  assert.equal(isRecord(null), false);
  assert.equal(isRecord("x"), false);
});

void test("truthiness follows Python for empty lists and dicts", () => {
  for (const falsy of [null, undefined, false, 0, "", [], {}]) assert.equal(truthy(falsy), false);
  for (const value of [1, "a", [0], { a: 1 }, true]) assert.equal(truthy(value), true);
});

void test("record and list fall back the way `or {}` and `or []` do", () => {
  assert.deepEqual(record(null), {});
  assert.deepEqual(record([1]), {});
  assert.deepEqual(record({ a: 1 }), { a: 1 });
  assert.deepEqual(list(undefined), []);
  assert.deepEqual(list({}), []);
  assert.deepEqual(records([{ a: 1 }, 2, null, { b: 2 }]), [{ a: 1 }, { b: 2 }]);
});

void test("text and num only accept their own type", () => {
  assert.equal(text("x"), "x");
  assert.equal(text(1), undefined);
  assert.equal(num(3), 3);
  assert.equal(num(Number.NaN), undefined);
  assert.equal(num("3"), undefined);
});
