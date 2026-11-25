/**
 * Install the prepared UE4SS package from .deps/ue4ss/ (dwmapi.dll and ue4ss/) into the game's
 * Win64 directory, then record every new file and directory in
 * <LAPTOP_ROOT>/ue4ss-manifest.json. Existing names are refused, never overwritten.
 *
 *   node src/host/cli/ue4ss-install.ts [--dry-run] [--machine-env <文件>]
 */

import { readdir, stat } from "node:fs/promises";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { psQuote, remoteJoin, runPs, runScp, target, type Machine } from "../../io/live/machine.ts";
import { describeError, isRecord, list, ue4ssDir } from "../../shared/index.ts";
import { assertGameStopped, loadConfiguredMachine, printDryRun, runCli } from "./common.ts";

const MANIFEST = "ue4ss-manifest.json";

const USAGE = `用法: node src/host/cli/ue4ss-install.ts [--dry-run] [--machine-env <文件>]

把 .deps/ue4ss/ 里的 dwmapi.dll 和 ue4ss/ 装进游戏的 Win64 目录。
目标里已有同名文件或目录时拒绝安装,不覆盖。装完在游戏机上写一份新增文件清单。

参数:
  --dry-run              只打印要做的事,不连接游戏机
  --machine-env <文件>   游戏机配置,默认 machine/machine.env
  -h, --help             显示这段说明`;

interface Tree {
  files: string[];
  dirs: string[];
}

interface Manifest {
  gameDir: string;
  files: string[];
  dirs: string[];
}

interface Options {
  readonly dryRun: boolean;
  readonly machineEnv?: string;
}

async function scanTree(root: string, relative: string, tree: Tree): Promise<void> {
  const entries = await readdir(root, { withFileTypes: true });
  entries.sort((left, right) => left.name.localeCompare(right.name, "en"));
  for (const entry of entries) {
    const path = relative ? `${relative}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      tree.dirs.push(path);
      await scanTree(join(root, entry.name), path, tree);
    } else if (entry.isFile()) {
      tree.files.push(path);
    } else {
      throw new Error(`.deps/ue4ss 里有不支持的条目(符号链接或设备):${path}`);
    }
  }
}

async function sourceTree(): Promise<Tree> {
  const source = ue4ssDir();
  const dll = join(source, "dwmapi.dll");
  const directory = join(source, "ue4ss");
  let dllStat;
  let dirStat;
  try {
    dllStat = await stat(dll);
    dirStat = await stat(directory);
  } catch (error) {
    throw new Error(
      `找不到 .deps/ue4ss/dwmapi.dll 或 .deps/ue4ss/ue4ss/:请先把 UE4SS 放到 ${source}`,
      { cause: error },
    );
  }
  if (!dllStat.isFile()) throw new Error(`${dll} 不是文件`);
  if (!dirStat.isDirectory()) throw new Error(`${directory} 不是目录`);
  const tree: Tree = { files: ["dwmapi.dll"], dirs: [] };
  await scanTree(directory, "ue4ss", tree);
  tree.files.sort();
  tree.dirs.sort();
  return tree;
}

function existingScript(machine: Machine): string {
  const names = ["dwmapi.dll", "ue4ss"].map(psQuote).join(", ");
  return (
    `$game = ${psQuote(machine.gameDir)}\n` +
    `$existing = @()\n` +
    `foreach ($name in @(${names})) { ` +
    "if (Test-Path -LiteralPath (Join-Path $game $name)) { $existing += $name } }\n" +
    // Quote by hand: ConvertTo-Json unwraps a one-element array.
    "'{\"existing\":[' + (($existing | ForEach-Object { '\"' + $_ + '\"' }) -join ',') + ']}'\n"
  );
}

function manifestScript(machine: Machine, manifest: Manifest): string {
  const path = remoteJoin(machine.root, MANIFEST);
  const body = JSON.stringify(manifest);
  return (
    `New-Item -ItemType Directory -Force -Path ${psQuote(machine.root)} | Out-Null\n` +
    `[IO.File]::WriteAllText(${psQuote(path)}, ${psQuote(body)}, ` +
    "(New-Object Text.UTF8Encoding $false))\n"
  );
}

function parseExisting(stdout: string): string[] {
  let value: unknown;
  try {
    value = JSON.parse(stdout.trim());
  } catch (error) {
    throw new Error(`检查目标目录时游戏机返回的不是合法 JSON:${describeError(error)}`, {
      cause: error,
    });
  }
  if (!isRecord(value)) throw new Error("检查目标目录时游戏机返回的不是对象");
  return list(value.existing).filter((item): item is string => typeof item === "string");
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
  const tree = await sourceTree();
  const manifest: Manifest = {
    gameDir: machine.gameDir,
    files: tree.files,
    dirs: tree.dirs,
  };
  if (options.dryRun) {
    printDryRun([
      `目标:${machine.gameDir}`,
      `会检查目标里没有 dwmapi.dll 和 ue4ss/(已有就拒绝)`,
      `会复制 ${tree.files.length} 个文件、${tree.dirs.length} 个目录`,
      ...tree.files.map((file) => `  ${file}`),
      `清单写到:${remoteJoin(machine.root, MANIFEST)}`,
    ]);
    return 0;
  }
  await assertGameStopped(machine);
  const existing = parseExisting(
    (await runPs(machine, existingScript(machine), { capture: true })).stdout,
  );
  if (existing.length > 0) {
    throw new Error(`目标目录已有同名文件或目录,已拒绝覆盖:${existing.join("、")}`);
  }
  // Write the manifest before copying: a half-finished scp can then be removed by uninstall.
  await runPs(machine, manifestScript(machine, manifest));
  await runScp(
    machine,
    [join(ue4ssDir(), "dwmapi.dll"), join(ue4ssDir(), "ue4ss")],
    `${target(machine)}:${machine.gameDir}`,
  );
  console.log(
    `已装 ${tree.files.length} 个文件到 ${machine.gameDir};` +
      `清单在 ${remoteJoin(machine.root, MANIFEST)}`,
  );
  return 0;
}

await runCli(async () => run(process.argv.slice(2)));
