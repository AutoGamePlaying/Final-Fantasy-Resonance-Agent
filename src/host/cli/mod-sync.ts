/**
 * Mirror one repo mod (mod/<名字>/) to <FFR_GAME_DIR>/ue4ss/Mods/<名字>/, then turn it on or off
 * in ue4ss/Mods/mods.txt. The destination mod directory is replaced; other mods are untouched.
 *
 *   node src/host/cli/mod-sync.ts <名字> [--disable] [--dry-run] [--machine-env <文件>]
 */

import { stat } from "node:fs/promises";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { psQuote, remoteJoin, runPs, runScp, target, type Machine } from "../../io/live/machine.ts";
import { REPO, describeError, isRecord, text } from "../../shared/index.ts";
import { assertGameStopped, loadConfiguredMachine, printDryRun, runCli } from "./common.ts";
import { hotkeyDir } from "./dump.ts";

const USAGE = `用法: node src/host/cli/mod-sync.ts <名字> [--disable] [--dry-run] [--machine-env <文件>]

把 mod/<名字>/ 同步到游戏机的 ue4ss/Mods/<名字>/,默认在 mods.txt 里启用。
目标 mod 目录会被替换;别的 mod 和 mods.txt 里的其他行不动。

参数:
  --disable              在 mods.txt 里停用,而不是启用
  --dry-run              只打印要做的事,不连接游戏机
  --machine-env <文件>   游戏机配置,默认 machine/machine.env
  -h, --help             显示这段说明

例:
  node src/host/cli/mod-sync.ts FfrProbe
  node src/host/cli/mod-sync.ts FfrProbe --disable`;

interface Options {
  readonly name: string;
  readonly enabled: boolean;
  readonly dryRun: boolean;
  readonly machineEnv?: string;
}

function parseName(value: string | undefined): string {
  if (!value) throw new Error("mod-sync 需要 mod 名字,例如 FfrProbe");
  if (!/^[A-Za-z0-9_]+$/u.test(value)) {
    throw new Error(`mod 名字只能包含字母、数字和下划线,收到 ${value}`);
  }
  return value;
}

async function sourceDirectory(name: string): Promise<string> {
  const source = join(REPO, "mod", name);
  let info;
  try {
    info = await stat(source);
  } catch (error) {
    throw new Error(`找不到 mod/${name}/:检查目录名`, { cause: error });
  }
  if (!info.isDirectory()) throw new Error(`mod/${name} 不是目录`);
  return source;
}

/** Update one line of UE4SS's mods.txt, or append it; other lines keep their text. */
export function setModState(content: string, name: string, enabled: boolean): string {
  const value = enabled ? "1" : "0";
  const lines = content.replaceAll("\r\n", "\n").replaceAll("\r", "\n").split("\n");
  while (lines.length > 0 && lines.at(-1) === "") lines.pop();
  let found = false;
  const updated: string[] = [];
  for (const line of lines) {
    const colon = line.indexOf(":");
    if (colon >= 0 && line.slice(0, colon).trim() === name) {
      found = true;
      const rest = line.slice(colon + 1);
      const hash = rest.indexOf("#");
      const comment = hash >= 0 ? ` ${rest.slice(hash).trim()}` : "";
      updated.push(`${line.slice(0, colon).trimEnd()} : ${value}${comment}`);
    } else {
      updated.push(line);
    }
  }
  if (!found) updated.push(`${name} : ${value}`);
  return `${updated.join("\r\n")}\r\n`;
}

export function stateScript(machine: Machine): string {
  const mods = remoteJoin(machine.gameDir, "ue4ss", "Mods");
  return (
    `$mods = ${psQuote(mods)}\n` +
    `if (-not (Test-Path -LiteralPath $mods -PathType Container)) { ` +
    'throw "游戏目录里没有 ue4ss/Mods:$mods,请先运行 ue4ss-install" }\n' +
    "$path = Join-Path $mods 'mods.txt'\n" +
    "$value = ''\n" +
    // 11-24 live: in PowerShell 5.1 `Get-Content -Raw` returns a string carrying PSPath etc. as
    // note properties, and ConvertTo-Json then emits an object instead of a string, so the
    // existing mods.txt read back as empty and was overwritten. ReadAllText gives a plain string.
    "if (Test-Path -LiteralPath $path -PathType Leaf) { " +
    "$value = [IO.File]::ReadAllText($path, [Text.Encoding]::UTF8) }\n" +
    "ConvertTo-Json @{ mods = $value } -Compress\n"
  );
}

export function deleteScript(machine: Machine, name: string): string {
  const destination = remoteJoin(machine.gameDir, "ue4ss", "Mods", name);
  return (
    `$dest = ${psQuote(destination)}\n` +
    "if (Test-Path -LiteralPath $dest) { Remove-Item -LiteralPath $dest -Recurse -Force }\n"
  );
}

export function writeModsScript(
  machine: Machine,
  content: string,
  ensureDirs: readonly string[] = [],
): string {
  const path = remoteJoin(machine.gameDir, "ue4ss", "Mods", "mods.txt");
  const createDirs = ensureDirs
    .map((dir) => `New-Item -ItemType Directory -Force -Path ${psQuote(dir)} | Out-Null\n`)
    .join("");
  return (
    `[IO.File]::WriteAllText(${psQuote(path)}, ${psQuote(content)}, ` +
    "(New-Object Text.UTF8Encoding $false))\n" +
    createDirs
  );
}

function parseState(stdout: string): string {
  let value: unknown;
  try {
    value = JSON.parse(stdout.trim());
  } catch (error) {
    throw new Error(`读 mods.txt 时游戏机返回的不是合法 JSON:${describeError(error)}`, {
      cause: error,
    });
  }
  if (!isRecord(value)) throw new Error("读 mods.txt 时游戏机返回的不是对象");
  return text(value.mods) ?? "";
}

async function run(argv: readonly string[]): Promise<number> {
  let parsed;
  try {
    parsed = parseArgs({
      args: [...argv],
      allowPositionals: true,
      options: {
        help: { type: "boolean", short: "h" },
        disable: { type: "boolean" },
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
      name: parseName(parsed.positionals[0]),
      enabled: values.disable !== true,
      dryRun: values["dry-run"] === true,
      machineEnv: values["machine-env"] ?? process.env.FFR_MACHINE_ENV,
    };
  } catch (error) {
    console.error(`错误:${error instanceof Error ? error.message : String(error)}\n`);
    console.error(USAGE);
    return 2;
  }
  const machine = await loadConfiguredMachine(options.machineEnv);
  const source = await sourceDirectory(options.name);
  const destination = remoteJoin(machine.gameDir, "ue4ss", "Mods", options.name);
  const ensureDirs = options.name === "FfrDump" ? [hotkeyDir(machine)] : [];
  if (options.dryRun) {
    printDryRun([
      `源:${source}`,
      `目标:${destination}`,
      "会先删除目标 mod 目录,再复制整棵目录",
      `mods.txt:${options.enabled ? "启用" : "停用"} ${options.name}`,
      ...ensureDirs.map((dir) => `确保目录存在:${dir}`),
    ]);
    return 0;
  }
  await assertGameStopped(machine);
  const state = parseState((await runPs(machine, stateScript(machine), { capture: true })).stdout);
  await runPs(machine, deleteScript(machine, options.name));
  await runScp(
    machine,
    source,
    `${target(machine)}:${remoteJoin(machine.gameDir, "ue4ss", "Mods")}`,
  );
  await runPs(
    machine,
    writeModsScript(machine, setModState(state, options.name, options.enabled), ensureDirs),
  );
  console.log(
    `已同步 ${options.name} 到 ${destination};mods.txt 里已${options.enabled ? "启用" : "停用"}`,
  );
  return 0;
}

if (import.meta.main) {
  await runCli(async () => run(process.argv.slice(2)));
}
