/**
 * The R1 dump commands, driven end to end with a fake ssh/scp on PATH. The fake scp can also
 * drop a prepared .tgz where scp would have written it, so the local unpack step is exercised.
 * Nothing here reads machine/machine.env or connects to a real game machine.
 */

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import type { Machine } from "../../../src/io/live/machine.ts";
import {
  JOBS,
  buildIdFromAcf,
  defaultTimeout,
  heartbeatPath,
  hotkeyDir,
  hotkeyMarkersPath,
  isHotkeySnapshotId,
  isJob,
  jobDir,
  jobId,
  localHotkeyDir,
  localHotkeyMarkersPath,
  localHotkeySnapshotDir,
  localHotkeyStatePath,
  localJobDir,
  outDir,
  parseDone,
  parseHeartbeat,
  parseHotkeySnapshotFile,
  parseJobId,
  parseLabel,
  remoteHotkeyPackage,
  remotePackage,
  triggerPath,
  triggerText,
} from "../../../src/host/cli/dump.ts";
import {
  deleteRemoteScript,
  flattenJobDirectory,
  unpackPackage,
} from "../../../src/host/cli/dump-fetch.ts";
import { deleteRemoteHotkeyScript } from "../../../src/host/cli/dump-fetch-hotkey.ts";

const CLI_DIR = join(import.meta.dirname, "..", "..", "..", "src", "host", "cli");
const FAKE_SSH = join(import.meta.dirname, "..", "..", "fixtures", "fake-ssh.mjs");
const ID = "20261124-031500-field-1-snapshot";

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
  readonly scpPackage?: string;
}

async function harness(
  responses: readonly Response[],
  options: HarnessOptions = {},
): Promise<Harness> {
  const directory = await mkdtemp(join(tmpdir(), "ffr-dump-"));
  const bin = join(directory, "bin");
  await mkdir(bin);
  const config = join(directory, "fake-ssh.json");
  const log = join(directory, "ssh.log");
  const machineEnv = join(directory, "machine.env");
  const deps = join(directory, "deps");
  await writeFile(config, JSON.stringify({ responses, captureUploaded: true }), "utf8");
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
  const fake = `FAKE_SSH_CONFIG=${shellQuote(config)} FAKE_SSH_LOG=${shellQuote(log)} `;
  const wrapper = `#!/bin/sh\n${fake}exec ${shellQuote(process.execPath)} ${shellQuote(FAKE_SSH)} "$@"\n`;
  for (const name of ["ssh", "scp"]) {
    const path = join(bin, name);
    await writeFile(path, wrapper, "utf8");
    await chmod(path, 0o755);
  }
  if (options.scpPackage) {
    const scpWrapper =
      "#!/bin/sh\n" +
      'for last in "$@"; do :; done\n' +
      'case "$last" in\n' +
      `  *.tgz) cp ${shellQuote(options.scpPackage)} "$last" || exit 1 ;;\n` +
      "esac\n" +
      `${fake}exec ${shellQuote(process.execPath)} ${shellQuote(FAKE_SSH)} "$@"\n`;
    const path = join(bin, "scp");
    await writeFile(path, scpWrapper, "utf8");
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

async function calls(
  path: string,
): Promise<{ args: string[]; stdin: string; uploadedBase64?: string }[]> {
  const body = await readFile(path, "utf8");
  return body
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line) as { args: string[]; stdin: string; uploadedBase64?: string });
}

function decodeUploadedScript(call: { uploadedBase64?: string } | undefined): string {
  if (!call?.uploadedBase64) return "";
  const raw = Buffer.from(call.uploadedBase64, "base64");
  const hasBom = raw.length >= 3 && raw[0] === 0xef && raw[1] === 0xbb && raw[2] === 0xbf;
  return (hasBom ? raw.subarray(3) : raw).toString("utf8");
}

function decodeScript(call: { args: string[] } | undefined): string {
  if (!call) return "";
  const index = call.args.indexOf("-EncodedCommand");
  const encoded = index >= 0 ? call.args[index + 1] : undefined;
  return encoded ? Buffer.from(encoded, "base64").toString("utf16le") : "";
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

async function makePackage(root: string, files: Record<string, string>): Promise<string> {
  const source = join(root, "tar-source");
  for (const [name, content] of Object.entries(files)) {
    const path = join(source, name);
    await mkdir(join(path, ".."), { recursive: true });
    await writeFile(path, content, "utf8");
  }
  const packagePath = join(root, "prepared.tgz");
  await new Promise<void>((resolve, reject) => {
    const child = spawn("tar", ["-czf", packagePath, "-C", source, "."], {
      stdio: ["ignore", "ignore", "pipe"],
    });
    const stderr: Buffer[] = [];
    child.stderr.on("data", (chunk: Buffer) => stderr.push(chunk));
    child.on("error", reject);
    child.on("close", (code) => {
      if (code === 0) resolve();
      else reject(new Error(Buffer.concat(stderr).toString("utf8")));
    });
  });
  return packagePath;
}

async function makeRealDumpPackage(root: string): Promise<string> {
  const game = join(root, "game");
  const job = join(game, "ue4ss", "ffr-out", "jobs", ID);
  await mkdir(job, { recursive: true });
  await writeFile(
    join(job, "meta.json"),
    JSON.stringify({
      id: ID,
      job: "snapshot",
      label: "field-1",
      startedAt: DONE.startedAt,
      finishedAt: DONE.finishedAt,
      files: 2,
      bytes: 30,
      buildid: null,
    }),
    "utf8",
  );
  await writeFile(join(job, "done.json"), JSON.stringify(DONE), "utf8");
  await writeFile(join(job, "context.json"), '{"scene":"field-1"}\n', "utf8");
  await writeFile(join(job, "UE4SS.log"), "log\n", "utf8");
  await writeFile(join(game, "ue4ss", "UE4SS_ObjectDump.txt"), "objects\n", "utf8");
  const paths = [
    `ue4ss/ffr-out/jobs/${ID}/meta.json`,
    `ue4ss/ffr-out/jobs/${ID}/done.json`,
    `ue4ss/ffr-out/jobs/${ID}/context.json`,
    `ue4ss/ffr-out/jobs/${ID}/UE4SS.log`,
    "ue4ss/UE4SS_ObjectDump.txt",
  ];
  const listPath = join(root, "files.txt");
  await writeFile(listPath, `${paths.join("\n")}\n`, "utf8");
  const packagePath = join(root, "prepared.tgz");
  await new Promise<void>((resolve, reject) => {
    const child = spawn("tar", ["-czf", packagePath, "-C", game, "-T", listPath], {
      stdio: ["ignore", "ignore", "pipe"],
    });
    const stderr: Buffer[] = [];
    child.stderr.on("data", (chunk: Buffer) => stderr.push(chunk));
    child.on("error", reject);
    child.on("close", (code) => {
      if (code === 0) resolve();
      else reject(new Error(Buffer.concat(stderr).toString("utf8")));
    });
  });
  return packagePath;
}

const DONE = {
  id: ID,
  job: "snapshot",
  label: "field-1",
  ok: true,
  error: null,
  startedAt: 1795000000,
  finishedAt: 1795000012,
  files: ["context.json"],
  stats: { objectsScanned: 12, objectsDumped: 3, truncated: false },
};

const ACF = '"AppState"\n{\n\t"appid"\t"12345"\n\t"buildid"\t"9876543"\n}\n';

const MACHINE: Machine = {
  host: "192.0.2.10",
  user: "ffr",
  key: "/tmp/id_ed25519",
  root: "C:/ffr",
  gameDir: "C:/game/Win64",
};

void test("parseLabel accepts the protocol labels and rejects unsafe ones", () => {
  assert.equal(parseLabel("title"), "title");
  assert.equal(parseLabel("field-1"), "field-1");
  assert.equal(parseLabel("battle-boss"), "battle-boss");
  for (const bad of ["", "..", "a/b", "Upper", "field_1", "a".repeat(33)]) {
    assert.throws(() => parseLabel(bad), /场面名/u);
  }
  assert.throws(() => parseLabel(undefined), /缺少场面名/u);
});

void test("parseJobId accepts the six jobs and rejects malformed ids", () => {
  assert.equal(parseJobId(ID), ID);
  for (const job of Object.keys(JOBS)) {
    assert.equal(parseJobId(`20261124-031500-field-1-${job}`), `20261124-031500-field-1-${job}`);
  }
  for (const bad of [
    "",
    "20261124-031500-../snapshot",
    "20261124-031500-Field-1-snapshot",
    `20261124-031500-${"a".repeat(33)}-snapshot`,
    "20261124-031500-field-1-probe",
    "2026112-031500-field-1-snapshot",
  ]) {
    assert.throws(() => parseJobId(bad), /任务号/u);
  }
});

void test("jobId uses UTC and the documented layout", () => {
  const now = new Date("2026-11-24T03:15:00.000Z");
  assert.equal(jobId(now, "field-1", "snapshot"), ID);
  assert.equal(
    jobId(new Date("2026-01-02T23:59:59.000Z"), "title", "uht"),
    "20260102-235959-title-uht",
  );
  assert.equal(defaultTimeout("snapshot"), 180);
  assert.equal(JOBS.jmap, 600);
  assert.equal(isJob("usmap"), true);
  assert.equal(isJob("probe"), false);
});

void test("triggerText is three CRLF lines", () => {
  assert.equal(
    triggerText(ID, "snapshot", "field-1"),
    `id=${ID}\r\njob=snapshot\r\nlabel=field-1\r\n`,
  );
});

void test("parseDone narrows every field and names the one that is wrong", () => {
  const done = parseDone(DONE);
  assert.equal(done.ok, true);
  assert.deepEqual(done.files, ["context.json"]);
  for (const key of Object.keys(DONE)) {
    const without = Object.fromEntries(
      Object.entries(DONE).filter(([name]) => name !== key),
    ) as Record<string, unknown>;
    assert.throws(() => parseDone(without), new RegExp(`缺少字段 ${key}`, "u"));
  }
  assert.throws(() => parseDone({ ...DONE, ok: "true" }), /ok 字段不是布尔值/u);
  assert.throws(() => parseDone({ ...DONE, files: [1] }), /files\[0\]/u);
  assert.throws(() => parseDone({ ...DONE, stats: [] }), /stats 字段不是对象/u);
  assert.throws(() => parseDone("nope"), /不是对象/u);
});

void test("parseHeartbeat reads the four fields", () => {
  const heartbeat = parseHeartbeat({ mod: "FfrDump", version: 1, time: 1795000000, busy: false });
  assert.deepEqual(heartbeat, { mod: "FfrDump", version: 1, time: 1795000000, busy: false });
  assert.throws(() => parseHeartbeat({ mod: "FfrDump" }), /缺少字段 version/u);
  assert.throws(() => parseHeartbeat({ ...heartbeat, busy: "no" }), /busy 字段不是布尔值/u);
});

void test("buildIdFromAcf reads buildid and returns null when absent", () => {
  const acf = [
    '"AppState"',
    "{",
    '\t"appid"\t\t"12345"',
    '\t"buildid"\t\t"9876543"',
    '\t"InstalledDepots"',
    "\t{",
    "\t}",
    "}",
    "",
  ].join("\n");
  assert.equal(buildIdFromAcf(acf), "9876543");
  assert.equal(buildIdFromAcf('"appid" "12345"'), null);
  assert.equal(buildIdFromAcf('"buildid" "unknown"'), null);
});

void test("the machine paths all come from machine.gameDir and machine.root", () => {
  assert.equal(outDir(MACHINE), "C:/game/Win64/ue4ss/ffr-out");
  assert.equal(heartbeatPath(MACHINE), "C:/game/Win64/ue4ss/ffr-out/heartbeat.json");
  assert.equal(triggerPath(MACHINE), "C:/game/Win64/ue4ss/ffr-out/trigger.txt");
  assert.equal(jobDir(MACHINE, ID), `C:/game/Win64/ue4ss/ffr-out/jobs/${ID}`);
  assert.equal(remotePackage(MACHINE, ID), `C:/ffr/ffr-dumps/${ID}.tgz`);
  const previous = process.env.FFR_DEPS;
  try {
    process.env.FFR_DEPS = "/tmp/ffr-test-deps";
    assert.equal(localJobDir(ID), join("/tmp/ffr-test-deps", "r1", ID));
  } finally {
    if (previous === undefined) delete process.env.FFR_DEPS;
    else process.env.FFR_DEPS = previous;
  }
});

void test("dump-run refuses when the game is not running", async () => {
  const fixture = await harness([{ stdout: "STOPPED\n" }]);
  try {
    const result = await run(
      fixture,
      "dump-run.ts",
      "snapshot",
      "--label",
      "field-1",
      "--no-fetch",
    );
    assert.equal(result.code, 1);
    assert.match(result.stderr, /游戏没在运行/u);
    assert.match(result.stderr, /game-launch/u);
    assert.equal((await calls(fixture.log)).length, 1);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("dump-run refuses when a trigger is already pending", async () => {
  const fixture = await harness([
    { stdout: "RUNNING:Game\n" },
    { code: 1, stderr: "上一个转储还没被模组取走:C:/game/Win64/ue4ss/ffr-out/trigger.txt\n" },
  ]);
  try {
    const result = await run(
      fixture,
      "dump-run.ts",
      "snapshot",
      "--label",
      "field-1",
      "--no-fetch",
    );
    assert.equal(result.code, 1);
    assert.match(result.stderr, /上一个转储还没被模组取走/u);
    assert.equal((await calls(fixture.log)).length, 2);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("dump-run reports a missing heartbeat and points at mod-sync", async () => {
  const fixture = await harness([
    { stdout: "RUNNING:Game\n" },
    { stdout: '{"heartbeat":null,"now":1795000000}\n' },
  ]);
  try {
    const result = await run(
      fixture,
      "dump-run.ts",
      "snapshot",
      "--label",
      "field-1",
      "--no-fetch",
    );
    assert.equal(result.code, 1);
    assert.match(result.stderr, /没有心跳/u);
    assert.match(result.stderr, /mod-sync FfrDump/u);
    // The trigger is not written when the heartbeat is missing.
    assert.equal((await calls(fixture.log)).length, 2);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("dump-run refuses a busy heartbeat before creating the job directory", async () => {
  const fixture = await harness([
    { stdout: "RUNNING:Game\n" },
    { code: 1, stderr: "FfrDump 正在执行上一个任务，用 dump-status 查看\n" },
  ]);
  try {
    const result = await run(
      fixture,
      "dump-run.ts",
      "snapshot",
      "--label",
      "field-1",
      "--no-fetch",
    );
    assert.equal(result.code, 1);
    assert.match(result.stderr, /正在执行上一个任务/u);
    assert.match(result.stderr, /dump-status/u);
    const logged = await calls(fixture.log);
    assert.equal(logged.length, 2);
    const prepare = decodeScript(logged[1]);
    const busyCheck = prepare.indexOf("$heartbeatValue.busy");
    const jobCreate = prepare.indexOf("New-Item -ItemType Directory -Force -Path $job");
    assert.ok(busyCheck >= 0 && jobCreate > busyCheck);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("dump-run writes the trigger, waits in one script, and stops on ok:false", async () => {
  const fixture = await harness([
    { stdout: "RUNNING:Game\n" },
    {
      stdout:
        '{"heartbeat":{"mod":"FfrDump","version":1,"time":1795000000,"busy":false},"now":1795000001}\n',
    },
    { stdout: "" },
    { stdout: `${JSON.stringify({ ...DONE, ok: false, error: "boom" })}\n` },
  ]);
  try {
    const result = await run(
      fixture,
      "dump-run.ts",
      "snapshot",
      "--label",
      "field-1",
      "--no-fetch",
    );
    assert.equal(result.code, 1);
    assert.match(result.stdout, /ok:false/u);
    assert.match(result.stdout, /error:boom/u);
    assert.match(result.stdout, /stats:/u);
    const logged = await calls(fixture.log);
    assert.equal(logged.length, 4);
    const trigger = decodeScript(logged[2]);
    assert.match(trigger, /id=\d{8}-\d{6}-field-1-snapshot/u);
    assert.match(trigger, /job=snapshot\r\nlabel=field-1/u);
    const wait = decodeScript(logged[3]);
    assert.match(wait, /done\.json/u);
    assert.match(wait, /trigger\.txt/u);
    assert.match(wait, /Start-Sleep/u);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("dump-run --dry-run prints the trigger and never opens ssh", async () => {
  const fixture = await harness([]);
  try {
    const result = await run(fixture, "dump-run.ts", "snapshot", "--label", "field-1", "--dry-run");
    assert.equal(result.code, 0);
    assert.match(result.stdout, /任务号:\d{8}-\d{6}-field-1-snapshot/u);
    assert.match(result.stdout, /trigger\.txt/u);
    assert.match(result.stdout, /job=snapshot/u);
    assert.match(result.stdout, /label=field-1/u);
    assert.match(result.stdout, /等待:180 秒/u);
    assert.deepEqual(await calls(fixture.log), []);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("dump-fetch refuses an existing local directory without touching ssh", async () => {
  const fixture = await harness([]);
  try {
    await mkdir(join(fixture.deps, "r1", ID), { recursive: true });
    const result = await run(fixture, "dump-fetch.ts", ID);
    assert.equal(result.code, 1);
    assert.match(result.stderr, /本地目录已存在/u);
    assert.deepEqual(await calls(fixture.log), []);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("dump-fetch removes the new local directory when scp fails", async () => {
  // fetchScript exceeds the cmd.exe limit so runPs uses mkdir + scp(.ps1) + -File + cleanup.
  const fixture = await harness([
    { stdout: "" },
    { stdout: "" },
    {
      stdout: `${JSON.stringify({
        doneText: JSON.stringify(DONE),
        files: 1,
        bytes: 20,
        acf: ACF,
        logCopied: false,
      })}\n`,
    },
    { stdout: "" },
    { code: 1, stderr: "scp failed\n" },
  ]);
  try {
    const result = await run(fixture, "dump-fetch.ts", ID);
    assert.equal(result.code, 1);
    await assert.rejects(stat(join(fixture.deps, "r1", ID)), (error: NodeJS.ErrnoException) => {
      return error.code === "ENOENT";
    });
    assert.equal((await calls(fixture.log)).length, 5);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("flattenJobDirectory moves job files and keeps UE4SS dumps", async () => {
  const root = await mkdtemp(join(tmpdir(), "ffr-flatten-"));
  try {
    const local = join(root, "r1", ID);
    const job = join(local, "ue4ss", "ffr-out", "jobs", ID);
    await mkdir(job, { recursive: true });
    await writeFile(join(job, "meta.json"), "{}", "utf8");
    await writeFile(join(job, "context.json"), "ctx", "utf8");
    await writeFile(join(local, "ue4ss", "UE4SS_ObjectDump.txt"), "objects\n", "utf8");
    await flattenJobDirectory(local, ID);
    assert.equal(await readFile(join(local, "meta.json"), "utf8"), "{}");
    assert.equal(await readFile(join(local, "context.json"), "utf8"), "ctx");
    assert.equal(await readFile(join(local, "ue4ss", "UE4SS_ObjectDump.txt"), "utf8"), "objects\n");
    await assert.rejects(
      stat(join(local, "ue4ss", "ffr-out")),
      (error: NodeJS.ErrnoException) => error.code === "ENOENT",
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

void test("flattenJobDirectory refuses to overwrite a root entry", async () => {
  const root = await mkdtemp(join(tmpdir(), "ffr-flatten-"));
  try {
    const local = join(root, "r1", ID);
    const job = join(local, "ue4ss", "ffr-out", "jobs", ID);
    await mkdir(job, { recursive: true });
    await writeFile(join(local, "meta.json"), "keep", "utf8");
    await writeFile(join(job, "meta.json"), "new", "utf8");
    await assert.rejects(flattenJobDirectory(local, ID), /同名条目/u);
    assert.equal(await readFile(join(local, "meta.json"), "utf8"), "keep");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

void test("unpackPackage extracts the tgz and deletes it", async () => {
  const root = await mkdtemp(join(tmpdir(), "ffr-unpack-"));
  try {
    const packagePath = await makePackage(root, {
      "context.json": '{"scene":"field-1"}\n',
      "nested/objects.jsonl": '{"name":"a"}\n',
    });
    const destination = join(root, "out");
    await unpackPackage(packagePath, destination);
    assert.equal(
      await readFile(join(destination, "context.json"), "utf8"),
      '{"scene":"field-1"}\n',
    );
    assert.equal(
      await readFile(join(destination, "nested", "objects.jsonl"), "utf8"),
      '{"name":"a"}\n',
    );
    await assert.rejects(
      stat(packagePath),
      (error: NodeJS.ErrnoException) => error.code === "ENOENT",
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

void test("dump-fetch packs, pulls, unpacks and deletes only its remote package", async () => {
  const packageRoot = await mkdtemp(join(tmpdir(), "ffr-package-"));
  try {
    const packagePath = await makeRealDumpPackage(packageRoot);
    // fetchScript runs via the long-script path: mkdir tmp, upload .ps1, run -File, delete .ps1;
    // then scp pulls package.tgz and a short script deletes only the remote .tgz.
    const fixture = await harness(
      [
        { stdout: "" },
        { stdout: "" },
        {
          stdout: `${JSON.stringify({
            doneText: JSON.stringify(DONE),
            files: 1,
            bytes: 20,
            acf: ACF,
            logCopied: false,
          })}\n`,
        },
        { stdout: "" },
        { stdout: "" },
        { stdout: "" },
      ],
      { scpPackage: packagePath },
    );
    try {
      const result = await run(fixture, "dump-fetch.ts", ID);
      assert.equal(result.code, 0);
      assert.match(result.stdout, /本地目录:/u);
      assert.match(result.stdout, /文件数:1/u);
      assert.match(result.stdout, /buildid:9876543/u);
      const local = join(fixture.deps, "r1", ID);
      assert.equal(await readFile(join(local, "context.json"), "utf8"), '{"scene":"field-1"}\n');
      assert.equal(await readFile(join(local, "done.json"), "utf8"), JSON.stringify(DONE));
      assert.equal(
        await readFile(join(local, "ue4ss", "UE4SS_ObjectDump.txt"), "utf8"),
        "objects\n",
      );
      await assert.rejects(
        stat(join(local, "ue4ss", "ffr-out")),
        (error: NodeJS.ErrnoException) => error.code === "ENOENT",
      );
      const meta = JSON.parse(await readFile(join(local, "meta.json"), "utf8")) as {
        buildid: unknown;
      };
      assert.equal(meta.buildid, "9876543");
      await assert.rejects(stat(join(local, "package.tgz")), (error: NodeJS.ErrnoException) => {
        return error.code === "ENOENT";
      });
      const logged = await calls(fixture.log);
      assert.equal(logged.length, 6);
      assert.ok(logged[2]?.args.includes("-File"));
      const fetch = decodeUploadedScript(logged[1]);
      assert.match(fetch, /files\.txt/u);
      assert.match(fetch, /System32\/tar\.exe/u);
      assert.match(fetch, /\.\.\/\.\.\/\.\.\/\.\.\/\.\.\/appmanifest_/u);
      assert.doesNotMatch(fetch, /steamapps/u);
      assert.match(fetch, /\[string\]\[char\]10/u);
      assert.doesNotMatch(fetch, /\[char\]13/u);
      assert.doesNotMatch(fetch, /\[regex\]::Match/u);
      assert.match(fetch, /doneText = \$doneText/u);
      const remove = decodeScript(logged[5]);
      assert.match(remove, new RegExp(`ffr-dumps/${ID}\\.tgz`, "u"));
      assert.equal((remove.match(/Remove-Item/gu) ?? []).length, 1);
      assert.doesNotMatch(remove, /ffr-out|Mods/u);
      assert.equal(
        deleteRemoteScript(MACHINE, ID),
        `$package = 'C:/ffr/ffr-dumps/${ID}.tgz'\n` +
          "if (Test-Path -LiteralPath $package -PathType Leaf) { " +
          "Remove-Item -LiteralPath $package -Force }\n",
      );
    } finally {
      await rm(fixture.directory, { recursive: true, force: true });
    }
  } finally {
    await rm(packageRoot, { recursive: true, force: true });
  }
});

void test("dump-status prints the heartbeat age, busy flag and pending trigger", async () => {
  const older = "20261124-020000-title-snapshot";
  const fixture = await harness([
    { stdout: "RUNNING:Game\n" },
    {
      stdout: `${JSON.stringify({
        heartbeat: { mod: "FfrDump", version: 1, time: 1795000000, busy: true },
        now: 1795000012,
        trigger: true,
        jobs: [
          { id: ID, done: true, ok: false },
          { id: older, done: false, ok: null },
        ],
      })}\n`,
    },
  ]);
  try {
    const result = await run(fixture, "dump-status.ts");
    assert.equal(result.code, 0);
    assert.match(result.stdout, /游戏:正在运行/u);
    assert.match(result.stdout, /心跳:12 秒前,busy=true/u);
    assert.match(result.stdout, /待处理触发文件:有/u);
    assert.match(result.stdout, new RegExp(`${ID} done=有 ok=false`, "u"));
    assert.match(result.stdout, new RegExp(`${older} done=没有 ok=\\?`, "u"));
    assert.equal((await calls(fixture.log)).length, 2);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("dump-status rejects a non-array jobs field and a malformed heartbeat", async () => {
  const fixture = await harness([
    { stdout: "RUNNING:Game\n" },
    {
      stdout: `${JSON.stringify({
        heartbeat: { mod: "FfrDump", version: 1, time: 1795000000, busy: false },
        now: 1795000012,
        trigger: false,
        jobs: {},
      })}\n`,
    },
    { stdout: "RUNNING:Game\n" },
    {
      stdout: `${JSON.stringify({
        heartbeat: { mod: "FfrDump" },
        now: 1795000012,
        trigger: false,
        jobs: [],
      })}\n`,
    },
  ]);
  try {
    const jobs = await run(fixture, "dump-status.ts");
    assert.equal(jobs.code, 1);
    assert.match(jobs.stderr, /jobs 不是数组/u);
    const heartbeat = await run(fixture, "dump-status.ts");
    assert.equal(heartbeat.code, 1);
    assert.match(heartbeat.stderr, /缺少字段 version/u);
    assert.equal((await calls(fixture.log)).length, 4);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("dump-status --dry-run never opens ssh", async () => {
  const fixture = await harness([]);
  try {
    const result = await run(fixture, "dump-status.ts", "--dry-run");
    assert.equal(result.code, 0);
    assert.match(result.stdout, /heartbeat\.json/u);
    assert.match(result.stdout, /ffr-out\/hotkey/u);
    assert.deepEqual(await calls(fixture.log), []);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("hotkey paths and filename parsers validate hotkey snapshot ids and files", () => {
  const hkId = "hotkey-0003-20261125-023045";
  assert.equal(isHotkeySnapshotId(hkId), true);
  assert.equal(isHotkeySnapshotId("hotkey-12345-20261125-023045"), true);
  for (const bad of ["hotkey-1-20261125-023045", "../hotkey-0001-20261125-023045", "field-1"]) {
    assert.equal(isHotkeySnapshotId(bad), false);
  }
  assert.deepEqual(parseHotkeySnapshotFile(`${hkId}-objects.jsonl`), {
    id: hkId,
    file: "objects.jsonl",
  });
  assert.deepEqual(parseHotkeySnapshotFile(`${hkId}-done.json`), {
    id: hkId,
    file: "done.json",
  });
  assert.equal(parseHotkeySnapshotFile(`${hkId}-evil.txt`), null);
  assert.equal(parseHotkeySnapshotFile("markers.jsonl"), null);

  assert.equal(hotkeyDir(MACHINE), "C:/game/Win64/ue4ss/ffr-out/hotkey");
  assert.equal(hotkeyMarkersPath(MACHINE), "C:/game/Win64/ue4ss/ffr-out/hotkey/markers.jsonl");
  assert.equal(remoteHotkeyPackage(MACHINE, "abc"), "C:/ffr/ffr-dumps/hotkey-abc.tgz");
  const previous = process.env.FFR_DEPS;
  try {
    process.env.FFR_DEPS = "/tmp/ffr-test-deps";
    assert.equal(localHotkeyDir(), join("/tmp/ffr-test-deps", "r1", "hotkey"));
    assert.equal(localHotkeySnapshotDir(hkId), join("/tmp/ffr-test-deps", "r1", "hotkey", hkId));
    assert.equal(
      localHotkeyMarkersPath(),
      join("/tmp/ffr-test-deps", "r1", "hotkey", "markers.jsonl"),
    );
    assert.equal(
      localHotkeyStatePath(),
      join("/tmp/ffr-test-deps", "r1", "hotkey", "fetched.json"),
    );
  } finally {
    if (previous === undefined) delete process.env.FFR_DEPS;
    else process.env.FFR_DEPS = previous;
  }
});

void test("dump-run prepareScript creates OUT/hotkey alongside OUT", async () => {
  const fixture = await harness([
    { stdout: "RUNNING:Game\n" },
    { stdout: '{"heartbeat":null,"now":1795000000}\n' },
  ]);
  try {
    await run(fixture, "dump-run.ts", "snapshot", "--label", "field-1", "--no-fetch");
    const logged = await calls(fixture.log);
    const prepare = decodeScript(logged[1]);
    assert.match(prepare, /ffr-out\/hotkey/u);
    assert.match(prepare, /New-Item -ItemType Directory -Force -Path \$hotkey/u);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("dump-status prints hotkey snapshot and marker counts and rejects malformed hotkey", async () => {
  const fixture = await harness([
    { stdout: "RUNNING:Game\n" },
    {
      stdout: `${JSON.stringify({
        heartbeat: { mod: "FfrDump", version: 1, time: 1795000000, busy: false },
        now: 1795000005,
        trigger: false,
        jobs: [],
        hotkey: { snapshots: 3, markers: 7 },
      })}\n`,
    },
    { stdout: "RUNNING:Game\n" },
    {
      stdout: `${JSON.stringify({
        heartbeat: { mod: "FfrDump", version: 1, time: 1795000000, busy: false },
        now: 1795000005,
        trigger: false,
        jobs: [],
        hotkey: { snapshots: -1, markers: 0 },
      })}\n`,
    },
  ]);
  try {
    const ok = await run(fixture, "dump-status.ts");
    assert.equal(ok.code, 0);
    assert.match(ok.stdout, /热键快照:3 个,标记:7 条/u);
    const bad = await run(fixture, "dump-status.ts");
    assert.equal(bad.code, 1);
    assert.match(bad.stderr, /hotkey\.snapshots/u);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("dump-fetch-hotkey --dry-run prints paths and never opens ssh", async () => {
  const fixture = await harness([]);
  try {
    const result = await run(fixture, "dump-fetch-hotkey.ts", "--dry-run");
    assert.equal(result.code, 0);
    assert.match(result.stdout, /ffr-out\/hotkey/u);
    assert.match(result.stdout, /markers\.jsonl/u);
    assert.match(result.stdout, /fetched\.json/u);
    assert.deepEqual(await calls(fixture.log), []);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

void test("dump-fetch-hotkey fetches incrementally, organizes snapshot dirs and appends markers", async () => {
  const packageRoot = await mkdtemp(join(tmpdir(), "ffr-hotkey-pkg-"));
  const HK1 = "hotkey-0001-20261125-023000";
  const HK2 = "hotkey-0002-20261125-023100";
  const done1 = {
    id: HK1,
    job: "snapshot",
    label: "hotkey-0001",
    ok: true,
    error: null,
    startedAt: 1795458600,
    finishedAt: 1795458604,
    files: [
      `${HK1}-context.json`,
      `${HK1}-classes.json`,
      `${HK1}-objects.jsonl`,
      `${HK1}-widgets.jsonl`,
      `${HK1}-texts.jsonl`,
    ],
    stats: { objectsScanned: 100, objectsDumped: 10, truncated: false },
  };
  const done2 = {
    ...done1,
    id: HK2,
    label: "hotkey-0002",
    startedAt: 1795458660,
    finishedAt: 1795458665,
    files: [
      `${HK2}-context.json`,
      `${HK2}-classes.json`,
      `${HK2}-objects.jsonl`,
      `${HK2}-widgets.jsonl`,
      `${HK2}-texts.jsonl`,
    ],
  };
  const marker1 =
    '{"clock_ms":1000,"localTime":"2026-11-25 10:30:00","seq":1,"time":1795458600,"world":"World /Game/A.A"}\n';
  const marker2 =
    '{"clock_ms":2000,"localTime":"2026-11-25 10:30:05","seq":2,"time":1795458605,"world":"World /Game/A.A"}\n';
  const marker3 =
    '{"clock_ms":9000,"localTime":"2026-11-25 10:31:00","seq":3,"time":1795458660,"world":"World /Game/B.B"}\n';
  const firstMarkersBuf = Buffer.from(marker1 + marker2, "utf8");
  const thirdMarkerBuf = Buffer.from(marker3, "utf8");

  try {
    const pkg1 = await makePackage(join(packageRoot, "p1"), {
      [`${HK1}-context.json`]: '{"world":"World /Game/A.A"}\n',
      [`${HK1}-classes.json`]: "[]\n",
      [`${HK1}-objects.jsonl`]: '{"name":"Chest1"}\n',
      [`${HK1}-widgets.jsonl`]: "",
      [`${HK1}-texts.jsonl`]: "",
      [`${HK1}-done.json`]: JSON.stringify(done1),
    });
    const pkg2 = await makePackage(join(packageRoot, "p2"), {
      [`${HK2}-context.json`]: '{"world":"World /Game/B.B"}\n',
      [`${HK2}-classes.json`]: "[]\n",
      [`${HK2}-objects.jsonl`]: '{"name":"Chest2"}\n',
      [`${HK2}-widgets.jsonl`]: "",
      [`${HK2}-texts.jsonl`]: "",
      [`${HK2}-done.json`]: JSON.stringify(done2),
    });

    // Run 1: fetch HK1 + 2 markers (long script: mkdir tmp, scp .ps1, -File, delete .ps1; then scp .tgz, delete .tgz).
    const fixture1 = await harness(
      [
        { stdout: "" },
        { stdout: "" },
        {
          stdout: `${JSON.stringify({
            exists: true,
            snapshots: [
              {
                id: HK1,
                doneText: JSON.stringify(done1),
                files: [
                  `${HK1}-classes.json`,
                  `${HK1}-context.json`,
                  `${HK1}-done.json`,
                  `${HK1}-objects.jsonl`,
                  `${HK1}-texts.jsonl`,
                  `${HK1}-widgets.jsonl`,
                ],
                bytes: 200,
              },
            ],
            packageCreated: true,
            files: 6,
            bytes: 200,
            acf: ACF,
            markersReset: false,
            markersBytes: firstMarkersBuf.byteLength,
            markersDeltaBase64: firstMarkersBuf.toString("base64"),
          })}\n`,
        },
        { stdout: "" },
        { stdout: "" },
        { stdout: "" },
      ],
      { scpPackage: pkg1 },
    );
    try {
      const r1 = await run(fixture1, "dump-fetch-hotkey.ts");
      assert.equal(r1.code, 0, r1.stderr);
      assert.match(r1.stdout, /新增热键快照:1 个\(累计 1 个\)/u);
      assert.match(r1.stdout, /新增时间标记:2 条\(累计 2 条\)/u);
      assert.match(r1.stdout, /buildid:9876543/u);

      const hk1Dir = join(fixture1.deps, "r1", "hotkey", HK1);
      assert.equal(
        await readFile(join(hk1Dir, "context.json"), "utf8"),
        '{"world":"World /Game/A.A"}\n',
      );
      assert.equal(await readFile(join(hk1Dir, "objects.jsonl"), "utf8"), '{"name":"Chest1"}\n');
      assert.equal(await readFile(join(hk1Dir, "done.json"), "utf8"), JSON.stringify(done1));
      const meta1 = JSON.parse(await readFile(join(hk1Dir, "meta.json"), "utf8")) as {
        id: string;
        label: string;
        buildid: string | null;
      };
      assert.equal(meta1.id, HK1);
      assert.equal(meta1.label, "hotkey-0001");
      assert.equal(meta1.buildid, "9876543");
      assert.equal(
        await readFile(join(fixture1.deps, "r1", "hotkey", "markers.jsonl"), "utf8"),
        marker1 + marker2,
      );
      const state1 = JSON.parse(
        await readFile(join(fixture1.deps, "r1", "hotkey", "fetched.json"), "utf8"),
      ) as { snapshots: string[]; markersBytes: number; markersLines: number };
      assert.deepEqual(state1.snapshots, [HK1]);
      assert.equal(state1.markersBytes, firstMarkersBuf.byteLength);
      assert.equal(state1.markersLines, 2);

      const logged1 = await calls(fixture1.log);
      assert.equal(logged1.length, 6);
      assert.ok(
        logged1[2]?.args.includes("-File"),
        "fetchHotkeyScript must use Task 07 long-script path",
      );
      const remove1 = decodeScript(logged1[5]);
      assert.match(remove1, /ffr-dumps\/hotkey-/u);
      assert.doesNotMatch(remove1, /ffr-out|Mods/u);
      assert.equal(
        deleteRemoteHotkeyScript("C:/ffr/ffr-dumps/hotkey-abc.tgz"),
        "$package = 'C:/ffr/ffr-dumps/hotkey-abc.tgz'\n" +
          "if (Test-Path -LiteralPath $package -PathType Leaf) { " +
          "Remove-Item -LiteralPath $package -Force }\n",
      );

      // Run 2 (reusing fixture1's deps dir): nothing new -> no scp or delete call.
      const fixture2 = await harness([
        { stdout: "" },
        { stdout: "" },
        {
          stdout: `${JSON.stringify({
            exists: true,
            snapshots: [],
            packageCreated: false,
            files: 0,
            bytes: 0,
            acf: null,
            markersReset: false,
            markersBytes: firstMarkersBuf.byteLength,
            markersDeltaBase64: "",
          })}\n`,
        },
        { stdout: "" },
      ]);
      try {
        const r2 = await run(
          { ...fixture2, env: { ...fixture2.env, FFR_DEPS: fixture1.deps } },
          "dump-fetch-hotkey.ts",
        );
        assert.equal(r2.code, 0, r2.stderr);
        assert.match(r2.stdout, /没有新的热键快照或时间标记/u);
        const logged2 = await calls(fixture2.log);
        assert.equal(logged2.length, 4, "no scp or remote tgz delete when nothing is new");
        const ps2 = decodeUploadedScript(logged2[1]);
        assert.match(ps2, new RegExp(`\\$already\\.Add\\('${HK1}'\\)`, "u"));
        assert.match(
          ps2,
          new RegExp(`\\$markersOffset = \\[long\\]${firstMarkersBuf.byteLength}`, "u"),
        );
      } finally {
        await rm(fixture2.directory, { recursive: true, force: true });
      }

      // Run 3 (reusing fixture1's deps dir): incremental HK2 + 1 appended marker line.
      const fixture3 = await harness(
        [
          { stdout: "" },
          { stdout: "" },
          {
            stdout: `${JSON.stringify({
              exists: true,
              snapshots: [
                {
                  id: HK2,
                  doneText: JSON.stringify(done2),
                  files: [
                    `${HK2}-classes.json`,
                    `${HK2}-context.json`,
                    `${HK2}-done.json`,
                    `${HK2}-objects.jsonl`,
                    `${HK2}-texts.jsonl`,
                    `${HK2}-widgets.jsonl`,
                  ],
                  bytes: 210,
                },
              ],
              packageCreated: true,
              files: 6,
              bytes: 210,
              acf: ACF,
              markersReset: false,
              markersBytes: firstMarkersBuf.byteLength + thirdMarkerBuf.byteLength,
              markersDeltaBase64: thirdMarkerBuf.toString("base64"),
            })}\n`,
          },
          { stdout: "" },
          { stdout: "" },
          { stdout: "" },
        ],
        { scpPackage: pkg2 },
      );
      try {
        const r3 = await run(
          { ...fixture3, env: { ...fixture3.env, FFR_DEPS: fixture1.deps } },
          "dump-fetch-hotkey.ts",
        );
        assert.equal(r3.code, 0, r3.stderr);
        assert.match(r3.stdout, /新增热键快照:1 个\(累计 2 个\)/u);
        assert.match(r3.stdout, /新增时间标记:1 条\(累计 3 条\)/u);
        const hk2Dir = join(fixture1.deps, "r1", "hotkey", HK2);
        assert.equal(await readFile(join(hk2Dir, "objects.jsonl"), "utf8"), '{"name":"Chest2"}\n');
        assert.equal(await readFile(join(hk1Dir, "objects.jsonl"), "utf8"), '{"name":"Chest1"}\n');
        assert.equal(
          await readFile(join(fixture1.deps, "r1", "hotkey", "markers.jsonl"), "utf8"),
          marker1 + marker2 + marker3,
        );
        const state3 = JSON.parse(
          await readFile(join(fixture1.deps, "r1", "hotkey", "fetched.json"), "utf8"),
        ) as { snapshots: string[]; markersBytes: number; markersLines: number };
        assert.deepEqual(state3.snapshots, [HK1, HK2]);
        assert.equal(state3.markersBytes, firstMarkersBuf.byteLength + thirdMarkerBuf.byteLength);
        assert.equal(state3.markersLines, 3);
      } finally {
        await rm(fixture3.directory, { recursive: true, force: true });
      }
    } finally {
      await rm(fixture1.directory, { recursive: true, force: true });
    }
  } finally {
    await rm(packageRoot, { recursive: true, force: true });
  }
});

void test("dump-fetch-hotkey pulls markers-only updates without scp and leaves state clean on scp failure", async () => {
  const markerLine =
    '{"clock_ms":500,"localTime":"2026-11-25 10:35:00","seq":1,"time":1795458900,"world":"World /Game/C.C"}\n';
  const markerBuf = Buffer.from(markerLine, "utf8");
  const fixture = await harness([
    { stdout: "" },
    { stdout: "" },
    {
      stdout: `${JSON.stringify({
        exists: true,
        snapshots: [],
        packageCreated: false,
        files: 0,
        bytes: 0,
        acf: null,
        markersReset: false,
        markersBytes: markerBuf.byteLength,
        markersDeltaBase64: markerBuf.toString("base64"),
      })}\n`,
    },
    { stdout: "" },
  ]);
  try {
    const res = await run(fixture, "dump-fetch-hotkey.ts");
    assert.equal(res.code, 0, res.stderr);
    assert.match(res.stdout, /新增热键快照:0 个\(累计 0 个\)/u);
    assert.match(res.stdout, /新增时间标记:1 条\(累计 1 条\)/u);
    assert.equal((await calls(fixture.log)).length, 4);
    assert.equal(
      await readFile(join(fixture.deps, "r1", "hotkey", "markers.jsonl"), "utf8"),
      markerLine,
    );
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
  }

  const HK = "hotkey-0001-20261125-023500";
  const done = {
    id: HK,
    job: "snapshot",
    label: "hotkey-0001",
    ok: true,
    error: null,
    startedAt: 1795458900,
    finishedAt: 1795458902,
    files: [`${HK}-context.json`],
    stats: { objectsScanned: 1, objectsDumped: 1, truncated: false },
  };
  const failFixture = await harness([
    { stdout: "" },
    { stdout: "" },
    {
      stdout: `${JSON.stringify({
        exists: true,
        snapshots: [
          { id: HK, doneText: JSON.stringify(done), files: [`${HK}-done.json`], bytes: 10 },
        ],
        packageCreated: true,
        files: 1,
        bytes: 10,
        acf: null,
        markersReset: false,
        markersBytes: 0,
        markersDeltaBase64: "",
      })}\n`,
    },
    { stdout: "" },
    { code: 1, stderr: "scp failed\n" },
  ]);
  try {
    const failed = await run(failFixture, "dump-fetch-hotkey.ts");
    assert.equal(failed.code, 1);
    await assert.rejects(
      stat(join(failFixture.deps, "r1", "hotkey", HK)),
      (error: NodeJS.ErrnoException) => error.code === "ENOENT",
    );
    await assert.rejects(
      stat(join(failFixture.deps, "r1", "hotkey", "fetched.json")),
      (error: NodeJS.ErrnoException) => error.code === "ENOENT",
    );
  } finally {
    await rm(failFixture.directory, { recursive: true, force: true });
  }
});
