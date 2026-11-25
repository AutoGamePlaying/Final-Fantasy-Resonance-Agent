/**
 * Stream-survey a UE4SS `.jmap` dump into `.deps/r1/survey/` (JSON + Markdown).
 *
 *   node src/host/cli/jmap-survey.ts [jmap文件] [--filter <正则>] [--out <目录>] [--stdout]
 */

import { mkdir, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { r1Dir } from "../../shared/index.ts";
import {
  compileFilter,
  findLatestJmap,
  renderSurveyMarkdown,
  surveyJmapFile,
} from "../jmap-survey.ts";
import { runCli } from "./common.ts";

const USAGE = `用法: node src/host/cli/jmap-survey.ts [jmap文件] [--filter <正则>] [--out <目录>] [--stdout]

流式读取 UE4SS 导出的 .jmap 文件，提取：
  - /Script/mucha 下的类、结构体、枚举（父类、属性名/类型/偏移、函数名）
  - /Game/ 下的蓝图生成类（类名、父类、属性名/类型）
并按主题归类，默认写入 .deps/r1/survey/survey.json 与 survey.md。

参数:
  jmap文件               要分析的 .jmap 路径；省略时自动在 .deps/r1/ 下找最新一份
  --filter <正则>        只保留名字或路径匹配该正则的条目（默认忽略大小写）
  --out <目录>           输出目录，默认 <FFR_DEPS 或 .deps>/r1/survey
  --stdout               直接向标准输出打印 Markdown，不写入文件
  -h, --help             显示这段说明

例:
  node src/host/cli/jmap-survey.ts
  node src/host/cli/jmap-survey.ts --filter "Battle|Party" --stdout
  node src/host/cli/jmap-survey.ts .deps/r1/20261124-125820-title-jmap/FFRS-5.6.1-0+UE5-30afe5ff.jmap`;

interface CliOptions {
  readonly inputPath?: string;
  readonly filter?: RegExp;
  readonly outDir: string;
  readonly stdout: boolean;
}

async function run(argv: readonly string[]): Promise<number> {
  let parsed;
  try {
    parsed = parseArgs({
      args: [...argv],
      allowPositionals: true,
      options: {
        help: { type: "boolean", short: "h" },
        filter: { type: "string" },
        out: { type: "string" },
        stdout: { type: "boolean" },
      },
    });
  } catch (error) {
    console.error(`错误:${error instanceof Error ? error.message : String(error)}\n`);
    console.error(USAGE);
    return 2;
  }

  const { values, positionals } = parsed;
  if (values.help === true) {
    console.log(USAGE);
    return 0;
  }

  let options: CliOptions;
  try {
    if (positionals.length > 1) {
      throw new Error(`最多接受一个 jmap 文件路径，收到 ${positionals.length} 个`);
    }
    const filter = values.filter !== undefined ? compileFilter(values.filter) : undefined;
    options = {
      ...(positionals[0] !== undefined ? { inputPath: resolve(positionals[0]) } : {}),
      ...(filter !== undefined ? { filter } : {}),
      outDir: values.out ? resolve(values.out) : join(r1Dir(), "survey"),
      stdout: values.stdout === true,
    };
  } catch (error) {
    console.error(`错误:${error instanceof Error ? error.message : String(error)}\n`);
    console.error(USAGE);
    return 2;
  }

  const inputPath = options.inputPath ?? (await findLatestJmap(r1Dir()));
  const report = await surveyJmapFile(inputPath, {
    ...(options.filter ? { filter: options.filter } : {}),
  });
  const markdown = renderSurveyMarkdown(report);

  if (options.stdout) {
    process.stdout.write(markdown);
    return 0;
  }

  await mkdir(options.outDir, { recursive: true });
  const jsonPath = join(options.outDir, "survey.json");
  const mdPath = join(options.outDir, "survey.md");
  await writeFile(jsonPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
  await writeFile(mdPath, markdown, "utf8");

  console.log(`源文件:${report.sourceFile ?? "-"}`);
  console.log(
    `对象:${report.stats.totalObjects}（解析 ${report.stats.parsedObjects}，跳过 ${report.stats.skippedObjects}）`,
  );
  console.log(
    `mucha:类 ${report.stats.muchaClasses}/${report.stats.muchaClassesTotal}，结构体 ${report.stats.muchaStructs}/${report.stats.muchaStructsTotal}，枚举 ${report.stats.muchaEnums}/${report.stats.muchaEnumsTotal}`,
  );
  console.log(`蓝图类:${report.stats.blueprintClasses}/${report.stats.blueprintClassesTotal}`);
  console.log(`输出:${jsonPath}`);
  console.log(`输出:${mdPath}`);
  return 0;
}

await runCli(async () => run(process.argv.slice(2)));
