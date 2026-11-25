/**
 * The link to the Windows game machine: SSH with one dedicated key, PowerShell over
 * -EncodedCommand, programs started on the desktop through the logon-only scheduled task, and
 * the few checks the UE4SS command lines need.
 *
 * Every process is spawned asynchronously: the command lines may also run from dsh's event loop,
 * so a blocking SSH call would freeze the harness. All waiting functions take the caller's
 * signal and kill the child when it aborts.
 *
 * machine.env keys such as LAPTOP_HOST, LAPTOP_ROOT and FFR_GAME_DIR are read here; this module
 * contains no dev-box or game-machine absolute paths.
 */

import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdtemp, rm, stat, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import {
  MACHINE_ENV,
  describeError,
  isMissingFile,
  isRecord,
  readEnvFile,
  type Json,
} from "../../shared/index.ts";

/** The task name used when machine.env has no LAPTOP_SESSION_TASK. */
export const SESSION_TASK = "FFR-Agent-Session";

/**
 * Without this, PowerShell writes to the SSH pipe in the OEM code page and Chinese is garbled.
 */
export const PS_PRELUDE =
  "[Console]::OutputEncoding = [Text.Encoding]::UTF8; $OutputEncoding = [Text.Encoding]::UTF8; " +
  "$ProgressPreference = 'SilentlyContinue'; $ErrorActionPreference = 'Stop'\n";

/**
 * Windows OpenSSH invokes cmd.exe, whose command line limit is 8191 characters. Any encoded
 * PowerShell command longer than this threshold is uploaded to <LAPTOP_ROOT>/tmp/ and run via
 * -File instead.
 */
export const MAX_ENCODED_COMMAND_LENGTH = 6000;

const PS_COMMON_ARGS = ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass"] as const;
const UTF8_BOM = Buffer.from([0xef, 0xbb, 0xbf]);
const UTF8_STRICT = new TextDecoder("utf-8", { fatal: true });
const GBK_DECODER = new TextDecoder("gbk");

/** With stderr on a pipe, powershell.exe serialises its error and warning streams as CLIXML. */
const CLIXML_BLOCK = /(<Objs\b.*?<\/Objs>)/s;
const CLIXML_STRING = /<S S="(\w+)">(.*?)<\/S>/gs;
const CLIXML_ESCAPE = /_x([0-9A-Fa-f]{4})_/g;

export class ConfigError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ConfigError";
  }
}

export class CalledProcessError extends Error {
  readonly returnCode: number;
  readonly cmd: readonly string[];
  readonly stdout: string;
  readonly stderr: string;

  /**
   * The message carries the reason (the program's own error text, for PowerShell the decoded
   * CLIXML), not the command line: the user reads it, and the base64 script tells them nothing.
   * `cmd` keeps the command line for callers that need it.
   */
  constructor(returnCode: number, cmd: readonly string[], stdout: string, stderr: string) {
    super(`命令退出码 ${returnCode}:${reasonOf(stderr) || cmd[0] || "?"}`);
    this.name = "CalledProcessError";
    this.returnCode = returnCode;
    this.cmd = cmd;
    this.stdout = stdout;
    this.stderr = stderr;
  }
}

/** PowerShell's location and category lines around an error message. */
const ERROR_NOISE = /^(所在位置|At line|At char|\+ |\s+\+ |CategoryInfo|FullyQualifiedErrorId)/;
const REASON_CHARS = 300;

/** The meaningful lines of a failed program's stderr. */
export function reasonOf(stderr: string): string {
  const lines = stderr
    .split(/\r\n|\r|\n/)
    .map((line) => line.trimEnd())
    .filter((line) => line.trim() !== "" && !ERROR_NOISE.test(line));
  return lines.join(" / ").slice(0, REASON_CHARS);
}

export class TimeoutError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "TimeoutError";
  }
}

export interface Machine {
  readonly host: string;
  readonly user: string;
  /** Path to the dedicated private key, kept as a string. */
  readonly key: string;
  /** The work directory on the game machine: session requests and the UE4SS manifest. */
  readonly root: string;
  /** The game's Win64 directory, where dwmapi.dll and ue4ss/ are installed. */
  readonly gameDir: string;
  /** The Steam AppID used by game-launch; optional for install-only commands. */
  readonly appId?: string;
  /** The scheduled task that runs in the desktop session; LAPTOP_SESSION_TASK. */
  readonly sessionTask?: string;
  /** The account logged on to the desktop, which the session task runs as (DOMAIN\user). */
  readonly desktopUser?: string;
  /** Tests point this at a fake ssh script; production uses the one on PATH. */
  readonly sshCommand?: string;
  /** Tests point this at a fake scp script; production uses the one on PATH. */
  readonly scpCommand?: string;
}

export interface ProcessResult {
  readonly stdout: string;
  readonly stderr: string;
  readonly returnCode: number;
}

interface RunOptions {
  capture?: boolean;
  check?: boolean;
  signal?: AbortSignal;
  env?: NodeJS.ProcessEnv;
}

export interface PsOptions {
  check?: boolean;
  capture?: boolean;
  signal?: AbortSignal;
}

export interface SessionRequest {
  readonly id: string;
  readonly file: string;
  readonly args: readonly string[];
  readonly cwd: string | null;
  readonly wait: boolean;
}

/**
 * Read machine.env. Tests always pass an explicit path (or FFR_MACHINE_ENV) so they never touch
 * the real machine/machine.env; production uses MACHINE_ENV.
 */
export async function loadMachine(
  path: string = process.env.FFR_MACHINE_ENV || MACHINE_ENV,
  signal?: AbortSignal,
): Promise<Machine> {
  try {
    await stat(path);
  } catch (error) {
    signal?.throwIfAborted();
    if (isMissingFile(error)) {
      throw new ConfigError(`缺少 ${path}:请复制 machine/machine.env.example 并填写`);
    }
    throw new ConfigError(`读不了 ${path}:${describeError(error)}`);
  }
  const values = await readEnvFile(path, signal);
  const required = ["LAPTOP_HOST", "LAPTOP_USER", "LAPTOP_ROOT", "FFR_GAME_DIR"];
  const missing = required.filter((name) => !values[name]);
  if (missing.length > 0) {
    throw new ConfigError(`${path} 里缺少 ${missing.join(", ")}`);
  }
  // Missing means absent above; noUncheckedIndexedAccess still needs a check for the compiler.
  const host = values.LAPTOP_HOST;
  const user = values.LAPTOP_USER;
  const root = values.LAPTOP_ROOT;
  const gameDir = values.FFR_GAME_DIR;
  if (!host || !user || !root || !gameDir) {
    throw new ConfigError(`${path} 里缺少 LAPTOP_HOST, LAPTOP_USER, LAPTOP_ROOT, FFR_GAME_DIR`);
  }
  const key = values.LAPTOP_KEY || "~/.ssh/ffr_laptop_ed25519";
  return {
    host,
    user,
    key: expandHome(key),
    root: trimTrailingSlash(root),
    gameDir: trimTrailingSlash(gameDir),
    appId: values.FFR_APP_ID || undefined,
    sessionTask: values.LAPTOP_SESSION_TASK || SESSION_TASK,
    desktopUser: values.LAPTOP_DESKTOP_USER || undefined,
    sshCommand: "ssh",
    scpCommand: "scp",
  };
}

export function target(machine: Machine): string {
  return `${machine.user}@${machine.host}`;
}

/** The configured session task, or the default shown in machine.env.example. */
export function sessionTaskOf(machine: Machine): string {
  return machine.sessionTask || SESSION_TASK;
}

export function ssh(machine: Machine): string[] {
  return [machine.sshCommand || "ssh", ...options(machine), target(machine)];
}

export function scp(machine: Machine): string[] {
  return [machine.scpCommand || "scp", ...options(machine), "-r"];
}

export function psQuote(value: string): string {
  return `'${value.replaceAll("'", "''")}'`;
}

/** Join remote paths with forward slashes; never use the dev box's path separator. */
export function remoteJoin(base: string, ...parts: readonly string[]): string {
  const head = base.replace(/[\\/]+$/u, "");
  const tail = parts.map((part) => part.replace(/^[\\/]+|[\\/]+$/gu, "")).filter(Boolean);
  return [head, ...tail].join("/");
}

/** Base64 of UTF-16LE, the format powershell -EncodedCommand expects. Avoids all quoting. */
export function encodePs(script: string): string {
  return Buffer.from(PS_PRELUDE + script, "utf16le").toString("base64");
}

/** Length of the remote PowerShell command line when sent inline via -EncodedCommand. */
export function encodedPsCommandLength(script: string): number {
  return ["powershell", ...PS_COMMON_ARGS, "-EncodedCommand", encodePs(script)].join(" ").length;
}

/**
 * Decode stderr bytes from Windows SSH/cmd/PowerShell: prefer strict UTF-8, and fall back
 * line by line to GBK (code page 936) so Windows system errors (such as cmd.exe's "命令行太长。")
 * remain readable even when mixed with UTF-8 lines.
 */
export function decodeStderr(raw: Uint8Array): string {
  if (raw.byteLength === 0) return "";
  try {
    return UTF8_STRICT.decode(raw);
  } catch {
    // Mixed or GBK output: decode line by line since 0x0A never appears inside a GBK/UTF-8 sequence.
  }
  const parts: string[] = [];
  let start = 0;
  for (let index = 0; index <= raw.byteLength; index++) {
    if (index === raw.byteLength || raw[index] === 0x0a) {
      const end = index < raw.byteLength ? index + 1 : index;
      const slice = raw.subarray(start, end);
      try {
        parts.push(UTF8_STRICT.decode(slice));
      } catch {
        parts.push(GBK_DECODER.decode(slice));
      }
      start = end;
    }
  }
  return parts.join("");
}

/** Turn PowerShell's CLIXML stderr back into the text it would have printed. */
export function decodeClixml(value: string): string {
  if (!value.startsWith("#< CLIXML")) return value;
  const out: string[] = [];
  for (const part of value
    .slice("#< CLIXML".length)
    .replace(/^[\r\n]+/, "")
    .split(CLIXML_BLOCK)) {
    if (!part.startsWith("<Objs")) {
      out.push(part);
      continue;
    }
    for (const match of part.matchAll(CLIXML_STRING)) {
      const stream = match[1] ?? "";
      const raw = match[2] ?? "";
      const decoded = decodeHtmlEntities(raw).replace(CLIXML_ESCAPE, (_, hex: string) =>
        String.fromCodePoint(Number.parseInt(hex, 16)),
      );
      out.push((stream.toLowerCase() === "warning" ? "警告:" : "") + decoded);
    }
  }
  return out.join("").replaceAll("\r\n", "\n");
}

export function sessionRequest(
  file: string,
  args: readonly string[],
  cwd: string | null,
  wait: boolean,
): SessionRequest {
  return {
    id: randomUUID().replaceAll("-", "").slice(0, 12),
    file,
    args,
    cwd,
    wait,
  };
}

/** Run a PowerShell script on the machine; the SSH child is killed when the signal aborts. */
export async function runPs(
  machine: Machine,
  script: string,
  options: PsOptions = {},
): Promise<ProcessResult> {
  if (encodedPsCommandLength(script) <= MAX_ENCODED_COMMAND_LENGTH) {
    return runEncodedPs(machine, script, options);
  }
  return runLongPs(machine, script, options);
}

async function executePsCommand(
  command: readonly string[],
  options: PsOptions,
  emitStderr = true,
): Promise<ProcessResult> {
  const result = await runProcess(command, {
    capture: options.capture === true,
    signal: options.signal,
  });
  const stderr = decodeClixml(result.stderr);
  if (emitStderr && stderr) process.stderr.write(stderr);
  if (options.check !== false && result.returnCode !== 0) {
    throw new CalledProcessError(result.returnCode, command, result.stdout, stderr);
  }
  return { ...result, stderr };
}

async function runEncodedPs(
  machine: Machine,
  script: string,
  options: PsOptions,
  emitStderr = true,
): Promise<ProcessResult> {
  // The machine's policy may be AllSigned; Bypass for this process lets our unsigned scripts
  // run without changing the policy itself.
  const command = [
    ...ssh(machine),
    "powershell",
    ...PS_COMMON_ARGS,
    "-EncodedCommand",
    encodePs(script),
  ];
  return executePsCommand(command, options, emitStderr);
}

/**
 * Upload a long script as UTF-8 with BOM (required by PowerShell 5.1 for -File) to
 * <LAPTOP_ROOT>/tmp/ps-<random>.ps1, run it with -File, and remove both temporary files in finally.
 */
async function runLongPs(
  machine: Machine,
  script: string,
  options: PsOptions,
): Promise<ProcessResult> {
  const fileName = `ps-${randomUUID().replaceAll("-", "").slice(0, 12)}.ps1`;
  const localDir = await mkdtemp(join(tmpdir(), "ffr-ps-"));
  const localFile = join(localDir, fileName);
  const remoteDir = remoteJoin(machine.root, "tmp");
  const remoteFile = remoteJoin(remoteDir, fileName);
  let uploaded = false;
  try {
    await writeFile(localFile, Buffer.concat([UTF8_BOM, Buffer.from(PS_PRELUDE + script, "utf8")]));
    await runEncodedPs(
      machine,
      `New-Item -ItemType Directory -Force -Path ${psQuote(remoteDir)} | Out-Null\n`,
      { capture: true, check: true, signal: options.signal },
    );
    await runScp(machine, localFile, `${target(machine)}:${remoteFile}`, {
      signal: options.signal,
    });
    uploaded = true;
    const command = [
      ...ssh(machine),
      "powershell",
      ...PS_COMMON_ARGS,
      "-File",
      quoteRemoteCmdArg(remoteFile),
    ];
    return await executePsCommand(command, options);
  } finally {
    await rm(localDir, { recursive: true, force: true }).catch(() => undefined);
    if (uploaded && !options.signal?.aborted) {
      try {
        const cleanup =
          `$path = ${psQuote(remoteFile)}\n` +
          "if (Test-Path -LiteralPath $path -PathType Leaf) { " +
          "Remove-Item -LiteralPath $path -Force }\n";
        await runEncodedPs(machine, cleanup, { capture: true, check: true }, false);
      } catch (cleanupError) {
        // Never mask a primary error (or break a succeeded command on a transient lock).
        process.stderr.write(
          `警告:删除远端临时脚本失败(${remoteFile}):${describeError(cleanupError)}\n`,
        );
      }
    }
  }
}

function quoteRemoteCmdArg(value: string): string {
  if (/[\s"&|<>^()%!]/u.test(value)) {
    return `"${value.replaceAll('"', '\\"')}"`;
  }
  return value;
}

/** Run scp with the dedicated key. */
export async function runScp(
  machine: Machine,
  sources: string | readonly string[],
  destination: string,
  options: Pick<RunOptions, "check" | "signal"> = {},
): Promise<ProcessResult> {
  const list = typeof sources === "string" ? [sources] : [...sources];
  const command = [...scp(machine), ...list, destination];
  return runProcess(command, {
    capture: true,
    check: options.check !== false,
    signal: options.signal,
  });
}

/** Run a program on the machine's desktop through the logon-only scheduled task. */
export async function runInSession(
  machine: Machine,
  request: SessionRequest,
  timeout: number,
  signal?: AbortSignal,
): Promise<Json> {
  const session = remoteJoin(machine.root, "session");
  const requestPath = remoteJoin(session, "request.json");
  const body = JSON.stringify(request);
  await runPs(
    machine,
    `New-Item -ItemType Directory -Force -Path ${psQuote(session)} | Out-Null\n` +
      `[IO.File]::WriteAllText(${psQuote(requestPath)}, ${psQuote(body)}, ` +
      "(New-Object Text.UTF8Encoding $false))\n" +
      // schtasks.exe, not Start-ScheduledTask: CIM is denied to the network logon.
      `schtasks.exe /run /tn ${psQuote(sessionTaskOf(machine))} | Out-Null\n` +
      "if ($LASTEXITCODE) { throw '触发计划任务失败:还没注册,或者没有授权给这个账号' }",
    { signal },
  );
  const statusPath = remoteJoin(session, `${request.id}.status.json`);
  const deadline = performance.now() + timeout * 1000;
  const final = request.wait
    ? ["done", "error", "refused"]
    : ["started", "done", "error", "refused"];
  while (performance.now() < deadline) {
    const out = (
      await runPs(
        machine,
        `if (Test-Path -LiteralPath ${psQuote(statusPath)}) {\n` +
          `  try { Get-Content -LiteralPath ${psQuote(statusPath)} -Raw -Encoding UTF8 } catch { }\n` +
          "}",
        { capture: true, signal },
      )
    ).stdout.trim();
    if (out) {
      const status = parseStatus(out);
      const state = status.state;
      if (typeof state === "string" && final.includes(state)) return status;
    }
    await sleep(2000, signal);
  }
  throw new TimeoutError("等待超时:游戏机上可能没有登录桌面,或者还没注册计划任务");
}

/** The PowerShell lines that decide whether any exe in the game directory is running. */
export function gameRunningScript(machine: Machine): string {
  return (
    `$dir = ${psQuote(machine.gameDir)}\n` +
    `if (-not (Test-Path -LiteralPath $dir -PathType Container)) { throw "游戏目录不存在:$dir" }\n` +
    "$names = @(Get-ChildItem -LiteralPath $dir -Filter '*.exe' -File | ForEach-Object { $_.BaseName })\n" +
    "$running = @()\n" +
    "foreach ($name in $names) { if (Get-Process -Name $name -ErrorAction SilentlyContinue) " +
    "{ $running += $name } }\n" +
    "if ($running.Count -gt 0) { 'RUNNING:' + ($running -join ',') } else { 'STOPPED' }\n"
  );
}

/** The base names of the game's running executables, or [] when the game is closed. */
export async function isGameRunning(machine: Machine, signal?: AbortSignal): Promise<string[]> {
  const result = await runPs(machine, gameRunningScript(machine), { capture: true, signal });
  const line =
    result.stdout
      .split(/\r\n|\r|\n/)
      .map((item) => item.trim())
      .filter(Boolean)
      .at(-1) ?? "";
  if (line === "STOPPED") return [];
  if (line.startsWith("RUNNING:")) {
    return line
      .slice("RUNNING:".length)
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean);
  }
  throw new Error(`读不了游戏是否在运行:游戏机输出 ${line || "(空)"}`);
}

/** IdentitiesOnly: never fall back to keys in a forwarded ssh-agent, only the dedicated key.
 * WarnWeakCrypto=no: Windows OpenSSH 9.5 offers no post-quantum key exchange, and the warning
 * about it would otherwise precede every command. ControlMaster: reuse one connection for a few
 * minutes; one command line sends several commands. */
function options(machine: Machine): string[] {
  return [
    "-i",
    machine.key,
    "-o",
    "IdentitiesOnly=yes",
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=10",
    "-o",
    "WarnWeakCrypto=no",
    "-o",
    "ControlMaster=auto",
    "-o",
    "ControlPersist=300",
    "-o",
    `ControlPath=${join(homedir(), ".ssh", "ffr-cm-%C")}`,
  ];
}

function runProcess(command: readonly string[], options: RunOptions = {}): Promise<ProcessResult> {
  return new Promise<ProcessResult>((resolve, reject) => {
    const [executable, ...args] = command;
    if (!executable) {
      reject(new Error("命令是空的"));
      return;
    }
    const child = spawn(executable, args, {
      stdio: ["ignore", options.capture === true ? "pipe" : "inherit", "pipe"],
      env: options.env,
      signal: options.signal,
    });
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    child.stdout?.on("data", (chunk: Buffer) => stdout.push(chunk));
    child.stderr?.on("data", (chunk: Buffer) => stderr.push(chunk));
    child.on("error", (error) => {
      if (options.signal?.aborted) reject(abortReason(options.signal));
      else
        reject(
          new Error(`启动 ${executable} 失败:${describeError(error)}`, {
            cause: error,
          }),
        );
    });
    child.on("close", (code) => {
      const result: ProcessResult = {
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: decodeStderr(Buffer.concat(stderr)),
        returnCode: code ?? -1,
      };
      if (options.check === true && result.returnCode !== 0) {
        reject(new CalledProcessError(result.returnCode, command, result.stdout, result.stderr));
        return;
      }
      resolve(result);
    });
  });
}

function parseStatus(value: string): Json {
  let parsed: unknown;
  try {
    parsed = JSON.parse(value);
  } catch (error) {
    throw new Error(`计划任务的 status.json 不是合法 JSON:${describeError(error)}`, {
      cause: error,
    });
  }
  if (!isRecord(parsed)) {
    throw new Error("计划任务的 status.json 不是对象");
  }
  return parsed;
}

function expandHome(path: string): string {
  if (path === "~") return homedir();
  if (path.startsWith("~/")) return join(homedir(), path.slice(2));
  return path;
}

function trimTrailingSlash(path: string): string {
  return path.replace(/[\\/]+$/u, "");
}

function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    const onAbort = (): void => {
      clearTimeout(timer);
      reject(abortReason(signal));
    };
    if (signal?.aborted) {
      clearTimeout(timer);
      reject(abortReason(signal));
      return;
    }
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

function abortReason(signal: AbortSignal | undefined): Error {
  const reason: unknown = signal?.reason;
  return reason instanceof Error ? reason : new DOMException("操作已取消", "AbortError");
}

/** The handful of HTML entities CLIXML uses. */
function decodeHtmlEntities(value: string): string {
  return value.replace(/&(#x[0-9A-Fa-f]+|#\d+|[a-zA-Z]+);/g, (match, body: string) => {
    if (body.startsWith("#x")) return String.fromCodePoint(Number.parseInt(body.slice(2), 16));
    if (body.startsWith("#")) return String.fromCodePoint(Number.parseInt(body.slice(1), 10));
    const named: Record<string, string> = {
      amp: "&",
      lt: "<",
      gt: ">",
      quot: '"',
      apos: "'",
      nbsp: "\u00a0",
    };
    return named[body] ?? match;
  });
}
