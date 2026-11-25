/**
 * Tests for generic CUE4Parse DataTable, CompositeDataTable, StringTable loading and UE path
 * mapping against synthetic fixtures.
 */

import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  datatableDir,
  loadDataTable,
  loadStringTable,
  normalizeRowStructName,
  parseDataTableDocument,
  parseStringTableDocument,
  ueObjectPathToExportJson,
} from "../../src/data/index.ts";

const FIXTURE_EXPORT = join(import.meta.dirname, "..", "fixtures", "data", "export");

await test("loadDataTable 读取普通 DataTable 并规范化 RowStruct 与行映射", async () => {
  const path = join(datatableDir(FIXTURE_EXPORT), "Scenario", "DT_ChapterData.json");
  const table = await loadDataTable(path);
  assert.equal(table.type, "DataTable");
  assert.equal(table.name, "DT_ChapterData");
  assert.equal(table.rowStruct, "ChapterData");
  assert.equal(table.rows.size, 3);
  assert.equal(table.rows.get("Ch1")?.ChapterNo, 1);
});

await test("loadDataTable 支持 CompositeDataTable", async () => {
  const path = join(datatableDir(FIXTURE_EXPORT), "Map", "CDT_MapData_Prod.json");
  const table = await loadDataTable(path);
  assert.equal(table.type, "CompositeDataTable");
  assert.equal(table.rowStruct, "MAP_DATA");
  assert.equal(table.rows.size, 2);
  assert.equal(table.rows.get("Map_Alpha")?.ID, 2001);
});

await test("loadStringTable 读取命名空间和词条映射", async () => {
  const path = join(
    datatableDir(FIXTURE_EXPORT),
    "Strings",
    "ChapterTitle",
    "ST_ChapterTitle.json",
  );
  const table = await loadStringTable(path);
  assert.equal(table.tableNamespace, "ST_ChapterTitle");
  assert.equal(table.entries.size, 3);
  assert.equal(table.entries.get("Title_001"), "はじまりの森");
});

await test("loadDataTable 遇到缺文件、非法 JSON 或类型不符时抛出中文错误", async () => {
  await assert.rejects(loadDataTable(join(FIXTURE_EXPORT, "non-existent.json")), /读取数据表失败/u);

  const stPath = join(
    datatableDir(FIXTURE_EXPORT),
    "Strings",
    "ChapterTitle",
    "ST_ChapterTitle.json",
  );
  await assert.rejects(loadDataTable(stPath), /不是 DataTable 或 CompositeDataTable/u);

  const dir = await mkdtemp(join(tmpdir(), "ffr-dt-bad-"));
  try {
    const brokenPath = join(dir, "broken.json");
    await writeFile(brokenPath, "{ not-json", "utf8");
    await assert.rejects(loadDataTable(brokenPath), /JSON 格式错误/u);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

await test("parseDataTableDocument 与 parseStringTableDocument 校验必填字段", () => {
  assert.throws(() => parseDataTableDocument([], "empty.json"), /不是非空 JSON 数组/u);
  assert.throws(
    () =>
      parseDataTableDocument(
        [{ Type: "DataTable", Name: "DT_X", Package: "/Game/X", Properties: {}, Rows: {} }],
        "no-struct.json",
      ),
    /RowStruct\.ObjectName/u,
  );
  assert.throws(
    () =>
      parseStringTableDocument(
        [{ Type: "DataTable", Name: "DT_X", Package: "/Game/X" }],
        "wrong-st.json",
      ),
    /不是 StringTable/u,
  );
  assert.equal(normalizeRowStructName("Class'ChapterData'"), "ChapterData");
  assert.equal(normalizeRowStructName("PlainStruct"), "PlainStruct");
});

await test("ueObjectPathToExportJson 映射合法 /Game 路径并拒绝越界路径", () => {
  assert.equal(
    ueObjectPathToExportJson(
      "/Game/Datatable/Strings/ChapterTitle/ST_ChapterTitle.ST_ChapterTitle",
      "/tmp/export",
    ),
    join(
      "/tmp/export",
      "FFRS",
      "Content",
      "Datatable",
      "Strings",
      "ChapterTitle",
      "ST_ChapterTitle.json",
    ),
  );
  assert.equal(
    ueObjectPathToExportJson("/Game/Datatable/Scenario/DT_ChapterData", "/tmp/export"),
    join("/tmp/export", "FFRS", "Content", "Datatable", "Scenario", "DT_ChapterData.json"),
  );
  assert.equal(ueObjectPathToExportJson("/Script/mucha.Foo", "/tmp/export"), null);
  assert.equal(ueObjectPathToExportJson("/Game/Datatable/../Secret.Secret", "/tmp/export"), null);
});
