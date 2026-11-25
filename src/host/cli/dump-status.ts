/**
 * Read the FfrDump module's heartbeat, the pending trigger and the recent job directories.
 * Read-only: the game may be running.
 *
 *   node src/host/cli/dump-status.ts [--dry-run] [--machine-env <文件>]
 */

import { parseArgs } from "node:util";
import { isGameRunning, psQuote, remoteJoin, runPs, type Machine } from "../../io/live/machine.ts";
import { isRecord } from "../../shared/index.ts";
import { loadConfiguredMachine, printDryRun, runCli } from "./common.ts";
import {
  heartbeatPath,
  hotkeyDir,
  outDir,
  parseHeartbeat,
  parseJsonDocument,
  triggerPath,
  type Heartbeat,
} from "./dump.ts";

const USAGE = `用法: node src/host/cli/dump-status.ts [--dry-run] [--machine-env <文件>]

只读查看 FfrDump 的状态:游戏是否在运行、心跳年龄和 busy、有没有待处理的 trigger.txt、
最近 10 个任务目录和它们的 done.json / ok,以及热键快照和标记数量。游戏在不在运行都可以执行。

参数:
  --dry-run              只打印要读哪些文件,不连接游戏机
  --machine-env <文件>   游戏机配置,默认 machine/machine.env
  -h, --help             显示这段说明`;

interface Options {
  readonly dryRun: boolean;
  readonly machineEnv?: string;
}

interface JobStatus {
  readonly id: string;
  readonly done: boolean;
  readonly ok: boolean | null;
}

interface HotkeyStatus {
  readonly snapshots: number;
  readonly markers: number;
}

interface Status {
  readonly heartbeat: Heartbeat | null;
  readonly now: number;
  readonly trigger: boolean;
  readonly jobs: readonly JobStatus[];
  readonly hotkey: HotkeyStatus;
}

export function statusScript(machine: Machine): string {
  const heartbeat = heartbeatPath(machine);
  const trigger = triggerPath(machine);
  const jobs = remoteJoin(outDir(machine), "jobs");
  const hotkey = hotkeyDir(machine);
  return (
    `$heartbeatPath = ${psQuote(heartbeat)}\n` +
    `$triggerPath = ${psQuote(trigger)}\n` +
    `$jobsPath = ${psQuote(jobs)}\n` +
    `$hotkeyPath = ${psQuote(hotkey)}\n` +
    "$heartbeat = $null\n" +
    "if (Test-Path -LiteralPath $heartbeatPath -PathType Leaf) {\n" +
    "  try { $heartbeat = Get-Content -LiteralPath $heartbeatPath -Raw -Encoding UTF8 | " +
    "ConvertFrom-Json } catch { }\n" +
    "}\n" +
    "$pending = Test-Path -LiteralPath $triggerPath -PathType Leaf\n" +
    "$jobs = @()\n" +
    "if (Test-Path -LiteralPath $jobsPath -PathType Container) {\n" +
    "  $jobs = @(Get-ChildItem -LiteralPath $jobsPath -Directory | " +
    "Sort-Object Name -Descending | Select-Object -First 10 | ForEach-Object {\n" +
    "    $donePath = Join-Path $_.FullName 'done.json'\n" +
    "    $done = Test-Path -LiteralPath $donePath -PathType Leaf\n" +
    "    $ok = $null\n" +
    "    if ($done) {\n" +
    "      try {\n" +
    "        $parsed = Get-Content -LiteralPath $donePath -Raw -Encoding UTF8 | ConvertFrom-Json\n" +
    "        if ($null -ne $parsed.ok) { $ok = [bool]$parsed.ok }\n" +
    "      } catch { }\n" +
    "    }\n" +
    "    [pscustomobject]@{ id = $_.Name; done = $done; ok = $ok }\n" +
    "  })\n" +
    "}\n" +
    "$hotkeySnapshots = 0\n" +
    "$hotkeyMarkers = 0\n" +
    "if (Test-Path -LiteralPath $hotkeyPath -PathType Container) {\n" +
    "  $hotkeyDone = @(Get-ChildItem -LiteralPath $hotkeyPath -File -Force | " +
    "Where-Object { $_.Name -match '^hotkey-\\d{4,}-\\d{8}-\\d{6}-done\\.json$' })\n" +
    "  $hotkeySnapshots = $hotkeyDone.Count\n" +
    "  $markersPath = Join-Path $hotkeyPath 'markers.jsonl'\n" +
    "  if (Test-Path -LiteralPath $markersPath -PathType Leaf) {\n" +
    "    $fs = [IO.File]::Open($markersPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, " +
    "[IO.FileShare]::ReadWrite)\n" +
    "    try {\n" +
    "      $sr = New-Object IO.StreamReader($fs, [Text.Encoding]::UTF8)\n" +
    "      while ($null -ne ($line = $sr.ReadLine())) {\n" +
    "        if ($line.Trim() -ne '') { $hotkeyMarkers++ }\n" +
    "      }\n" +
    "    } finally { $fs.Dispose() }\n" +
    "  }\n" +
    "}\n" +
    "$now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()\n" +
    "ConvertTo-Json @{ heartbeat = $heartbeat; now = $now; trigger = $pending; jobs = $jobs; " +
    "hotkey = @{ snapshots = $hotkeySnapshots; markers = $hotkeyMarkers } } " +
    "-Compress -Depth 6\n"
  );
}

function nonNegativeInt(value: unknown, label: string): number {
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0) {
    throw new Error(`dump-status 的结果里 ${label} 不是非负整数`);
  }
  return value;
}

function parseHotkeyStatus(input: unknown): HotkeyStatus {
  if (input === null || input === undefined) return { snapshots: 0, markers: 0 };
  if (!isRecord(input)) throw new Error("dump-status 的结果里 hotkey 不是对象");
  return {
    snapshots: nonNegativeInt(input.snapshots, "hotkey.snapshots"),
    markers: nonNegativeInt(input.markers, "hotkey.markers"),
  };
}

function parseStatus(stdout: string): Status {
  const value = parseJsonDocument(stdout, "dump-status 的结果");
  if (!isRecord(value)) throw new Error("dump-status 的结果不是对象");
  const now = value.now;
  if (typeof now !== "number" || !Number.isFinite(now)) {
    throw new Error("dump-status 的结果里 now 不是数字");
  }
  if (typeof value.trigger !== "boolean") {
    throw new Error("dump-status 的结果里 trigger 不是布尔值");
  }
  const heartbeat =
    value.heartbeat === null || value.heartbeat === undefined
      ? null
      : parseHeartbeat(value.heartbeat);
  if (!Array.isArray(value.jobs)) {
    throw new Error("dump-status 的结果里 jobs 不是数组");
  }
  const jobs = value.jobs.map((item, index) => {
    if (!isRecord(item)) throw new Error(`dump-status 的结果里 jobs[${index}] 不是对象`);
    const id = item.id;
    if (typeof id !== "string") {
      throw new Error(`dump-status 的结果里 jobs[${index}].id 不是字符串`);
    }
    if (typeof item.done !== "boolean") {
      throw new Error(`dump-status 的结果里 jobs[${index}].done 不是布尔值`);
    }
    const ok = item.ok;
    if (ok !== null && typeof ok !== "boolean") {
      throw new Error(`dump-status 的结果里 jobs[${index}].ok 不是布尔值或 null`);
    }
    return { id, done: item.done, ok };
  });
  const hotkey = parseHotkeyStatus(value.hotkey);
  return { heartbeat, now, trigger: value.trigger, jobs, hotkey };
}

function printStatus(running: readonly string[], status: Status): void {
  const game = running.length > 0 ? `正在运行(${running.join("、")})` : "没在运行";
  console.log(`游戏:${game}`);
  if (status.heartbeat === null) {
    console.log("心跳:没有(模组没在运行,或者还没写 heartbeat.json)");
  } else {
    const age = Math.max(0, Math.round(status.now - status.heartbeat.time));
    console.log(`心跳:${age} 秒前,busy=${status.heartbeat.busy}`);
  }
  console.log(`待处理触发文件:${status.trigger ? "有" : "没有"}`);
  console.log(`热键快照:${status.hotkey.snapshots} 个,标记:${status.hotkey.markers} 条`);
  console.log(`最近任务:${status.jobs.length} 个`);
  for (const job of status.jobs) {
    console.log(
      `  ${job.id} done=${job.done ? "有" : "没有"} ok=${job.ok === null ? "?" : String(job.ok)}`,
    );
  }
}

async function run(argv: readonly string[]): Promise<number> {
  let parsed;
  try {
    parsed = parseArgs({
      args: [...argv],
      allowPositionals: false,
      options: {
        help: { type: "boolean", short: "h" },
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
  const options: Options = {
    dryRun: values["dry-run"] === true,
    machineEnv: values["machine-env"] ?? process.env.FFR_MACHINE_ENV,
  };
  const machine = await loadConfiguredMachine(options.machineEnv);
  if (options.dryRun) {
    printDryRun([
      `心跳:${heartbeatPath(machine)}`,
      `触发文件:${triggerPath(machine)}`,
      `任务目录:${remoteJoin(outDir(machine), "jobs")}`,
      `热键目录:${hotkeyDir(machine)}`,
      "只读,不写游戏机",
    ]);
    return 0;
  }
  const running = await isGameRunning(machine);
  const status = parseStatus(
    (await runPs(machine, statusScript(machine), { capture: true })).stdout,
  );
  printStatus(running, status);
  return 0;
}

if (import.meta.main) {
  await runCli(async () => run(process.argv.slice(2)));
}
