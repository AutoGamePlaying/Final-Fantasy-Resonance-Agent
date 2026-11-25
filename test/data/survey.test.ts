/**
 * Tests for the read-only export survey library and the data-survey CLI on synthetic fixtures.
 */

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { join } from "node:path";
import test from "node:test";
import { formatSurveyReport, surveyExport } from "../../src/data/index.ts";

const FIXTURE_EXPORT = join(import.meta.dirname, "..", "fixtures", "data", "export");
const CLI_PATH = join(import.meta.dirname, "..", "..", "src", "host", "cli", "data-survey.ts");

function runSurveyCli(args: readonly string[]): Promise<{
  readonly code: number;
  readonly stdout: string;
  readonly stderr: string;
}> {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(process.execPath, [CLI_PATH, ...args], {
      stdio: ["ignore", "pipe", "pipe"],
    });
    const out: Buffer[] = [];
    const err: Buffer[] = [];
    child.stdout.on("data", (chunk: Buffer) => out.push(chunk));
    child.stderr.on("data", (chunk: Buffer) => err.push(chunk));
    child.on("error", reject);
    child.on("close", (code) => {
      resolvePromise({
        code: code ?? 1,
        stdout: Buffer.concat(out).toString("utf8"),
        stderr: Buffer.concat(err).toString("utf8"),
      });
    });
  });
}

await test("surveyExport 统计分类、表结构、字符串表引用和跨表引用字段", async () => {
  const report = await surveyExport({ exportRoot: FIXTURE_EXPORT });
  assert.equal(report.totalFiles, 7);
  assert.equal(report.objectTypes.DataTable, 2);
  assert.equal(report.objectTypes.CompositeDataTable, 1);
  assert.equal(report.objectTypes.StringTable, 4);

  const categoryNames = report.categories.map((cat) => cat.name);
  assert.deepEqual(categoryNames, ["Map", "Scenario", "Strings"]);

  const chapterTable = report.tables.find((table) =>
    table.relativePath.endsWith("DT_ChapterData.json"),
  );
  assert.ok(chapterTable !== undefined);
  assert.equal(chapterTable.rowStruct, "ChapterData");
  assert.equal(chapterTable.rowCount, 3);
  assert.deepEqual(chapterTable.fields, [
    { name: "ChapterNo", types: ["number"] },
    { name: "Name", types: ["FText"] },
  ]);

  assert.equal(report.strings.stringTableCount, 4);
  assert.equal(report.strings.referencedTableIdCount, 4);
  assert.deepEqual(report.strings.missingTableIds, []);
  assert.equal(report.strings.missingKeyCount, 0);
  assert.deepEqual(report.strings.exportedLocalizationLanguages, ["en", "ja", "zh-Hans"]);

  const refNames = new Set(report.referenceFields.map((ref) => ref.fieldName));
  assert.ok(refNames.has("ID"));
  assert.ok(refNames.has("Condition"));
  assert.ok(refNames.has("DestinationId"));
  assert.ok(refNames.has("EventId"));
  assert.ok(refNames.has("encountGroupId"));

  const rendered = formatSurveyReport(report);
  assert.match(rendered, /按分类统计（共 3 类）/u);
  assert.match(rendered, /DT_ChapterData\.json/u);
});

await test("data-survey 命令行支持 --dir、--category、--json、--help 与错误退出码", async () => {
  const help = await runSurveyCli(["--help"]);
  assert.equal(help.code, 0);
  assert.match(help.stdout, /用法：node src\/host\/cli\/data-survey\.ts/u);

  const jsonRun = await runSurveyCli(["--dir", FIXTURE_EXPORT, "--category", "Scenario", "--json"]);
  assert.equal(jsonRun.code, 0);
  const parsed = JSON.parse(jsonRun.stdout) as {
    totalFiles: number;
    tables: { rowStruct: string }[];
  };
  assert.equal(parsed.totalFiles, 2);
  assert.deepEqual(
    parsed.tables.map((t) => t.rowStruct),
    ["ChapterData", "ScenarioSection"],
  );

  const summaryRun = await runSurveyCli(["--dir", FIXTURE_EXPORT, "--summary"]);
  assert.equal(summaryRun.code, 0);
  assert.match(summaryRun.stdout, /文件总数：7/u);
  assert.doesNotMatch(summaryRun.stdout, /## 四、各表结构/u);

  const badArg = await runSurveyCli(["--unknown"]);
  assert.equal(badArg.code, 2);

  const missingDir = await runSurveyCli(["--dir", join(FIXTURE_EXPORT, "no-such-dir")]);
  assert.equal(missingDir.code, 1);
  assert.match(missingDir.stderr, /找不到数据表导出目录/u);
});
