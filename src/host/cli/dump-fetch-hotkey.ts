/**
 * Incrementally pull in-game hotkey snapshots (F9) and timestamp markers (F10) from
 * <gameDir>/ue4ss/ffr-out/hotkey/ into .deps/r1/hotkey/.
 *
 * Each finished hotkey snapshot (identified by <id>-done.json) is unpacked into
 * .deps/r1/hotkey/<id>/ with the prefix stripped so its layout matches a normal R1 snapshot
 * (context.json, classes.json, objects.jsonl, widgets.jsonl, texts.jsonl, done.json, meta.json).
 * markers.jsonl is pulled incrementally by byte offset up to the last complete line.
 * Already-fetched items are recorded in .deps/r1/hotkey/fetched.json; remote files in
 * OUT/hotkey/ are never deleted.
 *
 *   node src/host/cli/dump-fetch-hotkey.ts [--dry-run] [--machine-env <文件>]
 */

import { randomUUID } from "node:crypto";
import {
  appendFile,
  mkdir,
  mkdtemp,
  readFile,
  rename,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { psQuote, remoteJoin, runPs, runScp, target, type Machine } from "../../io/live/machine.ts";
import { isMissingFile, isRecord } from "../../shared/index.ts";
import { loadConfiguredMachine, printDryRun, runCli } from "./common.ts";
import { unpackPackage } from "./dump-fetch.ts";
import {
  buildIdFromAcf,
  hotkeyDir,
  hotkeyMarkersPath,
  isHotkeySnapshotId,
  localHotkeyDir,
  localHotkeyMarkersPath,
  localHotkeySnapshotDir,
  localHotkeyStatePath,
  parseDone,
  parseHotkeySnapshotFile,
  parseJsonDocument,
  remoteHotkeyPackage,
  type Done,
} from "./dump.ts";

const USAGE = `用法: node src/host/cli/dump-fetch-hotkey.ts [--dry-run] [--machine-env <文件>]

把游戏机上 OUT/hotkey/ 里还没拉过的 F9 热键快照和 F10 时间标记增量拉回 .deps/r1/hotkey/。
每份完成的热键快照按序号整理成 .deps/r1/hotkey/<id>/ 下的标准快照目录结构(去掉文件名前缀
并补写 meta.json),时间标记追加到 .deps/r1/hotkey/markers.jsonl,已拉取记录保存在
.deps/r1/hotkey/fetched.json。游戏在不在运行都可以执行,不删除游戏机上的原件。

参数:
  --dry-run              只打印要读写的路径和本地已拉取数量,不连接游戏机
  --machine-env <文件>   游戏机配置,默认 machine/machine.env
  -h, --help             显示这段说明`;

interface Options {
  readonly dryRun: boolean;
  readonly machineEnv?: string;
}

export interface HotkeyFetchState {
  readonly version: 1;
  readonly snapshots: readonly string[];
  readonly markersBytes: number;
  readonly markersLines: number;
}

export interface HotkeyFetchSummary {
  readonly localDir: string;
  readonly newSnapshots: readonly string[];
  readonly totalSnapshots: number;
  readonly newMarkers: number;
  readonly totalMarkers: number;
  readonly buildid: string | null;
}

interface RemoteSnapshotEntry {
  readonly id: string;
  readonly done: Done;
  readonly files: readonly string[];
  readonly bytes: number;
}

interface RemoteHotkeyFetchResult {
  readonly exists: boolean;
  readonly snapshots: readonly RemoteSnapshotEntry[];
  readonly packageCreated: boolean;
  readonly files: number;
  readonly bytes: number;
  readonly buildid: string | null;
  readonly markersReset: boolean;
  readonly markersBytes: number;
  readonly markersDelta: Buffer;
}

const EMPTY_STATE: HotkeyFetchState = {
  version: 1,
  snapshots: [],
  markersBytes: 0,
  markersLines: 0,
};

function nonNegativeInt(value: unknown, document: string, field: string): number {
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0) {
    throw new Error(`${document} 的 ${field} 字段不是非负整数`);
  }
  return value;
}

function booleanField(value: unknown, document: string, field: string): boolean {
  if (typeof value !== "boolean") {
    throw new Error(`${document} 的 ${field} 字段不是布尔值`);
  }
  return value;
}

function nullableText(value: unknown, document: string, field: string): string | null {
  if (value === null || value === undefined) return null;
  if (typeof value !== "string") {
    throw new Error(`${document} 的 ${field} 字段不是字符串或 null`);
  }
  return value;
}

export function parseHotkeyFetchState(input: unknown): HotkeyFetchState {
  if (!isRecord(input)) throw new Error("fetched.json 不是对象");
  if (input.version !== 1) {
    throw new Error(`fetched.json 的 version 不支持:${String(input.version)}`);
  }
  if (!Array.isArray(input.snapshots)) {
    throw new Error("fetched.json 的 snapshots 字段不是数组");
  }
  const snapshots = input.snapshots.map((item, index) => {
    if (typeof item !== "string" || !isHotkeySnapshotId(item)) {
      throw new Error(`fetched.json 的 snapshots[${index}] 不是合法的热键快照号`);
    }
    return item;
  });
  return {
    version: 1,
    snapshots,
    markersBytes: nonNegativeInt(input.markersBytes, "fetched.json", "markersBytes"),
    markersLines: nonNegativeInt(input.markersLines, "fetched.json", "markersLines"),
  };
}

export async function loadHotkeyFetchState(
  statePath: string = localHotkeyStatePath(),
): Promise<HotkeyFetchState> {
  let body: string;
  try {
    body = await readFile(statePath, "utf8");
  } catch (error) {
    if (isMissingFile(error)) return EMPTY_STATE;
    throw new Error(
      `读不了热键拉取记录 ${statePath}:${error instanceof Error ? error.message : String(error)}`,
      { cause: error },
    );
  }
  return parseHotkeyFetchState(parseJsonDocument(body, "fetched.json"));
}

export async function saveHotkeyFetchState(
  state: HotkeyFetchState,
  statePath: string = localHotkeyStatePath(),
): Promise<void> {
  const temporary = `${statePath}.tmp`;
  await writeFile(temporary, `${JSON.stringify(state, null, 2)}\n`, "utf8");
  await rename(temporary, statePath);
}

/** Delete only the temporary hotkey .tgz created by this fetch run. */
export function deleteRemoteHotkeyScript(packagePath: string): string {
  return (
    `$package = ${psQuote(packagePath)}\n` +
    "if (Test-Path -LiteralPath $package -PathType Leaf) { " +
    "Remove-Item -LiteralPath $package -Force }\n"
  );
}

/**
 * Inspect OUT/hotkey/ on the game machine, pack only completed snapshots not in alreadyFetched,
 * and return any newly appended complete lines of markers.jsonl as base64.
 */
export function fetchHotkeyScript(
  machine: Machine,
  alreadyFetched: readonly string[],
  markersOffset: number,
  packagePath: string,
  listPath: string,
): string {
  const game = machine.gameDir;
  const hotkey = hotkeyDir(machine);
  const appId = machine.appId?.trim() ?? "";
  const safeOffset = Math.max(0, Math.floor(markersOffset));
  const addFetched = alreadyFetched.map((id) => `[void]$already.Add(${psQuote(id)})\n`).join("");
  return (
    `$game = ${psQuote(game)}\n` +
    `$hotkey = ${psQuote(hotkey)}\n` +
    `$package = ${psQuote(packagePath)}\n` +
    `$listPath = ${psQuote(listPath)}\n` +
    `$appId = ${psQuote(appId)}\n` +
    `$markersOffset = [long]${safeOffset}\n` +
    "$already = [System.Collections.Generic.HashSet[string]]::new()\n" +
    addFetched +
    "if (-not (Test-Path -LiteralPath $hotkey -PathType Container)) {\n" +
    "  ConvertTo-Json @{ exists = $false; snapshots = @(); packageCreated = $false; files = 0; " +
    "bytes = 0; acf = $null; markersReset = $false; markersBytes = $markersOffset; " +
    "markersDeltaBase64 = '' } -Compress -Depth 6\n" +
    "  exit 0\n" +
    "}\n" +
    "$donePattern = '^hotkey-\\d{4,}-\\d{8}-\\d{6}-done\\.json$'\n" +
    "$allowedPattern = '^hotkey-\\d{4,}-\\d{8}-\\d{6}-(context\\.json|classes\\.json|" +
    "objects\\.jsonl|widgets\\.jsonl|texts\\.jsonl|done\\.json)$'\n" +
    "$allFiles = @(Get-ChildItem -LiteralPath $hotkey -File -Force | Sort-Object Name)\n" +
    "$byId = @{}\n" +
    "$doneIds = [System.Collections.Generic.List[string]]::new()\n" +
    "foreach ($item in $allFiles) {\n" +
    "  if ($item.Name -match $donePattern) {\n" +
    "    $snapId = $item.Name.Substring(0, $item.Name.Length - 10)\n" +
    "    if (-not $already.Contains($snapId)) { [void]$doneIds.Add($snapId) }\n" +
    "  }\n" +
    "  if ($item.Name -match $allowedPattern) {\n" +
    "    $idx = $item.Name.LastIndexOf('-')\n" +
    "    $snapId = $item.Name.Substring(0, $idx)\n" +
    "    if (-not $byId.ContainsKey($snapId)) {\n" +
    "      $byId[$snapId] = [System.Collections.Generic.List[object]]::new()\n" +
    "    }\n" +
    "    [void]$byId[$snapId].Add($item)\n" +
    "  }\n" +
    "}\n" +
    "$snapshots = [System.Collections.Generic.List[object]]::new()\n" +
    "$tarPaths = [System.Collections.Generic.List[string]]::new()\n" +
    "$totalFiles = 0\n" +
    "$totalBytes = [long]0\n" +
    "foreach ($snapId in $doneIds) {\n" +
    "  $donePath = Join-Path $hotkey ($snapId + '-done.json')\n" +
    "  $doneText = [IO.File]::ReadAllText($donePath, [Text.Encoding]::UTF8)\n" +
    "  $snapFiles = [System.Collections.Generic.List[string]]::new()\n" +
    "  $snapBytes = [long]0\n" +
    "  if ($byId.ContainsKey($snapId)) {\n" +
    "    foreach ($entry in $byId[$snapId]) {\n" +
    "      [void]$snapFiles.Add($entry.Name)\n" +
    "      [void]$tarPaths.Add($entry.Name)\n" +
    "      $snapBytes += [long]$entry.Length\n" +
    "      $totalFiles++\n" +
    "      $totalBytes += [long]$entry.Length\n" +
    "    }\n" +
    "  }\n" +
    "  [void]$snapshots.Add([pscustomobject]@{ id = $snapId; doneText = $doneText; " +
    "files = @($snapFiles); bytes = $snapBytes })\n" +
    "}\n" +
    "$markersReset = $false\n" +
    "$markersBytes = $markersOffset\n" +
    "$markersDeltaBase64 = ''\n" +
    "$markersPath = Join-Path $hotkey 'markers.jsonl'\n" +
    "if (Test-Path -LiteralPath $markersPath -PathType Leaf) {\n" +
    "  $fs = [IO.File]::Open($markersPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, " +
    "[IO.FileShare]::ReadWrite)\n" +
    "  try {\n" +
    "    $len = [long]$fs.Length\n" +
    "    $readFrom = $markersOffset\n" +
    "    if ($len -lt $readFrom) {\n" +
    "      $markersReset = $true\n" +
    "      $readFrom = [long]0\n" +
    "    }\n" +
    "    if ($len -gt $readFrom) {\n" +
    "      [void]$fs.Seek($readFrom, [IO.SeekOrigin]::Begin)\n" +
    "      $toRead = [int]($len - $readFrom)\n" +
    "      $buf = New-Object byte[] $toRead\n" +
    "      $offset = 0\n" +
    "      while ($offset -lt $toRead) {\n" +
    "        $n = $fs.Read($buf, $offset, $toRead - $offset)\n" +
    "        if ($n -le 0) { break }\n" +
    "        $offset += $n\n" +
    "      }\n" +
    "      $lastLf = -1\n" +
    "      for ($i = $offset - 1; $i -ge 0; $i--) {\n" +
    "        if ($buf[$i] -eq 10) { $lastLf = $i; break }\n" +
    "      }\n" +
    "      if ($lastLf -ge 0) {\n" +
    "        $markersDeltaBase64 = [Convert]::ToBase64String($buf, 0, $lastLf + 1)\n" +
    "        $markersBytes = $readFrom + $lastLf + 1\n" +
    "      } else {\n" +
    "        $markersBytes = $readFrom\n" +
    "      }\n" +
    "    } else {\n" +
    "      $markersBytes = $len\n" +
    "    }\n" +
    "  } finally { $fs.Dispose() }\n" +
    "}\n" +
    "$acfText = $null\n" +
    "if ($appId -ne '' -and $snapshots.Count -gt 0) {\n" +
    '  $acf = [IO.Path]::GetFullPath((Join-Path $game "../../../../../appmanifest_$appId.acf"))\n' +
    "  if (Test-Path -LiteralPath $acf -PathType Leaf) {\n" +
    "    $acfText = [IO.File]::ReadAllText($acf, [Text.Encoding]::UTF8)\n" +
    "  }\n" +
    "}\n" +
    "$packageCreated = $false\n" +
    "if ($tarPaths.Count -gt 0) {\n" +
    "  $tar = Join-Path $env:SystemRoot 'System32/tar.exe'\n" +
    '  if (-not (Test-Path -LiteralPath $tar -PathType Leaf)) { throw "找不到系统自带的 tar.exe:$tar" }\n' +
    "  $packageDir = Split-Path -Parent $package\n" +
    "  if (-not (Test-Path -LiteralPath $packageDir -PathType Container)) {\n" +
    "    New-Item -ItemType Directory -Force -Path $packageDir | Out-Null\n" +
    "  }\n" +
    "  $newline = [string][char]10\n" +
    "  $listText = (($tarPaths | ForEach-Object { $_ }) -join $newline) + $newline\n" +
    "  [IO.File]::WriteAllText($listPath, $listText, (New-Object Text.UTF8Encoding $false))\n" +
    "  try {\n" +
    "    & $tar -czf $package -C $hotkey -T $listPath\n" +
    '    if ($LASTEXITCODE -ne 0) { throw "打包失败:tar.exe 退出码 $LASTEXITCODE" }\n' +
    "    $packageCreated = $true\n" +
    "  } finally {\n" +
    "    if (Test-Path -LiteralPath $listPath -PathType Leaf) {\n" +
    "      Remove-Item -LiteralPath $listPath -Force\n" +
    "    }\n" +
    "  }\n" +
    "}\n" +
    "ConvertTo-Json @{ exists = $true; snapshots = @($snapshots); packageCreated = $packageCreated; " +
    "files = $totalFiles; bytes = $totalBytes; acf = $acfText; markersReset = $markersReset; " +
    "markersBytes = $markersBytes; markersDeltaBase64 = $markersDeltaBase64 } -Compress -Depth 6\n"
  );
}

function parseRemoteResult(stdout: string): RemoteHotkeyFetchResult {
  const doc = "dump-fetch-hotkey 的结果";
  const value = parseJsonDocument(stdout, doc);
  if (!isRecord(value)) throw new Error(`${doc}不是对象`);
  const exists = booleanField(value.exists, doc, "exists");
  const packageCreated = booleanField(value.packageCreated, doc, "packageCreated");
  const files = nonNegativeInt(value.files, doc, "files");
  const bytes = nonNegativeInt(value.bytes, doc, "bytes");
  const acf = nullableText(value.acf, doc, "acf");
  const buildid = acf === null ? null : buildIdFromAcf(acf);
  const markersReset = booleanField(value.markersReset, doc, "markersReset");
  const markersBytes = nonNegativeInt(value.markersBytes, doc, "markersBytes");
  if (typeof value.markersDeltaBase64 !== "string") {
    throw new Error(`${doc} 的 markersDeltaBase64 字段不是字符串`);
  }
  const markersDelta =
    value.markersDeltaBase64 === ""
      ? Buffer.alloc(0)
      : Buffer.from(value.markersDeltaBase64, "base64");
  if (!Array.isArray(value.snapshots)) {
    throw new Error(`${doc} 的 snapshots 字段不是数组`);
  }
  const snapshots = value.snapshots.map((item, index): RemoteSnapshotEntry => {
    if (!isRecord(item)) throw new Error(`${doc} 的 snapshots[${index}] 不是对象`);
    const id = item.id;
    if (typeof id !== "string" || !isHotkeySnapshotId(id)) {
      throw new Error(`${doc} 的 snapshots[${index}].id 不合法:${String(id)}`);
    }
    const doneText = item.doneText;
    if (typeof doneText !== "string") {
      throw new Error(`${doc} 的 snapshots[${index}].doneText 不是字符串`);
    }
    const done = parseDone(parseJsonDocument(doneText, `${id}-done.json`));
    if (!Array.isArray(item.files)) {
      throw new Error(`${doc} 的 snapshots[${index}].files 不是数组`);
    }
    const snapFiles = item.files.map((entry, fileIndex) => {
      if (typeof entry !== "string") {
        throw new Error(`${doc} 的 snapshots[${index}].files[${fileIndex}] 不是字符串`);
      }
      const parsedFile = parseHotkeySnapshotFile(entry);
      if (parsedFile?.id !== id) {
        throw new Error(`${doc} 的 snapshots[${index}].files[${fileIndex}] 不属于 ${id}:${entry}`);
      }
      return entry;
    });
    const snapBytes = nonNegativeInt(item.bytes, doc, `snapshots[${index}].bytes`);
    return { id, done, files: snapFiles, bytes: snapBytes };
  });
  return {
    exists,
    snapshots,
    packageCreated,
    files,
    bytes,
    buildid,
    markersReset,
    markersBytes,
    markersDelta,
  };
}

async function pathExists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch (error) {
    if (isMissingFile(error)) return false;
    throw error;
  }
}

function countNonEmptyLines(text: string): number {
  let count = 0;
  for (const line of text.split(/\r?\n/u)) {
    if (line.trim() !== "") count++;
  }
  return count;
}

/** Pull any newly completed hotkey snapshots and newly appended markers incrementally. */
export async function fetchHotkeys(
  machine: Machine,
  signal?: AbortSignal,
): Promise<HotkeyFetchSummary> {
  const localDir = localHotkeyDir();
  const statePath = localHotkeyStatePath();
  const markersLocal = localHotkeyMarkersPath();
  const state = await loadHotkeyFetchState(statePath);
  const effectiveMarkersOffset =
    state.markersBytes > 0 && !(await pathExists(markersLocal)) ? 0 : state.markersBytes;

  const tag = randomUUID().replaceAll("-", "").slice(0, 12);
  const remotePkg = remoteHotkeyPackage(machine, tag);
  const remoteList = remoteJoin(machine.root, "ffr-dumps", `hotkey-${tag}.files.txt`);

  const script = fetchHotkeyScript(
    machine,
    state.snapshots,
    effectiveMarkersOffset,
    remotePkg,
    remoteList,
  );
  const raw = await runPs(machine, script, { capture: true, signal });
  const parsed = parseRemoteResult(raw.stdout);

  if (!parsed.exists) {
    console.log(
      `游戏机上还没有热键目录:${hotkeyDir(machine)}(先运行 mod-sync FfrDump 或 dump-run)`,
    );
    return {
      localDir,
      newSnapshots: [],
      totalSnapshots: state.snapshots.length,
      newMarkers: 0,
      totalMarkers: state.markersLines,
      buildid: null,
    };
  }

  const hasNewMarkers = parsed.markersDelta.byteLength > 0 || parsed.markersReset;
  if (parsed.snapshots.length === 0 && !hasNewMarkers) {
    console.log(
      `没有新的热键快照或时间标记(已拉取 快照:${state.snapshots.length} 个,标记:${state.markersLines} 条)`,
    );
    return {
      localDir,
      newSnapshots: [],
      totalSnapshots: state.snapshots.length,
      newMarkers: 0,
      totalMarkers: state.markersLines,
      buildid: null,
    };
  }

  await mkdir(localDir, { recursive: true });
  const movedDirs: string[] = [];

  try {
    if (parsed.snapshots.length > 0) {
      if (!parsed.packageCreated) {
        throw new Error("游戏机报告有新热键快照,但没有生成压缩包");
      }
      // Verify no destination directory already exists before moving anything into place.
      for (const snap of parsed.snapshots) {
        const dest = localHotkeySnapshotDir(snap.id);
        if (await pathExists(dest)) {
          throw new Error(`本地热键快照目录已存在,不覆盖:${dest}`);
        }
      }
      const staging = await mkdtemp(join(localDir, ".staging-"));
      try {
        const packagePath = join(staging, "package.tgz");
        await runScp(machine, `${target(machine)}:${remotePkg}`, packagePath, { signal });
        await unpackPackage(packagePath, staging, signal);

        const prepared: { readonly from: string; readonly to: string }[] = [];
        for (const snap of parsed.snapshots) {
          const snapTmp = join(staging, snap.id);
          await mkdir(snapTmp, { recursive: true });
          for (const remoteName of snap.files) {
            const parsedName = parseHotkeySnapshotFile(remoteName);
            if (!parsedName) {
              throw new Error(`无法识别的热键快照文件名:${remoteName}`);
            }
            await rename(join(staging, remoteName), join(snapTmp, parsedName.file));
          }
          const meta = {
            id: snap.done.id,
            job: snap.done.job,
            label: snap.done.label,
            startedAt: snap.done.startedAt,
            finishedAt: snap.done.finishedAt,
            files: snap.files.length,
            bytes: snap.bytes,
            buildid: parsed.buildid,
          };
          await writeFile(join(snapTmp, "meta.json"), JSON.stringify(meta), "utf8");
          prepared.push({ from: snapTmp, to: localHotkeySnapshotDir(snap.id) });
        }

        for (const item of prepared) {
          await rename(item.from, item.to);
          movedDirs.push(item.to);
        }
      } finally {
        await rm(staging, { recursive: true, force: true });
      }
      await runPs(machine, deleteRemoteHotkeyScript(remotePkg), { signal });
    }

    const newMarkers = countNonEmptyLines(parsed.markersDelta.toString("utf8"));
    let totalMarkers = state.markersLines;
    if (parsed.markersReset || effectiveMarkersOffset === 0) {
      if (hasNewMarkers) {
        await writeFile(markersLocal, parsed.markersDelta);
        totalMarkers = newMarkers;
      }
    } else if (parsed.markersDelta.byteLength > 0) {
      await appendFile(markersLocal, parsed.markersDelta);
      totalMarkers = state.markersLines + newMarkers;
    }

    const fetchedSet = new Set(state.snapshots);
    const newIds: string[] = [];
    for (const snap of parsed.snapshots) {
      if (!fetchedSet.has(snap.id)) {
        fetchedSet.add(snap.id);
        newIds.push(snap.id);
      }
    }
    const updatedState: HotkeyFetchState = {
      version: 1,
      snapshots: [...fetchedSet].sort(),
      markersBytes: parsed.markersBytes,
      markersLines: totalMarkers,
    };
    await saveHotkeyFetchState(updatedState, statePath);

    console.log(`本地目录:${localDir}`);
    console.log(`新增热键快照:${newIds.length} 个(累计 ${updatedState.snapshots.length} 个)`);
    for (const snap of parsed.snapshots) {
      console.log(
        `  ${snap.id} ok=${snap.done.ok} 文件=${snap.files.length} 大小=${snap.bytes} 字节`,
      );
    }
    console.log(`新增时间标记:${newMarkers} 条(累计 ${updatedState.markersLines} 条)`);
    if (newIds.length > 0) {
      console.log(`buildid:${parsed.buildid ?? "未知"}`);
      if (parsed.buildid === null) {
        console.warn("警告:读不到游戏版本 buildid,meta.json 里是 null");
      }
    }
    return {
      localDir,
      newSnapshots: newIds,
      totalSnapshots: updatedState.snapshots.length,
      newMarkers,
      totalMarkers: updatedState.markersLines,
      buildid: parsed.buildid,
    };
  } catch (error) {
    for (const dir of movedDirs) {
      await rm(dir, { recursive: true, force: true }).catch(() => undefined);
    }
    throw error;
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
    const state = await loadHotkeyFetchState();
    printDryRun([
      `远端热键目录:${hotkeyDir(machine)}`,
      `远端标记文件:${hotkeyMarkersPath(machine)}`,
      `本地目录:${localHotkeyDir()}`,
      `拉取记录:${localHotkeyStatePath()}(已拉取 快照 ${state.snapshots.length} 个,标记 ${state.markersLines} 条)`,
      "会增量打包新完成的热键快照并拉取新增时间标记,不删除游戏机上的原件",
    ]);
    return 0;
  }
  await fetchHotkeys(machine);
  return 0;
}

if (import.meta.main) {
  await runCli(async () => run(process.argv.slice(2)));
}
