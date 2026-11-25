/**
 * Remove UE4SS using the file list written by ue4ss-install. Only the listed files and empty
 * listed directories are touched; anything else in Win64 is left alone.
 *
 *   node src/host/cli/ue4ss-uninstall.ts [--dry-run] [--machine-env <文件>]
 */

import { parseArgs } from "node:util";
import { psQuote, remoteJoin, runPs, type Machine } from "../../io/live/machine.ts";
import { describeError, isRecord, list, text } from "../../shared/index.ts";
import { assertGameStopped, loadConfiguredMachine, printDryRun, runCli } from "./common.ts";

const MANIFEST = "ue4ss-manifest.json";

const USAGE = `用法: node src/host/cli/ue4ss-uninstall.ts [--dry-run] [--machine-env <文件>]

按游戏机上的 ue4ss-manifest.json 删除安装时新增的文件和空目录,清单外的文件不碰。

参数:
  --dry-run              只打印要删的清单,不连接游戏机
  --machine-env <文件>   游戏机配置,默认 machine/machine.env
  -h, --help             显示这段说明`;

interface Manifest {
  readonly gameDir: string;
  readonly files: string[];
  readonly dirs: string[];
}

interface Options {
  readonly dryRun: boolean;
  readonly machineEnv?: string;
}

function relativePath(value: unknown, label: string): string {
  const raw = text(value);
  if (!raw) throw new Error(`UE4SS 安装清单里有一项 ${label} 不是非空字符串`);
  const normalized = raw.replaceAll("\\", "/");
  if (normalized.startsWith("/") || /^[A-Za-z]:/u.test(normalized)) {
    throw new Error(`UE4SS 安装清单里的 ${label} 是绝对路径,拒绝删除:${raw}`);
  }
  const parts = normalized.split("/");
  if (parts.some((part) => part === "" || part === "." || part === "..")) {
    throw new Error(`UE4SS 安装清单里的 ${label} 不安全,拒绝删除:${raw}`);
  }
  return normalized;
}

function parseManifest(raw: string): Manifest {
  let value: unknown;
  try {
    value = JSON.parse(raw.trim());
  } catch (error) {
    throw new Error(`UE4SS 安装清单不是合法 JSON:${describeError(error)}`, { cause: error });
  }
  if (!isRecord(value)) throw new Error("UE4SS 安装清单不是对象");
  const gameDir = text(value.gameDir);
  if (!gameDir) throw new Error("UE4SS 安装清单里缺少 gameDir");
  return {
    gameDir,
    files: list(value.files).map((item) => relativePath(item, "文件")),
    dirs: list(value.dirs).map((item) => relativePath(item, "目录")),
  };
}

function manifestScript(machine: Machine): string {
  return (
    `$path = ${psQuote(remoteJoin(machine.root, MANIFEST))}\n` +
    'if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "没有找到 UE4SS 安装清单:$path" }\n' +
    "Get-Content -LiteralPath $path -Raw -Encoding UTF8\n"
  );
}

function deleteScript(machine: Machine, manifest: Manifest): string {
  const files = psQuote(JSON.stringify(manifest.files));
  const dirs = psQuote(JSON.stringify(manifest.dirs));
  const path = psQuote(remoteJoin(machine.root, MANIFEST));
  return (
    `$game = ${psQuote(machine.gameDir)}\n` +
    `$files = @(ConvertFrom-Json ${files})\n` +
    "$removed = 0\n" +
    "foreach ($rel in $files) {\n" +
    "  $target = Join-Path $game $rel\n" +
    "  if (Test-Path -LiteralPath $target -PathType Leaf) {\n" +
    "    Remove-Item -LiteralPath $target -Force\n" +
    "    $removed++\n" +
    "  }\n" +
    "}\n" +
    `$dirs = @(ConvertFrom-Json ${dirs})\n` +
    "foreach ($rel in ($dirs | Sort-Object { $_.Length } -Descending)) {\n" +
    "  $target = Join-Path $game $rel\n" +
    "  if ((Test-Path -LiteralPath $target -PathType Container) -and " +
    "((Get-ChildItem -LiteralPath $target -Force | Measure-Object).Count -eq 0)) {\n" +
    "    Remove-Item -LiteralPath $target -Force\n" +
    "  }\n" +
    "}\n" +
    `Remove-Item -LiteralPath ${path} -Force\n` +
    "'removed=' + $removed\n"
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
      `目标:${machine.gameDir}`,
      `读取清单:${remoteJoin(machine.root, MANIFEST)}`,
      "按清单删除文件和空目录,清单外的文件不碰",
      `删除后一并删除清单本身`,
    ]);
    return 0;
  }
  await assertGameStopped(machine);
  const manifest = parseManifest(
    (await runPs(machine, manifestScript(machine), { capture: true })).stdout,
  );
  if (manifest.gameDir.replace(/[\\/]+$/u, "") !== machine.gameDir) {
    throw new Error(
      `安装清单里的游戏目录(${manifest.gameDir})与当前配置(${machine.gameDir})不一致,` +
        "拒绝按旧清单删除",
    );
  }
  const result = await runPs(machine, deleteScript(machine, manifest), { capture: true });
  const removed = result.stdout.trim();
  console.log(`已按清单清理 ${machine.gameDir}(${removed || "removed=?"})`);
  return 0;
}

await runCli(async () => run(process.argv.slice(2)));
