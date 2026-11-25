/**
 * Read-only structural survey CLI for exported FFR Datatable JSON files (docs/plan.md §3 R2,
 * docs/data-tables.md). Prints category counts, per-table RowStruct/rows/inferred field types,
 * StringTable/FText linkage, and cross-row reference field names without printing game text.
 *
 *   node src/host/cli/data-survey.ts [--dir <导出目录>] [--category <分类>] [--sample-rows <N>]
 *                                    [--summary] [--json]
 */

import { resolve } from "node:path";
import { parseArgs } from "node:util";
import { exportDir, formatSurveyReport, surveyExport } from "../../data/index.ts";
import { describeError } from "../../shared/index.ts";

const USAGE = `用法：node src/host/cli/data-survey.ts [选项]

只读统计导出的 FFR 数据表结构（分类表数、RowStruct、行数、字段类型、字符串表引用与行间引用字段）。
不修改任何文件，也不输出游戏正文内容。

选项：
  --dir <目录>          导出根目录，默认 $FFR_DEPS/ffr/export 或 .deps/ffr/export
  --category <分类>     只统计指定分类（例如 Scenario、Map、Skill）
  --sample-rows <数量>  每张表抽样推断字段类型的行数，默认 5
  --summary             省略第四节逐表字段明细，只输出概览、字符串与引用统计
  --json                以 JSON 格式输出完整统计结果
  -h, --help            显示这段说明`;

function parseSampleRows(raw: string | undefined): number {
  if (raw === undefined) return 5;
  if (!/^[1-9]\d*$/u.test(raw)) {
    throw new Error(`--sample-rows 必须是正整数，收到 ${raw}`);
  }
  return Number.parseInt(raw, 10);
}

export async function runDataSurvey(argv: readonly string[]): Promise<number> {
  let parsed;
  try {
    parsed = parseArgs({
      args: [...argv],
      allowPositionals: false,
      options: {
        help: { type: "boolean", short: "h" },
        dir: { type: "string" },
        category: { type: "string" },
        "sample-rows": { type: "string" },
        summary: { type: "boolean" },
        json: { type: "boolean" },
      },
    });
  } catch (error) {
    console.error(`错误：${describeError(error)}\n`);
    console.error(USAGE);
    return 2;
  }

  const values = parsed.values;
  if (values.help === true) {
    console.log(USAGE);
    return 0;
  }
  if (values.dir === "") {
    console.error("错误：--dir 不能为空\n");
    console.error(USAGE);
    return 2;
  }
  if (values.category === "") {
    console.error("错误：--category 不能为空\n");
    console.error(USAGE);
    return 2;
  }

  const sampleRows = parseSampleRows(values["sample-rows"]);
  const report = await surveyExport({
    exportRoot: values.dir ?? exportDir(),
    category: values.category,
    sampleRows,
  });

  if (values.json === true) {
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
    return 0;
  }

  process.stdout.write(formatSurveyReport(report, { includeTables: values.summary !== true }));
  return 0;
}

async function main(): Promise<void> {
  try {
    process.exitCode = await runDataSurvey(process.argv.slice(2));
  } catch (error) {
    console.error(`错误：${describeError(error)}`);
    process.exitCode = 1;
  }
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && resolve(invokedPath) === import.meta.filename) {
  await main();
}
