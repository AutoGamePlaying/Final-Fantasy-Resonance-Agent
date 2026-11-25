import assert from "node:assert/strict";
import { chmod, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  CalledProcessError,
  ConfigError,
  MAX_ENCODED_COMMAND_LENGTH,
  PS_PRELUDE,
  SESSION_TASK,
  decodeClixml,
  decodeStderr,
  encodePs,
  encodedPsCommandLength,
  isGameRunning,
  loadMachine,
  psQuote,
  reasonOf,
  remoteJoin,
  runInSession,
  runPs,
  scp,
  sessionRequest,
  sessionTaskOf,
  ssh,
  type Machine,
} from "../../src/io/live/machine.ts";

const FAKE_SSH = join(import.meta.dirname, "..", "fixtures", "fake-ssh.mjs");

const MACHINE: Machine = {
  host: "192.0.2.10",
  user: "ffr",
  key: "/home/dev/.ssh/ffr_laptop_ed25519",
  root: "C:/ffr",
  gameDir: "C:/game/Win64",
  appId: "12345",
};

void test("psQuote doubles single quotes; encodePs is UTF-16LE base64", () => {
  assert.equal(psQuote("C:/ffr/it's"), "'C:/ffr/it''s'");
  const script = "Write-Host '你好'";
  const encoded = encodePs(script);
  assert.equal(Buffer.from(encoded, "base64").toString("utf16le"), PS_PRELUDE + script);
});

void test("remoteJoin uses forward slashes and does not duplicate them", () => {
  assert.equal(remoteJoin("C:/game/Win64/", "ue4ss", "/Mods/"), "C:/game/Win64/ue4ss/Mods");
  assert.equal(remoteJoin("C:\\game\\Win64", "ue4ss"), "C:\\game\\Win64/ue4ss");
});

void test("decodeClixml recovers errors and warnings", () => {
  const input =
    "#< CLIXML\r\n" +
    '<Objs Version="1.1.0.1"><S S="Error">找不到文件_x000D__x000A_</S>' +
    '<S S="warning">注意一下_x000D__x000A_</S></Objs>';
  assert.equal(decodeClixml(input), "找不到文件\n警告:注意一下\n");
  assert.equal(decodeClixml("plain\n"), "plain\n");
});

void test("loadMachine reports missing files and missing keys", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-machine-"));
  try {
    const path = join(directory, "machine.env");
    await assert.rejects(loadMachine(path), (error: unknown) => {
      assert.ok(error instanceof ConfigError);
      assert.match(error.message, /machine\.env\.example/);
      return true;
    });
    await writeFile(path, "LAPTOP_HOST=192.0.2.10\n", "utf8");
    await assert.rejects(loadMachine(path), /LAPTOP_USER/);
    await writeFile(
      path,
      "LAPTOP_HOST=192.0.2.10\n" +
        "LAPTOP_USER=ffr\n" +
        "LAPTOP_ROOT=C:/ffr/\n" +
        "FFR_GAME_DIR=C:/game/Win64/\n" +
        "FFR_APP_ID=12345\n",
      "utf8",
    );
    const machine = await loadMachine(path);
    assert.equal(machine.host, "192.0.2.10");
    assert.equal(machine.root, "C:/ffr");
    assert.equal(machine.gameDir, "C:/game/Win64");
    assert.equal(machine.appId, "12345");
    assert.equal(machine.sessionTask, SESSION_TASK);
    assert.equal(sessionTaskOf(machine), SESSION_TASK);
    assert.equal(machine.key, join(homedir(), ".ssh", "ffr_laptop_ed25519"));
    assert.equal(machine.sshCommand, "ssh");
    assert.equal(`${machine.user}@${machine.host}`, "ffr@192.0.2.10");
    assert.ok(ssh(machine).includes("BatchMode=yes"));
    assert.ok(ssh(machine).includes("IdentitiesOnly=yes"));
    assert.ok(scp(machine).includes("-r"));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("loadMachine reads LAPTOP_SESSION_TASK for reusing a sibling task", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-machine-"));
  try {
    const path = join(directory, "machine.env");
    await writeFile(
      path,
      "LAPTOP_HOST=192.0.2.10\n" +
        "LAPTOP_USER=ffr\n" +
        "LAPTOP_ROOT=C:/ffr\n" +
        "FFR_GAME_DIR=C:/game/Win64\n" +
        "LAPTOP_SESSION_TASK=FFPR-Agent-Session\n",
      "utf8",
    );
    const machine = await loadMachine(path);
    assert.equal(machine.sessionTask, "FFPR-Agent-Session");
    assert.equal(sessionTaskOf(machine), "FFPR-Agent-Session");
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("runPs sends an EncodedCommand over the configured ssh and returns stdout", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-ssh-"));
  try {
    const log = await useFakeSsh(directory, [{ stdout: "你好\n" }]);
    const machine = { ...MACHINE, sshCommand: FAKE_SSH };
    const script = "Write-Host '你好'";
    const result = await runPs(machine, script, { capture: true });
    assert.equal(result.stdout, "你好\n");
    const [call] = await readSshLog(log);
    assert.ok(call);
    const index = call.args.indexOf("-EncodedCommand");
    assert.ok(index >= 0);
    const encoded = call.args[index + 1];
    assert.ok(encoded);
    assert.equal(Buffer.from(encoded, "base64").toString("utf16le"), PS_PRELUDE + script);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("runPs raises CalledProcessError by default and can skip the check", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-ssh-"));
  try {
    await useFakeSsh(directory, [
      { stdout: "", stderr: "坏了", code: 3 },
      { stdout: "", stderr: "坏了", code: 3 },
    ]);
    const machine = { ...MACHINE, sshCommand: FAKE_SSH };
    await assert.rejects(runPs(machine, "throw"), (error: unknown) => {
      assert.ok(error instanceof CalledProcessError);
      assert.equal(error.returnCode, 3);
      assert.match(error.message, /坏了/);
      return true;
    });
    const result = await runPs(machine, "throw", { check: false });
    assert.equal(result.returnCode, 3);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("isGameRunning parses STOPPED and RUNNING answers", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-ssh-"));
  try {
    await useFakeSsh(directory, [{ stdout: "STOPPED\n" }, { stdout: "RUNNING:Game,Crash\n" }]);
    const machine = { ...MACHINE, sshCommand: FAKE_SSH };
    assert.deepEqual(await isGameRunning(machine), []);
    assert.deepEqual(await isGameRunning(machine), ["Game", "Crash"]);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("runInSession returns the status the scheduled task wrote", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-ssh-"));
  try {
    const log = await useFakeSsh(directory, [{ stdout: "" }, { stdout: '{"state":"started"}' }]);
    const machine = { ...MACHINE, sshCommand: FAKE_SSH };
    const request = sessionRequest("steam://rungameid/12345", [], null, false);
    const status = await runInSession(machine, request, 5);
    assert.equal(status.state, "started");
    const calls = await readSshLog(log);
    assert.equal(calls.length, 2);
    const encoded = calls[0]?.args.at(-1) ?? "";
    const script = Buffer.from(encoded, "base64").toString("utf16le");
    assert.match(script, /steam:\/\/rungameid\/12345/u);
    assert.match(script, /FFR-Agent-Session/u);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("runInSession triggers the configured session task", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-ssh-"));
  try {
    const log = await useFakeSsh(directory, [{ stdout: "" }, { stdout: '{"state":"started"}' }]);
    const machine = { ...MACHINE, sshCommand: FAKE_SSH, sessionTask: "FFPR-Agent-Session" };
    const request = sessionRequest("steam://rungameid/12345", [], null, false);
    await runInSession(machine, request, 5);
    const calls = await readSshLog(log);
    const script = Buffer.from(calls[0]?.args.at(-1) ?? "", "base64").toString("utf16le");
    assert.match(script, /FFPR-Agent-Session/u);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("a failed command reports PowerShell's reason, not its encoded command line", () => {
  const stderr = [
    "游戏正在运行,请先退出",
    "所在位置 C:\\ffr\\deploy\\laptop\\run-in-session.ps1:1 字符: 1",
    "    + CategoryInfo          : OperationStopped: (游戏正在运行:String) [], RuntimeException",
    "",
  ].join("\r\n");
  assert.equal(reasonOf(stderr), "游戏正在运行,请先退出");
  const error = new CalledProcessError(1, ["ssh", "-EncodedCommand", "WwBD"], "", stderr);
  assert.equal(error.message, "命令退出码 1:游戏正在运行,请先退出");
});

// GBK (CP936) bytes for "命令行太长。\r\n"
const GBK_CMD_TOO_LONG = Buffer.from([
  0xc3, 0xfc, 0xc1, 0xee, 0xd0, 0xd0, 0xcc, 0xab, 0xb3, 0xa4, 0xa1, 0xa3, 0x0d, 0x0a,
]);

void test("decodeStderr preserves UTF-8, decodes GBK, and handles mixed UTF-8/GBK lines", () => {
  assert.equal(decodeStderr(Buffer.from("中文 UTF-8\n", "utf8")), "中文 UTF-8\n");
  assert.equal(decodeStderr(GBK_CMD_TOO_LONG), "命令行太长。\r\n");
  const mixed = Buffer.concat([Buffer.from("正常行\n", "utf8"), GBK_CMD_TOO_LONG]);
  assert.equal(decodeStderr(mixed), "正常行\n命令行太长。\r\n");
});

void test("runPs decodes GBK stderr from Windows cmd.exe into readable Chinese", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-ssh-"));
  try {
    await useFakeSsh(directory, [{ code: 1, stderrBase64: GBK_CMD_TOO_LONG.toString("base64") }]);
    const machine = { ...MACHINE, sshCommand: FAKE_SSH };
    await assert.rejects(runPs(machine, "Write-Host 'hi'"), (error: unknown) => {
      assert.ok(error instanceof CalledProcessError);
      assert.match(error.message, /命令行太长。/u);
      assert.match(error.stderr, /命令行太长。/u);
      return true;
    });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("runPs uploads long scripts as UTF-8 with BOM, runs via -File, and deletes temp files", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-ssh-"));
  try {
    const log = await useFakeSsh(
      directory,
      [{ stdout: "" }, { stdout: "" }, { stdout: "长脚本成功\n" }, { stdout: "" }],
      { captureUploaded: true },
    );
    const machine = { ...MACHINE, sshCommand: FAKE_SSH, scpCommand: FAKE_SSH };
    const script = `# 中文注释\n${"$x = 'a'\n".repeat(400)}Write-Output 'ok'\n`;
    assert.ok(encodedPsCommandLength(script) > MAX_ENCODED_COMMAND_LENGTH);

    const result = await runPs(machine, script, { capture: true });
    assert.equal(result.stdout, "长脚本成功\n");

    const calls = await readSshLog(log);
    assert.equal(calls.length, 4);

    // 1. Ensure <LAPTOP_ROOT>/tmp exists
    const mkdirCall = calls[0];
    assert.ok(mkdirCall);
    const mkdirIdx = mkdirCall.args.indexOf("-EncodedCommand");
    assert.ok(mkdirIdx >= 0);
    const mkdirScript = Buffer.from(mkdirCall.args[mkdirIdx + 1] ?? "", "base64").toString(
      "utf16le",
    );
    assert.match(mkdirScript, /C:\/ffr\/tmp/u);

    // 2. Upload local UTF-8 + BOM file via scp to C:/ffr/tmp/ps-<random>.ps1
    const uploadCall = calls[1];
    assert.ok(uploadCall);
    const localUploaded = uploadCall.args.at(-2) ?? "";
    const remoteDestination = uploadCall.args.at(-1) ?? "";
    const match = /^ffr@192\.0\.2\.10:(C:\/ffr\/tmp\/ps-[0-9a-f]+\.ps1)$/u.exec(remoteDestination);
    assert.ok(match);
    const remoteFile = match[1] ?? "";
    assert.ok(uploadCall.uploadedBase64);
    const uploadedBytes = Buffer.from(uploadCall.uploadedBase64, "base64");
    assert.deepEqual([...uploadedBytes.subarray(0, 3)], [0xef, 0xbb, 0xbf]);
    assert.equal(uploadedBytes.subarray(3).toString("utf8"), PS_PRELUDE + script);
    // Local temp file must be removed in finally.
    await assert.rejects(stat(localUploaded), (error: NodeJS.ErrnoException) => {
      return error.code === "ENOENT";
    });

    // 3. Execute via powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File <path>
    const execCall = calls[2];
    assert.ok(execCall);
    assert.equal(execCall.args.indexOf("-EncodedCommand"), -1);
    const fileIdx = execCall.args.indexOf("-File");
    assert.ok(fileIdx >= 0);
    assert.equal(execCall.args[fileIdx + 1], remoteFile);

    // 4. Delete the remote temporary script in finally
    const cleanupCall = calls[3];
    assert.ok(cleanupCall);
    const cleanupIdx = cleanupCall.args.indexOf("-EncodedCommand");
    assert.ok(cleanupIdx >= 0);
    const cleanupScript = Buffer.from(cleanupCall.args[cleanupIdx + 1] ?? "", "base64").toString(
      "utf16le",
    );
    assert.ok(cleanupScript.includes(remoteFile));
    assert.match(cleanupScript, /Remove-Item/u);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("runPs cleanup failure does not mask the primary error of a long script", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-ssh-"));
  try {
    const log = await useFakeSsh(directory, [
      { stdout: "" },
      { stdout: "" },
      { code: 7, stderr: "主脚本失败\n" },
      { code: 2, stderr: "清理删除失败\n" },
    ]);
    const machine = { ...MACHINE, sshCommand: FAKE_SSH, scpCommand: FAKE_SSH };
    const script = "$x = 'a'\n".repeat(400);
    assert.ok(encodedPsCommandLength(script) > MAX_ENCODED_COMMAND_LENGTH);

    await assert.rejects(runPs(machine, script, { capture: true }), (error: unknown) => {
      assert.ok(error instanceof CalledProcessError);
      assert.equal(error.returnCode, 7);
      assert.match(error.message, /主脚本失败/u);
      assert.doesNotMatch(error.message, /清理删除失败/u);
      return true;
    });
    assert.equal((await readSshLog(log)).length, 4);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

interface FakeResponse {
  stdout?: string;
  stdoutBase64?: string;
  stderr?: string;
  stderrBase64?: string;
  code?: number;
}

async function useFakeSsh(
  directory: string,
  responses: readonly FakeResponse[],
  options: { captureUploaded?: boolean } = {},
): Promise<string> {
  const config = join(directory, "fake-ssh.json");
  const log = join(directory, "ssh.log");
  await writeFile(
    config,
    JSON.stringify({ responses, captureUploaded: options.captureUploaded === true }),
    "utf8",
  );
  await writeFile(log, "", "utf8");
  await chmod(config, 0o600);
  process.env.FAKE_SSH_CONFIG = config;
  process.env.FAKE_SSH_LOG = log;
  return log;
}

async function readSshLog(
  path: string,
): Promise<{ args: string[]; stdin: string; uploadedBase64?: string }[]> {
  const text = await readFile(path, "utf8");
  return text
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line) as { args: string[]; stdin: string; uploadedBase64?: string });
}
