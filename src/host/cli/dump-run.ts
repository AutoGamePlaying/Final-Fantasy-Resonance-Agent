/**
 * Trigger one R1 dump job on the game machine and wait for the FfrDump module's done marker.
 *
 * The game must be running: the module reads live game state in the game thread. All waiting
 * happens inside one PowerShell script on the machine, so the dev box does not open a new SSH
 * connection every second.
 *
 *   node src/host/cli/dump-run.ts <job> --label <场面名> [--timeout <秒>] [--no-fetch]
 *                                  [--dry-run] [--machine-env <文件>]
 */

import { parseArgs } from "node:util";
import { isGameRunning, psQuote, remoteJoin, runPs, type Machine } from "../../io/live/machine.ts";
import { isRecord } from "../../shared/index.ts";
import { loadConfiguredMachine, printDryRun, runCli } from "./common.ts";
import { fetchDump } from "./dump-fetch.ts";
import {
  JOB_NAMES,
  defaultTimeout,
  heartbeatPath,
  hotkeyDir,
  isJob,
  jobDir,
  jobId,
  localJobDir,
  outDir,
  parseDone,
  parseHeartbeat,
  parseJsonDocument,
  parseLabel,
  remotePackage,
  triggerPath,
  triggerText,
  type Job,
} from "./dump.ts";

const USAGE = `用法: node src/host/cli/dump-run.ts <job> --label <场面名> [--timeout <秒>] [--no-fetch]
                                  [--dry-run] [--machine-env <文件>]

要求游戏正在运行。在游戏机上建好输出目录、写 trigger.txt,由 FfrDump 模组在游戏线程里执行,
等 done.json 出现后打印结果;ok:true 时默认接着把文件拉回 .deps/r1/<任务号>/。

任务(job)和默认等待:
  snapshot  场面快照              180 秒
  objects   DumpAllObjects        300 秒
  usmap     DumpUSMAP             180 秒
  jmap      DumpJMAP              600 秒
  sdk       GenerateSDK           900 秒
  uht       GenerateUHTCompatibleHeaders 900 秒

参数:
  --label <场面名>       小写字母、数字和连字符,1 到 32 个字符,例如 field-1
  --timeout <秒>         等完成标记多久,默认按任务决定
  --no-fetch             只等完成标记,不拉回本地
  --dry-run              只打印任务号和触发文件,不连接游戏机
  --machine-env <文件>   游戏机配置,默认 machine/machine.env
  -h, --help             显示这段说明

例:
  node src/host/cli/dump-run.ts snapshot --label field-1
  node src/host/cli/dump-run.ts usmap --label field-1 --timeout 240`;

interface Options {
  readonly job: Job;
  readonly label: string;
  readonly timeout: number;
  readonly noFetch: boolean;
  readonly dryRun: boolean;
  readonly machineEnv?: string;
}

const NO_HEARTBEAT =
  "FfrDump 没有心跳：先 `mod-sync FfrDump` 启用并重启游戏，" +
  '再用 `ue4ss-log --tag "[FfrDump]"` 看加载日志';

function parseJob(value: string | undefined): Job {
  if (value === undefined) {
    throw new Error(`dump-run 需要任务名:${JOB_NAMES.join(", ")}`);
  }
  if (!isJob(value)) {
    throw new Error(`未知任务:${value};可用:${JOB_NAMES.join(", ")}`);
  }
  return value;
}

function parseTimeout(value: string | undefined): number {
  if (value === undefined) throw new Error("内部错误:没有默认超时");
  const seconds = Number(value);
  if (!Number.isFinite(seconds) || seconds <= 0) {
    throw new Error(`--timeout 需要一个正数秒数,收到 ${value}`);
  }
  return seconds;
}

export function prepareScript(machine: Machine, id: string): string {
  const out = outDir(machine);
  const hotkey = hotkeyDir(machine);
  const job = jobDir(machine, id);
  const trigger = triggerPath(machine);
  const heartbeat = heartbeatPath(machine);
  return (
    `$out = ${psQuote(out)}\n` +
    `$hotkey = ${psQuote(hotkey)}\n` +
    `$job = ${psQuote(job)}\n` +
    `$trigger = ${psQuote(trigger)}\n` +
    `$heartbeat = ${psQuote(heartbeat)}\n` +
    "New-Item -ItemType Directory -Force -Path $out | Out-Null\n" +
    "New-Item -ItemType Directory -Force -Path $hotkey | Out-Null\n" +
    'if (Test-Path -LiteralPath $trigger) { throw "上一个转储还没被模组取走:$trigger" }\n' +
    'if (Test-Path -LiteralPath $job) { throw "任务目录已存在:$job" }\n' +
    "$heartbeatValue = $null\n" +
    "$deadline = [DateTime]::UtcNow.AddSeconds(15)\n" +
    "while ([DateTime]::UtcNow -lt $deadline) {\n" +
    "  if (Test-Path -LiteralPath $heartbeat -PathType Leaf) {\n" +
    "    try {\n" +
    "      $candidate = Get-Content -LiteralPath $heartbeat -Raw -Encoding UTF8 | ConvertFrom-Json\n" +
    "      $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()\n" +
    "      $valid = ($candidate.mod -is [string]) -and ($candidate.mod -eq 'FfrDump') -and " +
    "($candidate.version -is [int] -or $candidate.version -is [long]) -and " +
    "($candidate.time -is [int] -or $candidate.time -is [long]) -and " +
    "($candidate.busy -is [bool])\n" +
    "      if ($valid -and [Math]::Abs([double]$candidate.time - $now) -lt 20) {\n" +
    "        $heartbeatValue = $candidate\n" +
    "        break\n" +
    "      }\n" +
    "    } catch { }\n" +
    "  }\n" +
    "  Start-Sleep -Milliseconds 1000\n" +
    "}\n" +
    "if ($null -ne $heartbeatValue) {\n" +
    '  if ($heartbeatValue.busy) { throw "FfrDump 正在执行上一个任务，用 dump-status 查看" }\n' +
    "  New-Item -ItemType Directory -Force -Path $job | Out-Null\n" +
    "}\n" +
    "$now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()\n" +
    "ConvertTo-Json @{ heartbeat = $heartbeatValue; now = $now } -Compress -Depth 5\n"
  );
}

export function triggerScript(machine: Machine, content: string): string {
  const trigger = triggerPath(machine);
  const temporary = remoteJoin(outDir(machine), "trigger.tmp");
  return (
    `$temporary = ${psQuote(temporary)}\n` +
    `$trigger = ${psQuote(trigger)}\n` +
    `$content = ${psQuote(content)}\n` +
    "[IO.File]::WriteAllText($temporary, $content, (New-Object Text.UTF8Encoding $false))\n" +
    "Move-Item -LiteralPath $temporary -Destination $trigger -Force\n"
  );
}

export function waitScript(machine: Machine, id: string, timeout: number): string {
  const job = jobDir(machine, id);
  const trigger = triggerPath(machine);
  return (
    `$job = ${psQuote(job)}\n` +
    `$trigger = ${psQuote(trigger)}\n` +
    "$done = Join-Path $job 'done.json'\n" +
    "$started = [DateTime]::UtcNow\n" +
    `$deadline = $started.AddSeconds(${timeout})\n` +
    "while (-not (Test-Path -LiteralPath $done -PathType Leaf)) {\n" +
    "  $elapsed = ([DateTime]::UtcNow - $started).TotalSeconds\n" +
    "  if ($elapsed -ge 15 -and (Test-Path -LiteralPath $trigger -PathType Leaf)) {\n" +
    "    Remove-Item -LiteralPath $trigger -Force\n" +
    '    throw "模组没有取走触发文件:15 秒后 trigger.txt 还在,已删除;' +
    // Single quotes inside the PowerShell double-quoted string: inner double quotes ended the
    // string early and `[FfrDump]` parsed as an index expression (11-24 live, ParserError).
    "用 ue4ss-log --tag '[FfrDump]' 看模组加载日志\"\n" +
    "  }\n" +
    "  if ([DateTime]::UtcNow -ge $deadline) {\n" +
    "    $note = ''\n" +
    "    if (Test-Path -LiteralPath $trigger -PathType Leaf) {\n" +
    "      Remove-Item -LiteralPath $trigger -Force\n" +
    "      $note = ';已删除还没被取走的触发文件'\n" +
    "    }\n" +
    `    throw "等完成标记超时:${timeout} 秒内没有出现 done.json$note"\n` +
    "  }\n" +
    "  Start-Sleep -Milliseconds 1000\n" +
    "}\n" +
    "Get-Content -LiteralPath $done -Raw -Encoding UTF8\n"
  );
}

function parseSetup(stdout: string): void {
  const value = parseJsonDocument(stdout, "准备阶段的结果");
  if (!isRecord(value)) throw new Error("准备阶段的结果不是对象");
  const now = value.now;
  if (typeof now !== "number" || !Number.isFinite(now)) {
    throw new Error("准备阶段的结果里 now 不是数字");
  }
  if (value.heartbeat === null || value.heartbeat === undefined) {
    throw new Error(NO_HEARTBEAT);
  }
  const heartbeat = parseHeartbeat(value.heartbeat);
  if (heartbeat.mod !== "FfrDump") {
    throw new Error(`心跳的 mod 不是 FfrDump:${heartbeat.mod}`);
  }
  if (heartbeat.busy) {
    throw new Error("FfrDump 正在执行上一个任务，用 dump-status 查看");
  }
  if (Math.abs(heartbeat.time - now) >= 20) {
    throw new Error(`${NO_HEARTBEAT};心跳时间和游戏机当前时间相差太大`);
  }
}

async function run(argv: readonly string[]): Promise<number> {
  let parsed;
  try {
    parsed = parseArgs({
      args: [...argv],
      allowPositionals: true,
      options: {
        help: { type: "boolean", short: "h" },
        label: { type: "string" },
        timeout: { type: "string" },
        "no-fetch": { type: "boolean" },
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
    if (parsed.positionals.length !== 1) {
      throw new Error("dump-run 需要一个任务名");
    }
    const job = parseJob(parsed.positionals[0]);
    const label = parseLabel(values.label);
    options = {
      job,
      label,
      timeout: values.timeout === undefined ? defaultTimeout(job) : parseTimeout(values.timeout),
      noFetch: values["no-fetch"] === true,
      dryRun: values["dry-run"] === true,
      machineEnv: values["machine-env"] ?? process.env.FFR_MACHINE_ENV,
    };
  } catch (error) {
    console.error(`错误:${error instanceof Error ? error.message : String(error)}\n`);
    console.error(USAGE);
    return 2;
  }
  const machine = await loadConfiguredMachine(options.machineEnv);
  const id = jobId(new Date(), options.label, options.job);
  const content = triggerText(id, options.job, options.label);
  if (options.dryRun) {
    printDryRun([
      `任务号:${id}`,
      `输出根目录:${outDir(machine)}`,
      `热键输出目录:${hotkeyDir(machine)}`,
      `任务目录:${jobDir(machine, id)}`,
      `触发文件:${triggerPath(machine)}`,
      "触发文件内容:",
      ...content
        .trimEnd()
        .split("\r\n")
        .map((line) => `  ${line}`),
      `等待:${options.timeout} 秒`,
      options.noFetch
        ? "完成后不拉取文件"
        : `完成后拉取到:${localJobDir(id)}(远端压缩包 ${remotePackage(machine, id)})`,
    ]);
    return 0;
  }
  const running = await isGameRunning(machine);
  if (running.length === 0) {
    throw new Error("游戏没在运行：先用 game-launch 启动并走到要转储的场面");
  }
  parseSetup((await runPs(machine, prepareScript(machine, id), { capture: true })).stdout);
  await runPs(machine, triggerScript(machine, content));
  const waited = await runPs(machine, waitScript(machine, id, options.timeout), {
    capture: true,
    signal: AbortSignal.timeout(Math.ceil((options.timeout + 60) * 1000)),
  });
  const done = parseDone(parseJsonDocument(waited.stdout, "done.json"));
  console.log(`ok:${done.ok}`);
  console.log(`error:${done.error ?? "-"}`);
  console.log(`耗时:${done.finishedAt - done.startedAt} 秒`);
  console.log(`stats:${JSON.stringify(done.stats)}`);
  if (!done.ok) return 1;
  if (!options.noFetch) await fetchDump(machine, id);
  return 0;
}

if (import.meta.main) {
  await runCli(async () => run(process.argv.slice(2)));
}
