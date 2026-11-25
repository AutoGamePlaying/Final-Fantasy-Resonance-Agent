/**
 * The UE4SS command lines, driven end to end with a fake ssh/scp on PATH and a temporary
 * machine.env. Nothing here reads machine/machine.env or connects to a real game machine.
 */

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

const CLI_DIR = join(import.meta.dirname, "..", "..", "..", "src", "host", "cli");
const FAKE_SSH = join(import.meta.dirname, "..", "..", "fixtures", "fake-ssh.mjs");

interface Response {
  readonly stdout?: string;
  readonly stderr?: string;
  readonly code?: number;
}

interface Harness {
  readonly directory: string;
  readonly env: NodeJS.ProcessEnv;
  readonly log: string;
  readonly machineEnv: string;
  readonly deps: string;
}

interface HarnessOptions {
  readonly appId?: boolean;
}

async function harness(
  responses: readonly Response[],
  options: HarnessOptions = {},
): Promise<Harness> {
  const directory = await mkdtemp(join(tmpdir(), "ffr-cli-"));
  const bin = join(directory, "bin");
  await mkdir(bin);
  const config = join(directory, "fake-ssh.json");
  const log = join(directory, "ssh.log");
  const machineEnv = join(directory, "machine.env");
  const deps = join(directory, "deps");
  await writeFile(config, JSON.stringify({ responses }), "utf8");
  await writeFile(log, "", "utf8");
  const lines = [
    "LAPTOP_HOST=192.0.2.10",
    "LAPTOP_USER=ffr",
    `LAPTOP_KEY=${join(directory, "id_ed25519")}`,
    "LAPTOP_ROOT=C:/ffr",
    "FFR_GAME_DIR=C:/game/Win64",
  ];
  if (options.appId !== false) lines.push("FFR_APP_ID=12345");
  await writeFile(machineEnv, `${lines.join("\n")}\n`, "utf8");
  const wrapper =
    "#!/bin/sh\n" +
    `FAKE_SSH_CONFIG=${shellQuote(config)} FAKE_SSH_LOG=${shellQuote(log)} ` +
    `exec ${shellQuote(process.execPath)} ${shellQuote(FAKE_SSH)} "$@"\n`;
  for (const name of ["ssh", "scp"]) {
    const path = join(bin, name);
    await writeFile(path, wrapper, "utf8");
    await chmod(path, 0o755);
  }
  await chmod(config, 0o600);
  return {
    directory,
    log,
    machineEnv,
    deps,
    env: {
      ...process.env,
      PATH: `${bin}:${process.env.PATH ?? ""}`,
      FFR_DEPS: deps,
      FFR_MACHINE_ENV: machineEnv,
    },
  };
}

async function run(
  fixture: Harness,
  cli: string,
  ...args: string[]
): Promise<{ code: number; stdout: string; stderr: string }> {
  const child = spawn(process.execPath, [join(CLI_DIR, cli), ...args], {
    env: fixture.env,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (chunk: Buffer) => {
    stdout += chunk.toString("utf8");
  });
  child.stderr.on("data", (chunk: Buffer) => {
    stderr += chunk.toString("utf8");
  });
  const code = await new Promise<number>((resolve, reject) => {
    child.on("error", reject);
    child.on("close", (value) => {
      resolve(value ?? -1);
    });
  });
  return { code, stdout, stderr };
}

async function calls(path: string): Promise<{ args: string[]; stdin: string }[]> {
  const body = await readFile(path, "utf8");
  return body
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line) as { args: string[]; stdin: string });
}

function decodeScript(call: { args: string[]; stdin: string } | undefined): string {
  if (!call) return "";
  const index = call.args.indexOf("-EncodedCommand");
  const encoded = index >= 0 ? call.args[index + 1] : undefined;
  return encoded ? Buffer.from(encoded, "base64").toString("utf16le") : "";
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

async function prepareUe4ss(deps: string): Promise<void> {
  const root = join(deps, "ue4ss");
  await mkdir(join(root, "ue4ss", "Mods"), { recursive: true });
  await writeFile(join(root, "dwmapi.dll"), "dll", "utf8");
  await writeFile(join(root, "ue4ss", "Mods", "mods.txt"), "", "utf8");
}

void test("dry-run commands print a plan without opening ssh", async () => {
  const fixture = await harness([]);
  try {
    await prepareUe4ss(fixture.deps);
    const install = await run(fixture, "ue4ss-install.ts", "--dry-run");
    assert.equal(install.code, 0);
    assert.match(install.stdout, /C:\/game\/Win64/u);
    assert.match(install.stdout, /dwmapi\.dll/u);
    const uninstall = await run(fixture, "ue4ss-uninstall.ts", "--dry-run");
    assert.equal(uninstall.code, 0);
    assert.match(uninstall.stdout, /ue4ss-manifest\.json/u);
    const sync = await run(fixture, "mod-sync.ts", "FfrProbe", "--dry-run");
    assert.equal(sync.code, 0);
    assert.match(sync.stdout, /ue4ss\/Mods\/FfrProbe/u);
    const log = await run(fixture, "ue4ss-log.ts", "--tag", "[FfrProbe]", "--dry-run");
    assert.equal(log.code, 0);
    assert.match(log.stdout, /UE4SS\.log/u);
    const launch = await run(fixture, "game-launch.ts", "--dry-run");
    assert.equal(launch.code, 0);
    assert.match(launch.stdout, /steam:\/\/rungameid\/12345/u);
    assert.deepEqual(await calls(fixture.log), []);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("the FfrProbe mod exists so mod-sync has something to copy", async () => {
  const source = join(
    import.meta.dirname,
    "..",
    "..",
    "..",
    "mod",
    "FfrProbe",
    "Scripts",
    "main.lua",
  );
  const body = await readFile(source, "utf8");
  assert.match(body, /FfrProbe/u);
  assert.match(body, /ExecuteInGameThread/u);
});

void test("ue4ss-install refuses an existing name and never overwrites", async () => {
  const fixture = await harness([
    { stdout: "STOPPED\n" },
    { stdout: '{"existing":["dwmapi.dll"]}\n' },
  ]);
  try {
    await prepareUe4ss(fixture.deps);
    const result = await run(fixture, "ue4ss-install.ts");
    assert.equal(result.code, 1);
    assert.match(result.stderr, /已拒绝覆盖/u);
    assert.equal((await calls(fixture.log)).length, 2);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("ue4ss-install refuses while the game is running", async () => {
  const fixture = await harness([{ stdout: "RUNNING:Game\n" }]);
  try {
    await prepareUe4ss(fixture.deps);
    const result = await run(fixture, "ue4ss-install.ts");
    assert.equal(result.code, 1);
    assert.match(result.stderr, /游戏正在运行/u);
    assert.equal((await calls(fixture.log)).length, 1);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("ue4ss-install copies dwmapi.dll and ue4ss, then writes the manifest", async () => {
  const fixture = await harness([
    { stdout: "STOPPED\n" },
    { stdout: '{"existing":[]}\n' },
    { stdout: "" },
    { stdout: "" },
  ]);
  try {
    await prepareUe4ss(fixture.deps);
    const result = await run(fixture, "ue4ss-install.ts");
    assert.equal(result.code, 0);
    assert.match(result.stdout, /已装 2 个文件/u);
    assert.match(result.stdout, /ue4ss-manifest\.json/u);
    const logged = await calls(fixture.log);
    assert.equal(logged.length, 4);
    const manifest = decodeScript(logged[2]);
    assert.match(manifest, /"gameDir":"C:\/game\/Win64"/u);
    assert.match(manifest, /"ue4ss\/Mods\/mods\.txt"/u);
    const copy = logged[3]?.args ?? [];
    assert.ok(copy.includes("-r"));
    assert.ok(copy.some((arg) => arg.endsWith("dwmapi.dll")));
    assert.ok(copy.includes("ffr@192.0.2.10:C:/game/Win64"));
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("ue4ss-uninstall removes only the listed files and then the manifest", async () => {
  const manifest = {
    gameDir: "C:/game/Win64",
    files: ["dwmapi.dll", "ue4ss/Mods/mods.txt"],
    dirs: ["ue4ss", "ue4ss/Mods"],
  };
  const fixture = await harness([
    { stdout: "STOPPED\n" },
    { stdout: `${JSON.stringify(manifest)}\n` },
    { stdout: "removed=2\n" },
  ]);
  try {
    const result = await run(fixture, "ue4ss-uninstall.ts");
    assert.equal(result.code, 0);
    assert.match(result.stdout, /removed=2/u);
    const logged = await calls(fixture.log);
    assert.equal(logged.length, 3);
    const script = decodeScript(logged[2]);
    assert.match(script, /Remove-Item/u);
    assert.match(script, /ue4ss-manifest\.json/u);
    assert.match(script, /ue4ss\/Mods\/mods\.txt/u);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("ue4ss-uninstall refuses unsafe paths in the manifest", async () => {
  const manifest = { gameDir: "C:/game/Win64", files: ["../evil"], dirs: [] };
  const fixture = await harness([
    { stdout: "STOPPED\n" },
    { stdout: `${JSON.stringify(manifest)}\n` },
  ]);
  try {
    const result = await run(fixture, "ue4ss-uninstall.ts");
    assert.equal(result.code, 1);
    assert.match(result.stderr, /不安全/u);
    assert.equal((await calls(fixture.log)).length, 2);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("mod-sync copies the mod and enables it in mods.txt", async () => {
  const fixture = await harness([
    { stdout: "STOPPED\n" },
    { stdout: `${JSON.stringify({ mods: "FfrProbe : 0\r\nOther : 1\r\n" })}\n` },
    { stdout: "" },
    { stdout: "" },
    { stdout: "" },
  ]);
  try {
    const result = await run(fixture, "mod-sync.ts", "FfrProbe");
    assert.equal(result.code, 0);
    assert.match(result.stdout, /已同步 FfrProbe/u);
    const logged = await calls(fixture.log);
    assert.equal(logged.length, 5);
    assert.ok(logged[3]?.args.some((arg) => arg.endsWith("/FfrProbe")));
    const mods = decodeScript(logged[4]);
    assert.match(mods, /FfrProbe : 1/u);
    assert.match(mods, /Other : 1/u);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("mod-sync --disable appends an off line", async () => {
  const fixture = await harness([
    { stdout: "STOPPED\n" },
    { stdout: `${JSON.stringify({ mods: "" })}\n` },
    { stdout: "" },
    { stdout: "" },
    { stdout: "" },
  ]);
  try {
    const result = await run(fixture, "mod-sync.ts", "FfrProbe", "--disable");
    assert.equal(result.code, 0);
    assert.match(result.stdout, /停用/u);
    const logged = await calls(fixture.log);
    const mods = decodeScript(logged[4]);
    assert.match(mods, /FfrProbe : 0/u);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("mod-sync FfrDump ensures OUT/hotkey exists on the game machine", async () => {
  const fixture = await harness([
    { stdout: "STOPPED\n" },
    { stdout: `${JSON.stringify({ mods: "" })}\n` },
    { stdout: "" },
    { stdout: "" },
    { stdout: "" },
  ]);
  try {
    const dry = await run(fixture, "mod-sync.ts", "FfrDump", "--dry-run");
    assert.equal(dry.code, 0);
    assert.match(dry.stdout, /ffr-out\/hotkey/u);
    const result = await run(fixture, "mod-sync.ts", "FfrDump");
    assert.equal(result.code, 0);
    const logged = await calls(fixture.log);
    assert.equal(logged.length, 5);
    const modsScript = decodeScript(logged[4]);
    assert.match(modsScript, /FfrDump : 1/u);
    assert.match(
      modsScript,
      /New-Item -ItemType Directory -Force -Path 'C:\/game\/Win64\/ue4ss\/ffr-out\/hotkey'/u,
    );
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("ue4ss-log prints the whole log or only tagged lines", async () => {
  const body = "[FfrProbe] loaded\nnoise\n[FfrProbe] tick 1\n";
  const fixture = await harness([{ stdout: body }, { stdout: body }]);
  try {
    const all = await run(fixture, "ue4ss-log.ts");
    assert.equal(all.code, 0);
    assert.equal(all.stdout, body);
    const tagged = await run(fixture, "ue4ss-log.ts", "--tag", "[FfrProbe]");
    assert.equal(tagged.code, 0);
    assert.equal(tagged.stdout, "[FfrProbe] loaded\n[FfrProbe] tick 1\n");
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("ue4ss-log reads while the game may be running (no running check)", async () => {
  const fixture = await harness([{ stdout: "line\n" }]);
  try {
    const result = await run(fixture, "ue4ss-log.ts");
    assert.equal(result.code, 0);
    assert.equal((await calls(fixture.log)).length, 1);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("game-launch writes a steam URL request and reads the task's status", async () => {
  const fixture = await harness([
    { stdout: "STOPPED\n" },
    { stdout: "" },
    { stdout: '{"state":"started"}\n' },
  ]);
  try {
    const result = await run(fixture, "game-launch.ts");
    assert.equal(result.code, 0);
    assert.match(result.stdout, /steam:\/\/rungameid\/12345/u);
    assert.match(result.stdout, /启动请求已交给计划任务/u);
    const logged = await calls(fixture.log);
    assert.equal(logged.length, 3);
    assert.match(decodeScript(logged[1]), /steam:\/\/rungameid\/12345/u);
    assert.match(decodeScript(logged[1]), /FFR-Agent-Session/u);
    assert.match(decodeScript(logged[2]), /\.status\.json/u);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("game-launch refuses while the game is running", async () => {
  const fixture = await harness([{ stdout: "RUNNING:Game\n" }]);
  try {
    const result = await run(fixture, "game-launch.ts");
    assert.equal(result.code, 1);
    assert.match(result.stderr, /游戏正在运行/u);
    assert.equal((await calls(fixture.log)).length, 1);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("game-launch needs FFR_APP_ID before it contacts the machine", async () => {
  const fixture = await harness([], { appId: false });
  try {
    const result = await run(fixture, "game-launch.ts");
    assert.equal(result.code, 1);
    assert.match(result.stderr, /FFR_APP_ID/u);
    assert.deepEqual(await calls(fixture.log), []);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});
