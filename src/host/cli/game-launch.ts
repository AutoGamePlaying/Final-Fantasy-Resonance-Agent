/**
 * Ask Steam on the game machine's desktop to start the game through steam://rungameid/<appid>.
 * The request is written to <LAPTOP_ROOT>/session/request.json and picked up by the scheduled
 * task named in LAPTOP_SESSION_TASK, which runs in the logged-on user's desktop session.
 *
 *   node src/host/cli/game-launch.ts [--timeout <秒>] [--dry-run] [--machine-env <文件>]
 */

import { parseArgs } from "node:util";
import {
  SESSION_TASK,
  runInSession,
  sessionRequest,
  sessionTaskOf,
} from "../../io/live/machine.ts";
import { text, truthy, type Json } from "../../shared/index.ts";
import {
  assertGameStopped,
  loadConfiguredMachine,
  printDryRun,
  requireAppId,
  runCli,
} from "./common.ts";

const USAGE = `用法: node src/host/cli/game-launch.ts [--timeout <秒>] [--dry-run] [--machine-env <文件>]

在游戏机的桌面会话里用 steam://rungameid/<appid> 启动游戏,经 LAPTOP_SESSION_TASK 指定的计划任务
(默认 ${SESSION_TASK})。AppID 读 machine.env 的 FFR_APP_ID;游戏已经在运行时拒绝再启动。

参数:
  --timeout <秒>         等计划任务回报状态多久,默认 120 秒
  --dry-run              只打印启动请求,不连接游戏机
  --machine-env <文件>   游戏机配置,默认 machine/machine.env
  -h, --help             显示这段说明`;

interface Options {
  readonly timeout: number;
  readonly dryRun: boolean;
  readonly machineEnv?: string;
}

function parseTimeout(value: string | undefined): number {
  if (value === undefined) return 120;
  const seconds = Number(value);
  if (!Number.isFinite(seconds) || seconds <= 0) {
    throw new Error(`--timeout 需要一个正数秒数,收到 ${value}`);
  }
  return seconds;
}

function launchMessage(status: Json): string {
  const note = text(status.note)?.trim();
  if (truthy(status.already)) return note || "游戏已经开着";
  if (status.state === "error") return text(status.error)?.trim() || "计划任务报告启动失败";
  if (status.state === "refused") return note || "游戏机拒绝了启动请求";
  return note || "启动请求已交给计划任务";
}

async function run(argv: readonly string[]): Promise<number> {
  let parsed;
  try {
    parsed = parseArgs({
      args: [...argv],
      allowPositionals: false,
      options: {
        help: { type: "boolean", short: "h" },
        timeout: { type: "string" },
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
  let options: Options;
  try {
    options = {
      timeout: parseTimeout(values.timeout),
      dryRun: values["dry-run"] === true,
      machineEnv: values["machine-env"] ?? process.env.FFR_MACHINE_ENV,
    };
  } catch (error) {
    console.error(`错误:${error instanceof Error ? error.message : String(error)}\n`);
    console.error(USAGE);
    return 2;
  }
  const machine = await loadConfiguredMachine(options.machineEnv);
  const url = `steam://rungameid/${requireAppId(machine)}`;
  const request = sessionRequest(url, [], null, false);
  if (options.dryRun) {
    printDryRun([
      `目标:${machine.host} 的桌面会话`,
      `请求文件:${machine.root}/session/request.json`,
      `内容:${JSON.stringify(request)}`,
      `触发计划任务:schtasks.exe /run /tn ${sessionTaskOf(machine)}`,
      `等状态最多 ${options.timeout} 秒`,
    ]);
    return 0;
  }
  await assertGameStopped(machine);
  const status = await runInSession(machine, request, options.timeout);
  if (status.state === "error" || status.state === "refused") {
    throw new Error(launchMessage(status));
  }
  console.log(`${url}:${launchMessage(status)}`);
  return 0;
}

await runCli(async () => run(process.argv.slice(2)));
