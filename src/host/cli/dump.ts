/**
 * The R1 dump protocol shared by dump-run, dump-fetch and dump-status (docs/r1-dump.md):
 * validating a job id and a scene label, rendering the trigger file, reading the module's
 * heartbeat and done marker, and where the files live on both machines.
 *
 * Everything here is pure or depends only on the machine settings; the command lines do the IO.
 */

import { join } from "node:path";
import type { Machine } from "../../io/live/machine.ts";
import { remoteJoin } from "../../io/live/machine.ts";
import { isRecord, r1Dir, type Json } from "../../shared/index.ts";

/** The six jobs the FfrDump module understands, with their default wait in seconds. */
export const JOBS = {
  snapshot: 180,
  objects: 300,
  usmap: 180,
  jmap: 600,
  sdk: 900,
  uht: 900,
} as const satisfies Record<string, number>;

export type Job = keyof typeof JOBS;

export const JOB_NAMES = Object.keys(JOBS) as Job[];

export function isJob(value: unknown): value is Job {
  return typeof value === "string" && Object.hasOwn(JOBS, value);
}

/** The default wait for a job; callers may override it with --timeout. */
export function defaultTimeout(job: Job): number {
  return JOBS[job];
}

const LABEL = /^[a-z0-9-]{1,32}$/u;
const JOB_ID = /^\d{8}-\d{6}-[a-z0-9-]{1,32}-(snapshot|objects|usmap|jmap|sdk|uht)$/u;
const HOTKEY_SNAPSHOT_ID = /^hotkey-\d{4,}-\d{8}-\d{6}$/u;
const HOTKEY_SNAPSHOT_FILE =
  /^(hotkey-\d{4,}-\d{8}-\d{6})-(context\.json|classes\.json|objects\.jsonl|widgets\.jsonl|texts\.jsonl|done\.json)$/u;

export function isHotkeySnapshotId(value: string): boolean {
  return HOTKEY_SNAPSHOT_ID.test(value);
}

export function parseHotkeySnapshotFile(name: string): { id: string; file: string } | null {
  const match = HOTKEY_SNAPSHOT_FILE.exec(name);
  if (!match?.[1] || !match[2]) return null;
  return { id: match[1], file: match[2] };
}

export function parseLabel(value: string | undefined): string {
  if (value === undefined || value === "") {
    throw new Error("缺少场面名:用 --label <场面名>,例如 field-1");
  }
  if (!LABEL.test(value)) {
    throw new Error(`场面名只能是小写字母、数字和连字符,1 到 32 个字符,收到 ${value}`);
  }
  return value;
}

/** Build `<UTC 日期>-<UTC 时间>-<label>-<job>` from one instant. */
export function jobId(now: Date, label: string, job: Job): string {
  const iso = now.toISOString();
  const date = iso.slice(0, 10).replaceAll("-", "");
  const time = iso.slice(11, 19).replaceAll(":", "");
  return `${date}-${time}-${label}-${job}`;
}

export function parseJobId(value: string | undefined): string {
  if (value === undefined || value === "") {
    throw new Error("缺少任务号:例如 20261124-031500-field-1-snapshot");
  }
  if (!JOB_ID.test(value)) {
    throw new Error(`任务号不合法:${value};格式是 <UTC 日期>-<UTC 时间>-<场面名>-<任务>`);
  }
  return value;
}

/** The three-line trigger file. The module accepts CRLF or LF; we always write CRLF. */
export function triggerText(id: string, job: Job, label: string): string {
  return `id=${id}\r\njob=${job}\r\nlabel=${label}\r\n`;
}

export interface Heartbeat {
  readonly mod: string;
  readonly version: number;
  readonly time: number;
  readonly busy: boolean;
}

export interface Done {
  readonly id: string;
  readonly job: Job;
  readonly label: string;
  readonly ok: boolean;
  readonly error: string | null;
  readonly startedAt: number;
  readonly finishedAt: number;
  readonly files: readonly string[];
  readonly stats: Json;
}

function field(value: Json, name: string, document: string): unknown {
  if (!Object.hasOwn(value, name)) {
    throw new Error(`${document} 缺少字段 ${name}`);
  }
  return value[name];
}

function stringField(value: Json, name: string, document: string): string {
  const item = field(value, name, document);
  if (typeof item !== "string") throw new Error(`${document} 的 ${name} 字段不是字符串`);
  return item;
}

function nullableStringField(value: Json, name: string, document: string): string | null {
  const item = field(value, name, document);
  if (item === null) return null;
  if (typeof item !== "string") throw new Error(`${document} 的 ${name} 字段不是字符串或 null`);
  return item;
}

function numberField(value: Json, name: string, document: string): number {
  const item = field(value, name, document);
  if (typeof item !== "number" || !Number.isFinite(item)) {
    throw new Error(`${document} 的 ${name} 字段不是数字`);
  }
  return item;
}

function booleanField(value: Json, name: string, document: string): boolean {
  const item = field(value, name, document);
  if (typeof item !== "boolean") throw new Error(`${document} 的 ${name} 字段不是布尔值`);
  return item;
}

export function parseHeartbeat(input: unknown): Heartbeat {
  if (!isRecord(input)) throw new Error("heartbeat.json 不是对象");
  return {
    mod: stringField(input, "mod", "heartbeat.json"),
    version: numberField(input, "version", "heartbeat.json"),
    time: numberField(input, "time", "heartbeat.json"),
    busy: booleanField(input, "busy", "heartbeat.json"),
  };
}

export function parseDone(input: unknown): Done {
  if (!isRecord(input)) throw new Error("done.json 不是对象");
  const job = field(input, "job", "done.json");
  if (!isJob(job)) throw new Error(`done.json 的 job 字段不是已知任务:${String(job)}`);
  const files = field(input, "files", "done.json");
  if (!Array.isArray(files)) throw new Error("done.json 的 files 字段不是数组");
  const stats = field(input, "stats", "done.json");
  if (!isRecord(stats)) throw new Error("done.json 的 stats 字段不是对象");
  return {
    id: stringField(input, "id", "done.json"),
    job,
    label: stringField(input, "label", "done.json"),
    ok: booleanField(input, "ok", "done.json"),
    error: nullableStringField(input, "error", "done.json"),
    startedAt: numberField(input, "startedAt", "done.json"),
    finishedAt: numberField(input, "finishedAt", "done.json"),
    files: files.map((item, index) => {
      if (typeof item !== "string") {
        throw new Error(`done.json 的 files[${index}] 不是字符串`);
      }
      return item;
    }),
    stats,
  };
}

/** Steam writes `"buildid" "12345678"` inside appmanifest_<appid>.acf. */
export function buildIdFromAcf(content: string): string | null {
  const match = /"buildid"\s+"(\d+)"/iu.exec(content);
  return match?.[1] ?? null;
}

/** OUT: the module's output root, outside ue4ss/Mods so mod-sync cannot replace it. */
export function outDir(machine: Machine): string {
  return remoteJoin(machine.gameDir, "ue4ss", "ffr-out");
}

export function jobDir(machine: Machine, id: string): string {
  return remoteJoin(outDir(machine), "jobs", id);
}

export function hotkeyDir(machine: Machine): string {
  return remoteJoin(outDir(machine), "hotkey");
}

export function hotkeyMarkersPath(machine: Machine): string {
  return remoteJoin(hotkeyDir(machine), "markers.jsonl");
}

export function triggerPath(machine: Machine): string {
  return remoteJoin(outDir(machine), "trigger.txt");
}

export function heartbeatPath(machine: Machine): string {
  return remoteJoin(outDir(machine), "heartbeat.json");
}

/** The single .tgz left on the game machine until dump-fetch's last step. */
export function remotePackage(machine: Machine, id: string): string {
  return remoteJoin(machine.root, "ffr-dumps", `${id}.tgz`);
}

/** The temporary .tgz left on the game machine until dump-fetch-hotkey's last step. */
export function remoteHotkeyPackage(machine: Machine, tag: string): string {
  return remoteJoin(machine.root, "ffr-dumps", `hotkey-${tag}.tgz`);
}

/** The local directory that holds one fetched dump. */
export function localJobDir(id: string): string {
  return join(r1Dir(), id);
}

/** The local root directory for fetched hotkey snapshots, markers and incremental state. */
export function localHotkeyDir(): string {
  return join(r1Dir(), "hotkey");
}

export function localHotkeySnapshotDir(id: string): string {
  return join(localHotkeyDir(), id);
}

export function localHotkeyMarkersPath(): string {
  return join(localHotkeyDir(), "markers.jsonl");
}

export function localHotkeyStatePath(): string {
  return join(localHotkeyDir(), "fetched.json");
}

/** Parse the JSON the machine sent back, with a Chinese error naming the document. */
export function parseJsonDocument(stdout: string, document: string): unknown {
  try {
    return JSON.parse(stdout.trim());
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    throw new Error(`${document} 不是合法 JSON:${reason}`, { cause: error });
  }
}
