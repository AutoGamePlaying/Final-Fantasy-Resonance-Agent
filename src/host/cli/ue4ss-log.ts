/**
 * Read UE4SS.log from the game machine. The game may still be running: Get-Content opens the
 * file for shared reading, which is why this command does not refuse while the game is open.
 *
 *   node src/host/cli/ue4ss-log.ts [--tag <标签>] [--dry-run] [--machine-env <文件>]
 */

import { parseArgs } from "node:util";
import { psQuote, remoteJoin, runPs, type Machine } from "../../io/live/machine.ts";
import { loadConfiguredMachine, printDryRun, runCli } from "./common.ts";

const USAGE = `用法: node src/host/cli/ue4ss-log.ts [--tag <标签>] [--dry-run] [--machine-env <文件>]

取游戏机上的 ue4ss/UE4SS.log。游戏在运行时也能读,用共享读不锁文件。

参数:
  --tag <标签>           只输出包含这个标签的行,例如 --tag "[FfrProbe]"
  --dry-run              只打印要读哪个文件,不连接游戏机
  --machine-env <文件>   游戏机配置,默认 machine/machine.env
  -h, --help             显示这段说明`;

interface Options {
  readonly tag?: string;
  readonly dryRun: boolean;
  readonly machineEnv?: string;
}

export function logPath(machine: Machine): string {
  return remoteJoin(machine.gameDir, "ue4ss", "UE4SS.log");
}

export function filterTag(content: string, tag: string | undefined): string {
  if (!tag) return content;
  const lines = content.split(/\r?\n/u).filter((line) => line.includes(tag));
  return lines.length > 0 ? `${lines.join("\n")}\n` : "";
}

function readScript(machine: Machine): string {
  const path = logPath(machine);
  return (
    `$path = ${psQuote(path)}\n` +
    'if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "找不到 UE4SS 日志:$path" }\n' +
    // Shared read: the game keeps the log open while it runs.
    "Get-Content -LiteralPath $path -Raw -Encoding UTF8\n"
  );
}

async function run(argv: readonly string[]): Promise<number> {
  let parsed;
  try {
    parsed = parseArgs({
      args: [...argv],
      allowPositionals: false,
      options: {
        help: { type: "boolean", short: "h" },
        tag: { type: "string" },
        "dry-run": { type: "boolean" },
        "machine-env": { type: "string" },
      },
    });
  } catch (error) {
    console.error(`错误:${error instanceof Error ? error.message : String(error)}\n`);
    console.error(USAGE);
    return 2;
  }
  const values = parsed.values;
  if (values.help === true) {
    console.log(USAGE);
    return 0;
  }
  if (values.tag === "") {
    console.error(`错误:--tag 不能是空字符串\n`);
    console.error(USAGE);
    return 2;
  }
  const options: Options = {
    tag: values.tag,
    dryRun: values["dry-run"] === true,
    machineEnv: values["machine-env"] ?? process.env.FFR_MACHINE_ENV,
  };
  const machine = await loadConfiguredMachine(options.machineEnv);
  if (options.dryRun) {
    printDryRun([
      `读取:${logPath(machine)}`,
      options.tag ? `只保留包含 ${options.tag} 的行` : "输出整个文件",
    ]);
    return 0;
  }
  const result = await runPs(machine, readScript(machine), { capture: true });
  const body = filterTag(result.stdout, options.tag);
  if (options.tag && body === "") {
    console.error(`没有找到带 ${options.tag} 的日志行`);
    return 0;
  }
  if (body) process.stdout.write(body.endsWith("\n") ? body : `${body}\n`);
  return 0;
}

await runCli(async () => run(process.argv.slice(2)));
