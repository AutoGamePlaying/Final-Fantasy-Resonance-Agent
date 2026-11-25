/**
 * Unit and CLI tests for the streaming jmap survey tool, driven by a small hand-authored fixture
 * in `test/fixtures/sample.jmap`. Nothing here touches `.deps/` or real game dumps.
 */

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { streamJmap } from "../../src/host/jmap-stream.ts";
import {
  classifyTopics,
  compileFilter,
  formatPropertyType,
  renderSurveyMarkdown,
  surveyJmapFile,
  surveyJmapStream,
  type JmapSurveyReport,
} from "../../src/host/jmap-survey.ts";

const FIXTURE = join(import.meta.dirname, "..", "fixtures", "sample.jmap");
const CLI = join(import.meta.dirname, "..", "..", "src", "host", "cli", "jmap-survey.ts");

function chunkString(text: string, size: number): AsyncIterable<string> {
  return {
    [Symbol.asyncIterator](): AsyncIterator<string> {
      let offset = 0;
      return {
        next(): Promise<IteratorResult<string>> {
          if (offset >= text.length) return Promise.resolve({ done: true, value: undefined });
          const value = text.slice(offset, offset + size);
          offset += size;
          return Promise.resolve({ done: false, value });
        },
      };
    },
  };
}

async function runCliCommand(
  args: readonly string[],
  env: NodeJS.ProcessEnv = process.env,
): Promise<{ code: number; stdout: string; stderr: string }> {
  const child = spawn(process.execPath, [CLI, ...args], {
    env,
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

void test("streamJmap parses across tiny chunks and skips unrequested objects", async () => {
  const raw = await readFile(FIXTURE, "utf8");
  const minified = JSON.stringify(JSON.parse(raw));
  const seenPretty: string[] = [];
  const seenMinified: string[] = [];

  const prettyResult = await streamJmap(chunkString(raw, 17), {
    shouldParseObject: (path) => path.startsWith("/Script/mucha."),
    onObject: (path) => {
      seenPretty.push(path);
    },
  });
  const minifiedResult = await streamJmap(chunkString(minified, 13), {
    shouldParseObject: (path) => path.startsWith("/Script/mucha."),
    onObject: (path) => {
      seenMinified.push(path);
    },
  });

  assert.deepEqual(seenPretty, seenMinified);
  assert.equal(prettyResult.totalObjects, 19);
  assert.equal(prettyResult.parsedObjects, seenPretty.length);
  assert.equal(prettyResult.skippedObjects, 19 - seenPretty.length);
  assert.deepEqual(prettyResult.metadata, {
    tool: "SampleJMapFixture",
    timestamp: "2026-01-01T00:00:00Z",
    source: "SampleGame-Win64-Shipping.exe",
    engineVersion: { major: 5, minor: 6 },
    buildChangeList: "0",
  });
  assert.deepEqual(minifiedResult.metadata, prettyResult.metadata);
});

void test("streamJmap rejects malformed or truncated jmap documents in Chinese", async () => {
  await assert.rejects(
    streamJmap(chunkString("[]", 8), { onObject: () => undefined }),
    /顶层不是 JSON 对象/u,
  );
  await assert.rejects(
    streamJmap(chunkString('{"metadata":{}}', 8), { onObject: () => undefined }),
    /缺少顶层 objects 字段/u,
  );
  await assert.rejects(
    streamJmap(chunkString('{"objects":{"/Script/mucha.A":{"type":"Class"', 8), {
      onObject: () => undefined,
    }),
    /不完整或提前结束/u,
  );
});

void test("formatPropertyType renders primitive, container, reference and fixed-array types", () => {
  assert.equal(formatPropertyType({ type: "IntProperty", array_dim: 1 }), "int32");
  assert.equal(formatPropertyType({ type: "IntProperty", array_dim: 4 }), "int32[4]");
  assert.equal(
    formatPropertyType({ type: "ByteProperty", enum: "/Script/mucha.ESampleBattlePhase" }),
    "TEnumAsByte<ESampleBattlePhase>",
  );
  assert.equal(
    formatPropertyType({ type: "ObjectProperty", property_class: "/Script/mucha.CPP_Foo" }),
    "CPP_Foo*",
  );
  assert.equal(
    formatPropertyType({
      type: "MapProperty",
      key_prop: { type: "NameProperty" },
      value_prop: {
        type: "ArrayProperty",
        inner: { type: "SoftObjectProperty", property_class: "/Script/Engine.World" },
      },
    }),
    "TMap<FName, TArray<TSoftObjectPtr<World>>>",
  );
  assert.equal(
    formatPropertyType({
      type: "InterfaceProperty",
      interface_class: "/Script/EnhancedUI.CPP_EU_BI_DirGridPanel",
    }),
    "TScriptInterface<CPP_EU_BI_DirGridPanel>",
  );
  assert.equal(
    formatPropertyType({
      type: "OptionalProperty",
      inner: { type: "StructProperty", struct: "/Script/CoreUObject.Vector" },
    }),
    "TOptional<Vector>",
  );
});

void test("surveyJmapFile extracts mucha classes, structs, enums and /Game/ blueprints", async () => {
  const report = await surveyJmapFile(FIXTURE);

  assert.equal(report.sourceFile, "sample.jmap");
  assert.equal(report.filter, null);
  assert.equal(report.stats.totalObjects, 19);
  assert.equal(report.stats.muchaClassesTotal, 9);
  assert.equal(report.stats.muchaStructsTotal, 1);
  assert.equal(report.stats.muchaEnumsTotal, 1);
  assert.equal(report.stats.blueprintClassesTotal, 2);

  // Mucha class: properties sorted by offset, functions merged from children + Function objects.
  const party = report.mucha.classes.find((c) => c.name === "CPP_SamplePartySubsystem");
  assert.ok(party);
  assert.equal(party.superClass, "/Script/Engine.GameInstanceSubsystem");
  assert.deepEqual(party.properties, [
    { name: "Members", type: "TArray<SampleMemberStatus>", offset: 48 },
    { name: "Gil", type: "int32", offset: 64 },
    { name: "MemberLookup", type: "TMap<FName, TWeakObjectPtr<CPP_SampleFieldPawn>>", offset: 72 },
  ]);
  assert.deepEqual(party.functions, ["GetActiveParty", "IsPartyFull"]);

  const battle = report.mucha.classes.find((c) => c.name === "CPP_SampleBattleManager");
  assert.ok(battle);
  assert.deepEqual(battle.properties, [
    { name: "TurnNumber", type: "int32", offset: 40 },
    { name: "Phase", type: "ESampleBattlePhase", offset: 44 },
    { name: "TargetSlot", type: "TEnumAsByte<ESampleBattlePhase>", offset: 45 },
    { name: "ActorOrder", type: "int32[4]", offset: 48 },
  ]);

  // Mucha struct & enum.
  const struct = report.mucha.structs[0];
  assert.ok(struct);
  assert.equal(struct.name, "SampleMemberStatus");
  assert.equal(struct.superStruct, "/Script/Engine.TableRowBase");
  assert.deepEqual(struct.properties, [
    { name: "Hp", type: "int32", offset: 8 },
    { name: "Mp", type: "int32", offset: 12 },
  ]);

  const enumEntry = report.mucha.enums[0];
  assert.ok(enumEntry);
  assert.equal(enumEntry.name, "ESampleBattlePhase");
  assert.deepEqual(enumEntry.values, [
    { name: "ESampleBattlePhase::Start", value: 0 },
    { name: "ESampleBattlePhase::Command", value: 1 },
    { name: "ESampleBattlePhase::End", value: 2 },
  ]);

  // /Game/ blueprints keep only class name, super class, and property name + type (no offsets).
  const menuBp = report.blueprints.find((b) => b.name === "WBP_SampleMainMenu_C");
  assert.ok(menuBp);
  assert.equal(menuBp.generatedBy, "WidgetBlueprintGeneratedClass");
  assert.equal(menuBp.superClass, "/Script/UMG.UserWidget");
  assert.deepEqual(menuBp.properties, [
    { name: "CursorIndex", type: "int32" },
    { name: "EntryClass", type: "TSubclassOf<UserWidget>" },
  ]);
  for (const prop of menuBp.properties) {
    assert.equal("offset" in prop, false);
  }
});

void test("surveyJmapStream applies --filter regex and topic classification across all 10 topics", async () => {
  const raw = await readFile(FIXTURE, "utf8");
  const full = await surveyJmapStream(chunkString(raw, 29));

  const topicMap = new Map(full.topics.map((bucket) => [bucket.id, bucket]));
  assert.ok(topicMap.get("party")?.muchaClasses.includes("CPP_SamplePartySubsystem"));
  assert.ok(topicMap.get("party")?.muchaStructs.includes("SampleMemberStatus"));
  assert.ok(topicMap.get("battle")?.muchaClasses.includes("CPP_SampleBattleManager"));
  assert.ok(topicMap.get("battle")?.muchaEnums.includes("ESampleBattlePhase"));
  assert.ok(topicMap.get("field")?.muchaClasses.includes("CPP_SampleFieldPawn"));
  assert.ok(topicMap.get("field")?.blueprints.includes("BP_SampleFieldHero_C"));
  assert.ok(topicMap.get("map")?.muchaClasses.includes("CPP_SampleMapTransition"));
  assert.ok(topicMap.get("dialog")?.muchaClasses.includes("CPP_SampleDialogRunner"));
  assert.ok(topicMap.get("menu")?.blueprints.includes("WBP_SampleMainMenu_C"));
  assert.ok(topicMap.get("shop")?.muchaClasses.includes("CPP_SampleShopController"));
  assert.ok(topicMap.get("story")?.muchaClasses.includes("CPP_SampleStoryFlagManager"));
  assert.ok(topicMap.get("save")?.muchaClasses.includes("CPP_SampleSaveGame"));
  assert.ok(topicMap.get("input")?.muchaClasses.includes("CPP_SampleInputRegistrant"));

  const filtered = await surveyJmapStream(chunkString(raw, 29), {
    filter: compileFilter("Battle|MainMenu"),
  });
  assert.equal(filtered.filter, "Battle|MainMenu");
  assert.equal(filtered.stats.muchaClassesTotal, 9);
  assert.equal(filtered.stats.muchaClasses, 1);
  assert.equal(filtered.stats.muchaEnums, 1);
  assert.equal(filtered.stats.muchaStructs, 0);
  assert.equal(filtered.stats.blueprintClasses, 1);
  assert.equal(filtered.mucha.classes[0]?.name, "CPP_SampleBattleManager");
  assert.equal(filtered.blueprints[0]?.name, "WBP_SampleMainMenu_C");

  assert.deepEqual(classifyTopics("WBP_ShopMenu_C", "/Game/UI/Shop/WBP_ShopMenu.WBP_ShopMenu_C"), [
    "menu",
    "shop",
  ]);
  assert.throws(() => compileFilter("["), /--filter 正则无效/u);
  assert.throws(() => compileFilter(""), /不能为空/u);
});

void test("jmap-survey CLI writes JSON and Markdown and supports --filter and --stdout", async () => {
  const dir = await mkdtemp(join(tmpdir(), "ffr-jmap-survey-"));
  try {
    const outDir = join(dir, "survey");
    const run = await runCliCommand([FIXTURE, "--out", outDir]);
    assert.equal(run.code, 0, run.stderr);
    assert.match(run.stdout, /mucha:类 9\/9，结构体 1\/1，枚举 1\/1/u);
    assert.match(run.stdout, /蓝图类:2\/2/u);

    const jsonText = await readFile(join(outDir, "survey.json"), "utf8");
    const mdText = await readFile(join(outDir, "survey.md"), "utf8");
    const parsed = JSON.parse(jsonText) as JmapSurveyReport;
    assert.equal(parsed.mucha.classes.length, 9);
    assert.equal(parsed.blueprints.length, 2);
    assert.equal(mdText, renderSurveyMarkdown(parsed));
    assert.match(mdText, /0x0030.*Members.*TArray<SampleMemberStatus>/u);
    assert.match(mdText, /WBP_SampleMainMenu_C/u);

    const filtered = await runCliCommand([FIXTURE, "--filter", "SaveGame", "--stdout"]);
    assert.equal(filtered.code, 0, filtered.stderr);
    assert.match(filtered.stdout, /CPP_SampleSaveGame/u);
    assert.doesNotMatch(filtered.stdout, /CPP_SampleBattleManager/u);

    const badFilter = await runCliCommand([FIXTURE, "--filter", "[invalid"]);
    assert.equal(badFilter.code, 2);
    assert.match(badFilter.stderr, /--filter 正则无效/u);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
