/**
 * Pull one finished R1 dump from the game machine into .deps/r1/<id>/.
 *
 * The game may still be running: every read on the machine is a shared read, and the only writes
 * are files.txt, meta.json, a copy of UE4SS.log and the temporary .tgz under <LAPTOP_ROOT>/ffr-dumps.
 * UE4SS's own dump files and the job directory stay on the game machine.
 *
 *   node src/host/cli/dump-fetch.ts <id> [--dry-run] [--machine-env <文件>]
 */

import { spawn } from "node:child_process";
import { mkdir, readFile, readdir, rename, rm, rmdir, stat, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { psQuote, remoteJoin, runPs, runScp, target, type Machine } from "../../io/live/machine.ts";
import { isRecord } from "../../shared/index.ts";
import { loadConfiguredMachine, printDryRun, runCli } from "./common.ts";
import {
  buildIdFromAcf,
  jobDir,
  localJobDir,
  parseDone,
  parseJobId,
  parseJsonDocument,
  remotePackage,
} from "./dump.ts";

const USAGE = `用法: node src/host/cli/dump-fetch.ts <id> [--dry-run] [--machine-env <文件>]

把游戏机上已经完成的 R1 转储拉到 .deps/r1/<id>/。游戏在不在运行都可以执行。
本地目录已存在时拒绝,不覆盖。拉回后把 ue4ss/ffr-out/jobs/<id>/ 里的任务文件摊到本地
目录根上,UE4SS 自己的转储留在 ue4ss/ 下。

参数:
  --dry-run              只打印要读哪些文件,不连接游戏机
  --machine-env <文件>   游戏机配置,默认 machine/machine.env
  -h, --help             显示这段说明

例:
  node src/host/cli/dump-fetch.ts 20261124-031500-field-1-snapshot`;

interface Options {
  readonly id: string;
  readonly dryRun: boolean;
  readonly machineEnv?: string;
}

export interface FetchSummary {
  readonly localDir: string;
  readonly files: number;
  readonly bytes: number;
  readonly buildid: string | null;
}

/** The last step on the machine: delete only the temporary .tgz named by this id. */
export function deleteRemoteScript(machine: Machine, id: string): string {
  const packagePath = remotePackage(machine, id);
  return (
    `$package = ${psQuote(packagePath)}\n` +
    "if (Test-Path -LiteralPath $package -PathType Leaf) { " +
    "Remove-Item -LiteralPath $package -Force }\n"
  );
}

export function fetchScript(machine: Machine, id: string): string {
  const game = machine.gameDir;
  const job = jobDir(machine, id);
  const packagePath = remotePackage(machine, id);
  const appId = machine.appId?.trim() ?? "";
  return (
    `$game = ${psQuote(game)}\n` +
    `$gameFull = [IO.Path]::GetFullPath($game).TrimEnd([char]'\\', [char]'/')\n` +
    `$job = ${psQuote(job)}\n` +
    `$package = ${psQuote(packagePath)}\n` +
    `$appId = ${psQuote(appId)}\n` +
    "function Get-RelativePathFromGame {\n" +
    "  param([string]$Path)\n" +
    "  $full = [IO.Path]::GetFullPath($Path)\n" +
    "  return $full.Substring($gameFull.Length + 1).Replace('\\', '/')\n" +
    "}\n" +
    "$donePath = Join-Path $job 'done.json'\n" +
    "if (-not (Test-Path -LiteralPath $donePath -PathType Leaf)) { " +
    'throw "找不到完成标记:$donePath" }\n' +
    // ReadAllText avoids PowerShell 5.1 Get-Content -Raw note properties (PSPath etc.) that
    // make ConvertTo-Json serialise strings as objects instead of plain JSON strings.
    "$doneText = [IO.File]::ReadAllText($donePath, [Text.Encoding]::UTF8)\n" +
    "try { $done = $doneText | ConvertFrom-Json } catch { " +
    'throw "done.json 不是合法 JSON:$donePath" }\n' +
    "if ($null -eq $done.startedAt -or $null -eq $done.finishedAt) { " +
    'throw "done.json 缺少 startedAt 或 finishedAt" }\n' +
    "$started = [double]$done.startedAt\n" +
    "$finished = [double]$done.finishedAt\n" +
    // UE4SS.log is copied with Get-Content so a running game does not block the read.
    "$logCopied = $false\n" +
    "$logPath = Join-Path $game 'ue4ss/UE4SS.log'\n" +
    "if (Test-Path -LiteralPath $logPath -PathType Leaf) {\n" +
    "  $logText = Get-Content -LiteralPath $logPath -Raw -Encoding UTF8\n" +
    "  if ($null -eq $logText) { $logText = '' }\n" +
    "  [IO.File]::WriteAllText((Join-Path $job 'UE4SS.log'), $logText, " +
    "(New-Object Text.UTF8Encoding $false))\n" +
    "  $logCopied = $true\n" +
    "}\n" +
    "$entries = [System.Collections.Generic.List[object]]::new()\n" +
    "$seen = [System.Collections.Generic.HashSet[string]]::new()\n" +
    "function Add-Entry {\n" +
    "  param([System.IO.FileInfo]$Item)\n" +
    "  $relative = Get-RelativePathFromGame $Item.FullName\n" +
    "  if ($seen.Add($relative)) {\n" +
    "    [void]$entries.Add([pscustomobject]@{ Path = $relative; Length = $Item.Length })\n" +
    "  }\n" +
    "}\n" +
    "Get-ChildItem -LiteralPath $job -File -Recurse -Force | Sort-Object FullName | " +
    "ForEach-Object {\n" +
    "  if (-not ($_.Name -ieq 'files.txt' -or $_.Name -ieq 'meta.json')) { Add-Entry $_ }\n" +
    "}\n" +
    "$min = $started - 2\n" +
    "$max = $finished + 30\n" +
    "Get-ChildItem -LiteralPath $game -File -Recurse -Force | Sort-Object FullName | " +
    "ForEach-Object {\n" +
    "  $relative = Get-RelativePathFromGame $_.FullName\n" +
    "  $excluded = ($relative -match '^ue4ss/Mods/') -or ($relative -match '^ue4ss/ffr-out/') -or " +
    "($relative -ieq 'ue4ss/UE4SS.log')\n" +
    "  if (-not $excluded) {\n" +
    "    $seconds = [DateTimeOffset]::new($_.LastWriteTimeUtc).ToUnixTimeSeconds()\n" +
    "    if ($seconds -ge $min -and $seconds -le $max) { Add-Entry $_ }\n" +
    "  }\n" +
    "}\n" +
    "$count = $entries.Count\n" +
    "$totalBytes = [long](($entries | Measure-Object -Property Length -Sum).Sum)\n" +
    "$acfText = $null\n" +
    "if ($appId -ne '') {\n" +
    '  $acf = [IO.Path]::GetFullPath((Join-Path $game "../../../../../appmanifest_$appId.acf"))\n' +
    "  if (Test-Path -LiteralPath $acf -PathType Leaf) {\n" +
    "    $acfText = [IO.File]::ReadAllText($acf, [Text.Encoding]::UTF8)\n" +
    "  }\n" +
    "}\n" +
    "$meta = [ordered]@{\n" +
    "  id = [string]$done.id\n" +
    "  job = [string]$done.job\n" +
    "  label = [string]$done.label\n" +
    "  startedAt = $started\n" +
    "  finishedAt = $finished\n" +
    "  files = $count\n" +
    "  bytes = $totalBytes\n" +
    "  buildid = $null\n" +
    "}\n" +
    "$metaPath = Join-Path $job 'meta.json'\n" +
    "[IO.File]::WriteAllText($metaPath, (ConvertTo-Json $meta -Compress), " +
    "(New-Object Text.UTF8Encoding $false))\n" +
    "[void]$entries.Add([pscustomobject]@{ Path = (Get-RelativePathFromGame $metaPath); " +
    "Length = (Get-Item -LiteralPath $metaPath).Length })\n" +
    "$newline = [string][char]10\n" +
    "$listText = (($entries | ForEach-Object { $_.Path }) -join $newline) + $newline\n" +
    "$listPath = Join-Path $job 'files.txt'\n" +
    "[IO.File]::WriteAllText($listPath, $listText, (New-Object Text.UTF8Encoding $false))\n" +
    "$tar = Join-Path $env:SystemRoot 'System32/tar.exe'\n" +
    'if (-not (Test-Path -LiteralPath $tar -PathType Leaf)) { throw "找不到系统自带的 tar.exe:$tar" }\n' +
    "$packageDir = Split-Path -Parent $package\n" +
    "if (-not (Test-Path -LiteralPath $packageDir -PathType Container)) { " +
    "New-Item -ItemType Directory -Force -Path $packageDir | Out-Null }\n" +
    "& $tar -czf $package -C $game -T $listPath\n" +
    'if ($LASTEXITCODE -ne 0) { throw "打包失败:tar.exe 退出码 $LASTEXITCODE" }\n' +
    "ConvertTo-Json @{ doneText = $doneText; files = $count; bytes = $totalBytes; " +
    "acf = $acfText; logCopied = $logCopied } -Compress -Depth 6\n"
  );
}

async function pathExists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") return false;
    throw error;
  }
}

/** Extract package.tgz into the destination and then delete the archive. */
export async function unpackPackage(
  packagePath: string,
  destination: string,
  signal?: AbortSignal,
): Promise<void> {
  await mkdir(destination, { recursive: true });
  await new Promise<void>((resolve, reject) => {
    const child = spawn("tar", ["-xzf", packagePath, "-C", destination], {
      stdio: ["ignore", "pipe", "pipe"],
      signal,
    });
    const stderr: Buffer[] = [];
    child.stderr.on("data", (chunk: Buffer) => stderr.push(chunk));
    child.on("error", (error) => {
      reject(
        new Error(`解包失败:${error instanceof Error ? error.message : String(error)}`, {
          cause: error,
        }),
      );
    });
    child.on("close", (code) => {
      if (code === 0) resolve();
      else {
        const reason = Buffer.concat(stderr).toString("utf8").trim();
        reject(new Error(`解包失败:tar 退出码 ${code ?? -1}${reason ? `:${reason}` : ""}`));
      }
    });
  });
  await rm(packagePath, { force: true });
}

function nonNegativeNumber(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0) {
    throw new Error(`dump-fetch 的结果里 ${field} 不是非负整数`);
  }
  return value;
}

function nullableText(value: unknown, field: string): string | null {
  if (value === null || value === undefined) return null;
  if (typeof value !== "string") throw new Error(`dump-fetch 的结果里 ${field} 不是字符串或 null`);
  return value;
}

interface FetchResult {
  readonly files: number;
  readonly bytes: number;
  readonly buildid: string | null;
}

function parseFetchResult(stdout: string): FetchResult {
  const value = parseJsonDocument(stdout, "dump-fetch 的结果");
  if (!isRecord(value)) throw new Error("dump-fetch 的结果不是对象");
  const doneText = value.doneText;
  if (typeof doneText !== "string") {
    throw new Error("dump-fetch 的结果里 doneText 不是字符串");
  }
  parseDone(parseJsonDocument(doneText, "done.json"));
  const files = nonNegativeNumber(value.files, "files");
  const bytes = nonNegativeNumber(value.bytes, "bytes");
  const acf = nullableText(value.acf, "acf");
  const buildid = acf === null ? null : buildIdFromAcf(acf);
  return { files, bytes, buildid };
}

/**
 * Move the job's files from <local>/ue4ss/ffr-out/jobs/<id>/ to <local>/ so callers see the
 * dump at the top level. Refuse to overwrite anything already there; UE4SS's own dumps under
 * ue4ss/ stay put, and only empty ffr-out/jobs directories are removed.
 */
export async function flattenJobDirectory(localDir: string, id: string): Promise<void> {
  const ue4ss = join(localDir, "ue4ss");
  const out = join(ue4ss, "ffr-out");
  const jobs = join(out, "jobs");
  const job = join(jobs, id);
  let entries: string[];
  try {
    entries = await readdir(job);
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") {
      throw new Error(`压缩包里找不到任务目录:${job}`, { cause: error });
    }
    throw error;
  }
  const existing = new Map((await readdir(localDir)).map((name) => [name.toLowerCase(), name]));
  const conflicts = entries.filter((name) => existing.has(name.toLowerCase()));
  if (conflicts.length > 0) {
    throw new Error(`本地目录根上已有同名条目,不覆盖:${conflicts.join("、")}`);
  }
  for (const name of entries) {
    await rename(join(job, name), join(localDir, name));
  }
  await removeIfEmpty(job);
  await removeIfEmpty(jobs);
  await removeIfEmpty(out);
  await removeIfEmpty(ue4ss);
}

async function removeIfEmpty(path: string): Promise<void> {
  try {
    if ((await readdir(path)).length === 0) await rmdir(path);
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") return;
    throw error;
  }
}

/** Fill buildid into the local meta.json; only the dev box reads the ACF text. */
async function stampMetaBuildId(localDir: string, buildid: string | null): Promise<void> {
  const path = join(localDir, "meta.json");
  let parsed: unknown;
  try {
    parsed = JSON.parse(await readFile(path, "utf8"));
  } catch (error) {
    throw new Error(
      `读不了本地 meta.json:${error instanceof Error ? error.message : String(error)}`,
      {
        cause: error,
      },
    );
  }
  if (!isRecord(parsed)) throw new Error("本地 meta.json 不是对象");
  await writeFile(path, JSON.stringify({ ...parsed, buildid }), "utf8");
}

/** Read, pack, pull, unpack and clean up one dump. dump-run shares this exact flow. */
export async function fetchDump(
  machine: Machine,
  id: string,
  signal?: AbortSignal,
): Promise<FetchSummary> {
  const localDir = localJobDir(id);
  if (await pathExists(localDir)) {
    throw new Error(`本地目录已存在,不覆盖:${localDir}`);
  }
  const result = await runPs(machine, fetchScript(machine, id), { capture: true, signal });
  const parsed = parseFetchResult(result.stdout);
  await mkdir(localDir, { recursive: true });
  try {
    const packagePath = join(localDir, "package.tgz");
    await runScp(machine, `${target(machine)}:${remotePackage(machine, id)}`, packagePath, {
      signal,
    });
    await unpackPackage(packagePath, localDir, signal);
    await flattenJobDirectory(localDir, id);
    await stampMetaBuildId(localDir, parsed.buildid);
    await runPs(machine, deleteRemoteScript(machine, id), { signal });
  } catch (error) {
    await rm(localDir, { recursive: true, force: true });
    throw error;
  }
  console.log(`本地目录:${localDir}`);
  console.log(`文件数:${parsed.files}`);
  console.log(`大小:${parsed.bytes} 字节`);
  console.log(`buildid:${parsed.buildid ?? "未知"}`);
  if (parsed.buildid === null) {
    console.warn("警告:读不到游戏版本 buildid,meta.json 里是 null");
  }
  return { localDir, files: parsed.files, bytes: parsed.bytes, buildid: parsed.buildid };
}

async function run(argv: readonly string[]): Promise<number> {
  let parsed;
  try {
    parsed = parseArgs({
      args: [...argv],
      allowPositionals: true,
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
  let options: Options;
  try {
    if (parsed.positionals.length !== 1) {
      throw new Error("dump-fetch 需要一个任务号");
    }
    options = {
      id: parseJobId(parsed.positionals[0]),
      dryRun: values["dry-run"] === true,
      machineEnv: values["machine-env"] ?? process.env.FFR_MACHINE_ENV,
    };
  } catch (error) {
    console.error(`错误:${error instanceof Error ? error.message : String(error)}\n`);
    console.error(USAGE);
    return 2;
  }
  const machine = await loadConfiguredMachine(options.machineEnv);
  if (options.dryRun) {
    printDryRun([
      `完成标记:${remoteJoin(jobDir(machine, options.id), "done.json")}`,
      `远端压缩包:${remotePackage(machine, options.id)}`,
      `本地目录:${localJobDir(options.id)}`,
      "会在游戏机上收文件、写 meta.json、用 tar.exe 打包,再拉回本地解开",
    ]);
    return 0;
  }
  await fetchDump(machine, options.id);
  return 0;
}

if (import.meta.main) {
  await runCli(async () => run(process.argv.slice(2)));
}
