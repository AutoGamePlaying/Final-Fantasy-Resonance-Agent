/**
 * Static safety checks for generated remote PowerShell 5.1 scripts.
 *
 * Catches the two live bugs found on 11-24 across every exported script builder:
 * 1. PowerShell 5.1 Get-Content -Raw attaches PSPath/etc. note properties so ConvertTo-Json
 *    serializes the string as an object instead of a string.
 * 2. Nested unescaped double quotes inside `throw "..."` end the string early and cause a
 *    ParserError (such as `[FfrDump]` being parsed as an array index expression).
 */

import assert from "node:assert/strict";
import test from "node:test";
import {
  MAX_ENCODED_COMMAND_LENGTH,
  encodedPsCommandLength,
  gameRunningScript,
  type Machine,
} from "../../../src/io/live/machine.ts";
import { deleteScript, stateScript, writeModsScript } from "../../../src/host/cli/mod-sync.ts";
import { prepareScript, triggerScript, waitScript } from "../../../src/host/cli/dump-run.ts";
import { deleteRemoteScript, fetchScript } from "../../../src/host/cli/dump-fetch.ts";
import { statusScript } from "../../../src/host/cli/dump-status.ts";
import {
  deleteRemoteHotkeyScript,
  fetchHotkeyScript,
} from "../../../src/host/cli/dump-fetch-hotkey.ts";
import { hotkeyDir, remoteHotkeyPackage, triggerText } from "../../../src/host/cli/dump.ts";

const MACHINE: Machine = {
  host: "192.0.2.10",
  user: "ffr",
  key: "/tmp/id_ed25519",
  root: "C:/ffr",
  gameDir: "C:/game/Win64",
  appId: "12345",
};

const ID = "20261124-031500-title-snapshot";

/**
 * Validate every `throw "..."` / `throw '...'` statement and basic quote/brace balance.
 * Returns a list of human-readable issues (empty when clean).
 */
function findPowerShellIssues(script: string): string[] {
  const issues: string[] = [];
  const lines = script.split(/\r?\n/u);

  for (let lineIndex = 0; lineIndex < lines.length; lineIndex++) {
    const line = lines[lineIndex] ?? "";
    const lineNo = lineIndex + 1;

    // Inspect every `throw` statement on this line (may appear inside `{ throw "..." }`).
    const throwPattern = /\bthrow\s+(["'])/gu;
    for (const match of line.matchAll(throwPattern)) {
      const quote = match[1];
      if (!quote) continue;
      const quoteStart = match.index + match[0].length - 1;
      let cursor = quoteStart + 1;
      let closed = false;
      while (cursor < line.length) {
        const ch = line[cursor];
        if (quote === '"') {
          if (ch === "`") {
            cursor += 2;
            continue;
          }
          if (ch === '"') {
            closed = true;
            cursor++;
            break;
          }
        } else {
          if (ch === "'") {
            if (line[cursor + 1] === "'") {
              cursor += 2;
              continue;
            }
            closed = true;
            cursor++;
            break;
          }
        }
        cursor++;
      }
      if (!closed) {
        issues.push(`第 ${lineNo} 行 throw 字符串没有闭合: ${line.trim()}`);
        continue;
      }
      // After the closing quote of `throw "..."`, only whitespace, `}`, `;` or `# comment` may follow.
      // An unescaped inner quote (e.g. `throw "a "[FfrDump]" b"`) leaves stray tokens here.
      const trailing = line.slice(cursor).trim();
      if (trailing !== "" && !/^[};]+(?:\s*#.*)?$/u.test(trailing) && !trailing.startsWith("#")) {
        issues.push(
          `第 ${lineNo} 行 throw 字符串闭合后仍有未转义内容(${trailing}): ${line.trim()}`,
        );
      }
    }
  }

  // Track single/double quotes and `{}` / `()` balance across the full script.
  let inSingle = false;
  let inDouble = false;
  let inComment = false;
  let braceDepth = 0;
  let parenDepth = 0;
  for (let index = 0; index < script.length; index++) {
    const ch = script[index];
    if (inComment) {
      if (ch === "\n") inComment = false;
      continue;
    }
    if (inSingle) {
      if (ch === "'") {
        if (script[index + 1] === "'") index++;
        else inSingle = false;
      }
      continue;
    }
    if (inDouble) {
      if (ch === "`") {
        index++;
        continue;
      }
      if (ch === '"') inDouble = false;
      continue;
    }
    if (ch === "#") {
      inComment = true;
    } else if (ch === "'") {
      inSingle = true;
    } else if (ch === '"') {
      inDouble = true;
    } else if (ch === "{") {
      braceDepth++;
    } else if (ch === "}") {
      braceDepth--;
      if (braceDepth < 0) issues.push("多余的右大括号 }");
    } else if (ch === "(") {
      parenDepth++;
    } else if (ch === ")") {
      parenDepth--;
      if (parenDepth < 0) issues.push("多余的右括号 )");
    }
  }
  if (inSingle) issues.push("存在未闭合的单引号字符串");
  if (inDouble) issues.push("存在未闭合的双引号字符串");
  if (braceDepth !== 0) issues.push(`大括号不配对(差值 ${braceDepth})`);
  if (parenDepth !== 0) issues.push(`圆括号不配对(差值 ${parenDepth})`);

  // PowerShell 5.1 hazard: `$var = Get-Content ... -Raw` (without `| ConvertFrom-Json`) passed
  // into `ConvertTo-Json` serializes PSPath note properties as an object instead of a string.
  const rawContentVars = new Set<string>();
  for (const line of lines) {
    const assign = /^\s*(\$[A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.+)$/u.exec(line);
    if (!assign) continue;
    const variable = assign[1];
    const rhs = assign[2];
    if (!variable || !rhs) continue;
    if (
      /\bGet-Content\b/iu.test(rhs) &&
      /-Raw\b/iu.test(rhs) &&
      !/\|\s*ConvertFrom-Json\b/iu.test(rhs)
    ) {
      rawContentVars.add(variable);
    }
  }
  if (rawContentVars.size > 0 && /\bConvertTo-Json\b/iu.test(script)) {
    for (const line of lines) {
      if (!/\bConvertTo-Json\b/iu.test(line)) continue;
      for (const variable of rawContentVars) {
        const escaped = variable.replace("$", "\\$");
        if (new RegExp(`${escaped}\\b`, "u").test(line)) {
          issues.push(`ConvertTo-Json 直接序列化了 Get-Content -Raw 的变量 ${variable}`);
        }
      }
    }
  }

  return issues;
}

void test("PowerShell checker catches the two 11-24 live bugs", () => {
  const nestedThrow =
    'throw "模组没有取走触发文件:用 ue4ss-log --tag "[FfrDump]" 看模组加载日志"\n';
  const nestedIssues = findPowerShellIssues(nestedThrow);
  assert.ok(nestedIssues.length > 0, "should flag nested double quotes in throw");

  const ps51RawJson =
    "$value = Get-Content -LiteralPath $path -Raw -Encoding UTF8\n" +
    "ConvertTo-Json @{ mods = $value } -Compress\n";
  const rawIssues = findPowerShellIssues(ps51RawJson);
  assert.ok(rawIssues.length > 0, "should flag Get-Content -Raw variable passed to ConvertTo-Json");
});

void test("mod-sync stateScript uses ReadAllText and never Get-Content -Raw", () => {
  const script = stateScript(MACHINE);
  assert.match(script, /\[IO\.File\]::ReadAllText/u);
  assert.doesNotMatch(script, /Get-Content\b[^\n]*-Raw\b/iu);
});

void test("dump-run waitScript uses single quotes inside throw and has no nested double quotes", () => {
  const script = waitScript(MACHINE, ID, 180);
  assert.match(script, /ue4ss-log --tag '\[FfrDump\]'/u);
  assert.deepEqual(findPowerShellIssues(script), []);
});

void test("dump-fetch fetchScript avoids Get-Content -Raw for JSON fields and uses the long-script path", () => {
  const script = fetchScript(MACHINE, ID);
  assert.match(script, /\$doneText\s*=\s*\[IO\.File\]::ReadAllText\(/u);
  assert.match(script, /\$acfText\s*=\s*\[IO\.File\]::ReadAllText\(/u);
  assert.ok(encodedPsCommandLength(script) > MAX_ENCODED_COMMAND_LENGTH);
  assert.deepEqual(findPowerShellIssues(script), []);
});

void test("dump-fetch-hotkey fetchHotkeyScript avoids Get-Content -Raw and uses the long-script path", () => {
  const pkg = remoteHotkeyPackage(MACHINE, "test123");
  const list = "C:/ffr/ffr-dumps/hotkey-test123.files.txt";
  const script = fetchHotkeyScript(MACHINE, ["hotkey-0001-20261125-023000"], 120, pkg, list);
  assert.match(script, /\$doneText\s*=\s*\[IO\.File\]::ReadAllText\(/u);
  assert.match(script, /\$acfText\s*=\s*\[IO\.File\]::ReadAllText\(/u);
  assert.doesNotMatch(script, /Get-Content\b[^\n]*-Raw\b/iu);
  assert.ok(encodedPsCommandLength(script) > MAX_ENCODED_COMMAND_LENGTH);
  assert.deepEqual(findPowerShellIssues(script), []);
});

void test("all exported remote PowerShell script generators pass static safety checks", () => {
  const hotkeyPkg = remoteHotkeyPackage(MACHINE, "abc");
  const scripts: Record<string, string> = {
    "machine.gameRunningScript": gameRunningScript(MACHINE),
    "mod-sync.stateScript": stateScript(MACHINE),
    "mod-sync.deleteScript": deleteScript(MACHINE, "FfrDump"),
    "mod-sync.writeModsScript": writeModsScript(MACHINE, "FfrDump : 1\r\nOther : 1\r\n", [
      hotkeyDir(MACHINE),
    ]),
    "dump-run.prepareScript": prepareScript(MACHINE, ID),
    "dump-run.triggerScript": triggerScript(MACHINE, triggerText(ID, "snapshot", "title")),
    "dump-run.waitScript": waitScript(MACHINE, ID, 180),
    "dump-status.statusScript": statusScript(MACHINE),
    "dump-fetch.fetchScript": fetchScript(MACHINE, ID),
    "dump-fetch.deleteRemoteScript": deleteRemoteScript(MACHINE, ID),
    "dump-fetch-hotkey.fetchHotkeyScript": fetchHotkeyScript(
      MACHINE,
      ["hotkey-0001-20261125-023000"],
      0,
      hotkeyPkg,
      "C:/ffr/ffr-dumps/hotkey-abc.files.txt",
    ),
    "dump-fetch-hotkey.deleteRemoteHotkeyScript": deleteRemoteHotkeyScript(hotkeyPkg),
  };
  for (const [name, script] of Object.entries(scripts)) {
    assert.deepEqual(
      findPowerShellIssues(script),
      [],
      `${name} has PowerShell syntax/safety issues`,
    );
  }
});
