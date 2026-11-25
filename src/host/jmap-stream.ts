/**
 * Streaming JSON scanner for large UE4SS `.jmap` files (100+ MB).
 *
 * Instead of loading the whole file into memory and calling `JSON.parse` once, this scanner walks
 * UTF-8 chunks with a character-code state machine:
 * - top-level `metadata` is parsed into a compact summary;
 * - top-level `objects` entries are streamed one by one, skipping values whose keys fail
 *   `shouldParseObject` with zero string allocation;
 * - other top-level sections (such as `vtables`) are skipped directly in the stream.
 *
 * It does not depend on indentation or line breaks, so pretty-printed and minified jmap files both
 * parse identically regardless of chunk boundaries.
 */

import { createReadStream } from "node:fs";
import { isRecord, num, text } from "../shared/index.ts";

export interface JmapEngineVersion {
  readonly major: number;
  readonly minor: number;
}

export interface JmapMetadata {
  readonly tool?: string;
  readonly timestamp?: string;
  readonly source?: string;
  readonly engineVersion?: JmapEngineVersion;
  readonly buildChangeList?: string;
}

export interface JmapStreamOptions {
  /** Return true to parse this object's JSON and pass it to `onObject`; false skips it. */
  readonly shouldParseObject?: (path: string) => boolean;
  readonly onObject: (path: string, value: unknown) => void;
}

export interface JmapStreamResult {
  readonly metadata: JmapMetadata | null;
  readonly totalObjects: number;
  readonly parsedObjects: number;
  readonly skippedObjects: number;
}

type State =
  | "start"
  | "top_key_or_end"
  | "top_key"
  | "top_after_key"
  | "top_value_start"
  | "top_after_value"
  | "obj_key_or_end"
  | "obj_key"
  | "obj_after_key"
  | "obj_value_start"
  | "obj_after_value"
  | "key_string"
  | "value_compound"
  | "value_string"
  | "value_primitive"
  | "done";

const CHAR_TAB = 9;
const CHAR_LF = 10;
const CHAR_CR = 13;
const CHAR_SPACE = 32;
const CHAR_QUOTE = 34;
const CHAR_COMMA = 44;
const CHAR_COLON = 58;
const CHAR_LBRACKET = 91;
const CHAR_BACKSLASH = 92;
const CHAR_RBRACKET = 93;
const CHAR_LBRACE = 123;
const CHAR_RBRACE = 125;

function isWhitespace(code: number): boolean {
  return code === CHAR_SPACE || code === CHAR_TAB || code === CHAR_LF || code === CHAR_CR;
}

function parseJsonStringLiteral(raw: string, label: string): string {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new Error(`jmap ${label} 字符串格式非法:${detail}`, { cause: error });
  }
  if (typeof value !== "string") {
    throw new Error(`jmap ${label} 不是字符串`);
  }
  return value;
}

function parseMetadata(raw: string): JmapMetadata {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new Error(`jmap metadata 解析失败:${detail}`, { cause: error });
  }
  if (!isRecord(parsed)) {
    throw new Error("jmap metadata 不是对象");
  }
  const tool = text(parsed.tool);
  const timestamp = text(parsed.timestamp);
  const source = text(parsed.source);
  const buildChangeList = text(parsed.build_change_list);
  let engineVersion: JmapEngineVersion | undefined;
  if (isRecord(parsed.engine_version)) {
    const major = num(parsed.engine_version.major);
    const minor = num(parsed.engine_version.minor);
    if (major !== undefined && minor !== undefined) {
      engineVersion = { major, minor };
    }
  }
  return {
    ...(tool !== undefined ? { tool } : {}),
    ...(timestamp !== undefined ? { timestamp } : {}),
    ...(source !== undefined ? { source } : {}),
    ...(engineVersion !== undefined ? { engineVersion } : {}),
    ...(buildChangeList !== undefined ? { buildChangeList } : {}),
  };
}

function isValueState(state: State): boolean {
  return state === "value_compound" || state === "value_string" || state === "value_primitive";
}

function isContinuingCapture(state: State, valueCapture: boolean): boolean {
  return state === "key_string" || (valueCapture && isValueState(state));
}

function assertCompleted(state: State): void {
  if (state !== "done") {
    throw new Error(`jmap 文件不完整或提前结束(状态:${state})`);
  }
}

/** Stream a jmap document from an async iterable of UTF-8 text chunks. */
export async function streamJmap(
  chunks: AsyncIterable<string>,
  options: JmapStreamOptions,
): Promise<JmapStreamResult> {
  const shouldParseObject = options.shouldParseObject ?? (() => true);

  let state: State = "start";
  let keyReturnState: "top_after_key" | "obj_after_key" = "top_after_key";
  let valueReturnState: "top_after_value" | "obj_after_value" = "top_after_value";
  let valueTarget: "skip" | "metadata" | "object" = "skip";

  let currentTopKey = "";
  let currentObjectKey = "";

  let stringEscaped = false;
  let stringParts: string[] = [];

  let valueCapture = false;
  let valueDepth = 0;
  let valueInString = false;
  let valueEscaped = false;
  let valueParts: string[] = [];

  let sawObjects = false;
  let metadata: JmapMetadata | null = null;
  let totalObjects = 0;
  let parsedObjects = 0;
  let skippedObjects = 0;

  function finishValue(raw: string | null): State {
    if (valueTarget === "metadata") {
      if (raw === null) throw new Error("内部错误:缺少 metadata 文本");
      metadata = parseMetadata(raw);
    } else if (valueTarget === "object") {
      totalObjects += 1;
      if (raw !== null) {
        let parsed: unknown;
        try {
          parsed = JSON.parse(raw);
        } catch (error) {
          const detail = error instanceof Error ? error.message : String(error);
          throw new Error(`jmap 对象 ${currentObjectKey} 解析失败:${detail}`, { cause: error });
        }
        parsedObjects += 1;
        options.onObject(currentObjectKey, parsed);
      } else {
        skippedObjects += 1;
      }
    }
    valueParts = [];
    valueCapture = false;
    valueTarget = "skip";
    return valueReturnState;
  }

  for await (const chunk of chunks) {
    const length = chunk.length;
    let i = 0;
    let captureStart = isContinuingCapture(state, valueCapture) ? 0 : -1;

    while (i < length) {
      const code = chunk.charCodeAt(i);

      switch (state) {
        case "start": {
          if (isWhitespace(code)) {
            i += 1;
            continue;
          }
          if (code !== CHAR_LBRACE) {
            throw new Error("jmap 顶层不是 JSON 对象");
          }
          state = "top_key_or_end";
          i += 1;
          break;
        }

        case "top_key_or_end": {
          if (isWhitespace(code)) {
            i += 1;
            continue;
          }
          if (code === CHAR_RBRACE) {
            state = "done";
            i += 1;
            break;
          }
          if (code !== CHAR_QUOTE) {
            throw new Error(`jmap 顶层键名位置出现意外字符:${chunk[i] ?? ""}`);
          }
          keyReturnState = "top_after_key";
          stringEscaped = false;
          stringParts = [];
          captureStart = i;
          state = "key_string";
          i += 1;
          break;
        }

        case "top_key": {
          if (isWhitespace(code)) {
            i += 1;
            continue;
          }
          if (code !== CHAR_QUOTE) {
            throw new Error(`jmap 顶层键名位置出现意外字符:${chunk[i] ?? ""}`);
          }
          keyReturnState = "top_after_key";
          stringEscaped = false;
          stringParts = [];
          captureStart = i;
          state = "key_string";
          i += 1;
          break;
        }

        case "top_after_key": {
          if (isWhitespace(code)) {
            i += 1;
            continue;
          }
          if (code !== CHAR_COLON) {
            throw new Error(`jmap 顶层键 ${currentTopKey} 后缺少冒号`);
          }
          state = "top_value_start";
          i += 1;
          break;
        }

        case "top_value_start": {
          if (isWhitespace(code)) {
            i += 1;
            continue;
          }
          if (currentTopKey === "objects") {
            if (code !== CHAR_LBRACE) {
              throw new Error("jmap 的 objects 字段不是对象");
            }
            sawObjects = true;
            state = "obj_key_or_end";
            i += 1;
            break;
          }
          valueReturnState = "top_after_value";
          if (currentTopKey === "metadata") {
            valueTarget = "metadata";
            valueCapture = true;
          } else {
            valueTarget = "skip";
            valueCapture = false;
          }
          valueParts = [];
          if (code === CHAR_LBRACE || code === CHAR_LBRACKET) {
            valueDepth = 1;
            valueInString = false;
            valueEscaped = false;
            captureStart = valueCapture ? i : -1;
            state = "value_compound";
            i += 1;
          } else if (code === CHAR_QUOTE) {
            valueEscaped = false;
            captureStart = valueCapture ? i : -1;
            state = "value_string";
            i += 1;
          } else {
            captureStart = valueCapture ? i : -1;
            state = "value_primitive";
            i += 1;
          }
          break;
        }

        case "top_after_value": {
          if (isWhitespace(code)) {
            i += 1;
            continue;
          }
          if (code === CHAR_COMMA) {
            state = "top_key";
            i += 1;
            break;
          }
          if (code === CHAR_RBRACE) {
            state = "done";
            i += 1;
            break;
          }
          throw new Error(`jmap 顶层值后出现意外字符:${chunk[i] ?? ""}`);
        }

        case "obj_key_or_end": {
          if (isWhitespace(code)) {
            i += 1;
            continue;
          }
          if (code === CHAR_RBRACE) {
            state = "top_after_value";
            i += 1;
            break;
          }
          if (code !== CHAR_QUOTE) {
            throw new Error(`jmap objects 键名位置出现意外字符:${chunk[i] ?? ""}`);
          }
          keyReturnState = "obj_after_key";
          stringEscaped = false;
          stringParts = [];
          captureStart = i;
          state = "key_string";
          i += 1;
          break;
        }

        case "obj_key": {
          if (isWhitespace(code)) {
            i += 1;
            continue;
          }
          if (code !== CHAR_QUOTE) {
            throw new Error(`jmap objects 键名位置出现意外字符:${chunk[i] ?? ""}`);
          }
          keyReturnState = "obj_after_key";
          stringEscaped = false;
          stringParts = [];
          captureStart = i;
          state = "key_string";
          i += 1;
          break;
        }

        case "obj_after_key": {
          if (isWhitespace(code)) {
            i += 1;
            continue;
          }
          if (code !== CHAR_COLON) {
            throw new Error(`jmap 对象 ${currentObjectKey} 后缺少冒号`);
          }
          state = "obj_value_start";
          i += 1;
          break;
        }

        case "obj_value_start": {
          if (isWhitespace(code)) {
            i += 1;
            continue;
          }
          valueReturnState = "obj_after_value";
          valueTarget = "object";
          valueCapture = shouldParseObject(currentObjectKey);
          valueParts = [];
          if (code === CHAR_LBRACE || code === CHAR_LBRACKET) {
            valueDepth = 1;
            valueInString = false;
            valueEscaped = false;
            captureStart = valueCapture ? i : -1;
            state = "value_compound";
            i += 1;
          } else if (code === CHAR_QUOTE) {
            valueEscaped = false;
            captureStart = valueCapture ? i : -1;
            state = "value_string";
            i += 1;
          } else {
            captureStart = valueCapture ? i : -1;
            state = "value_primitive";
            i += 1;
          }
          break;
        }

        case "obj_after_value": {
          if (isWhitespace(code)) {
            i += 1;
            continue;
          }
          if (code === CHAR_COMMA) {
            state = "obj_key";
            i += 1;
            break;
          }
          if (code === CHAR_RBRACE) {
            state = "top_after_value";
            i += 1;
            break;
          }
          throw new Error(`jmap 对象 ${currentObjectKey} 值后出现意外字符:${chunk[i] ?? ""}`);
        }

        case "key_string": {
          if (stringEscaped) {
            stringEscaped = false;
            i += 1;
            continue;
          }
          if (code === CHAR_BACKSLASH) {
            stringEscaped = true;
            i += 1;
            continue;
          }
          if (code === CHAR_QUOTE) {
            stringParts.push(chunk.slice(captureStart, i + 1));
            captureStart = -1;
            const decoded = parseJsonStringLiteral(
              stringParts.join(""),
              keyReturnState === "top_after_key" ? "顶层键" : "对象路径",
            );
            stringParts = [];
            if (keyReturnState === "top_after_key") currentTopKey = decoded;
            else currentObjectKey = decoded;
            state = keyReturnState;
            i += 1;
            continue;
          }
          i += 1;
          break;
        }

        case "value_compound": {
          // Tight inner loop over the current chunk to avoid switch overhead on 100 MB files.
          let doneCompound = false;
          while (i < length) {
            const c = chunk.charCodeAt(i);
            if (valueInString) {
              if (valueEscaped) {
                valueEscaped = false;
              } else if (c === CHAR_BACKSLASH) {
                valueEscaped = true;
              } else if (c === CHAR_QUOTE) {
                valueInString = false;
              }
            } else if (c === CHAR_QUOTE) {
              valueInString = true;
            } else if (c === CHAR_LBRACE || c === CHAR_LBRACKET) {
              valueDepth += 1;
            } else if (c === CHAR_RBRACE || c === CHAR_RBRACKET) {
              valueDepth -= 1;
              if (valueDepth === 0) {
                if (valueCapture) {
                  valueParts.push(chunk.slice(captureStart, i + 1));
                  captureStart = -1;
                }
                i += 1;
                state = finishValue(valueCapture ? valueParts.join("") : null);
                doneCompound = true;
                break;
              }
            }
            i += 1;
          }
          if (doneCompound) break;
          break;
        }

        case "value_string": {
          while (i < length) {
            const c = chunk.charCodeAt(i);
            if (valueEscaped) {
              valueEscaped = false;
            } else if (c === CHAR_BACKSLASH) {
              valueEscaped = true;
            } else if (c === CHAR_QUOTE) {
              if (valueCapture) {
                valueParts.push(chunk.slice(captureStart, i + 1));
                captureStart = -1;
              }
              i += 1;
              state = finishValue(valueCapture ? valueParts.join("") : null);
              break;
            }
            i += 1;
          }
          break;
        }

        case "value_primitive": {
          while (i < length) {
            const c = chunk.charCodeAt(i);
            if (isWhitespace(c) || c === CHAR_COMMA || c === CHAR_RBRACE || c === CHAR_RBRACKET) {
              if (valueCapture) {
                valueParts.push(chunk.slice(captureStart, i));
                captureStart = -1;
              }
              state = finishValue(valueCapture ? valueParts.join("") : null);
              break;
            }
            i += 1;
          }
          break;
        }

        case "done": {
          if (isWhitespace(code)) {
            i += 1;
            continue;
          }
          throw new Error(`jmap 根对象结束后还有多余内容:${chunk[i] ?? ""}`);
        }
      }
    }

    if (state === "key_string" && captureStart >= 0) {
      stringParts.push(chunk.slice(captureStart));
    } else if (valueCapture && captureStart >= 0 && isValueState(state)) {
      valueParts.push(chunk.slice(captureStart));
    }
  }

  assertCompleted(state);
  if (!sawObjects) {
    throw new Error("jmap 缺少顶层 objects 字段");
  }

  return { metadata, totalObjects, parsedObjects, skippedObjects };
}

/** Stream a `.jmap` file from disk in 256 KB UTF-8 chunks. */
export async function streamJmapFile(
  filePath: string,
  options: JmapStreamOptions,
): Promise<JmapStreamResult> {
  const stream = createReadStream(filePath, {
    encoding: "utf8",
    highWaterMark: 256 * 1024,
  });
  return streamJmap(stream as AsyncIterable<string>, options);
}
