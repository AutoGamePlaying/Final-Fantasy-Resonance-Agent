/**
 * Tests for typed Scenario loaders and minimal queries (chapters and main scenario entries)
 * against synthetic fixtures, including default "zh-Hans" and explicit language lookups.
 */

import assert from "node:assert/strict";
import { join } from "node:path";
import test from "node:test";
import {
  chapterNoFromProgress,
  findChapter,
  getChapterMainScenario,
  getChapterName,
  loadChapters,
  loadMainScenario,
  loadScenarioData,
  parseMainProgressCondition,
  resolveMainScenarioTexts,
} from "../../src/data/index.ts";

const FIXTURE_EXPORT = join(import.meta.dirname, "..", "fixtures", "data", "export");

await test("parseMainProgressCondition 与 chapterNoFromProgress 解析主线进度表达式", () => {
  assert.equal(parseMainProgressCondition("{progress:0}>=1020"), 1020);
  assert.equal(parseMainProgressCondition(" {progress:0}>=8050 "), 8050);
  assert.equal(parseMainProgressCondition(""), null);
  assert.equal(parseMainProgressCondition("{progress:1}==50"), null);

  assert.equal(chapterNoFromProgress(1020), 1);
  assert.equal(chapterNoFromProgress(2000), 2);
  assert.equal(chapterNoFromProgress(8050), 8);
  assert.equal(chapterNoFromProgress(999), null);
});

await test("loadChapters 按章节号升序加载，getChapterName 默认 zh-Hans 并支持按语言切换", async () => {
  const chapters = await loadChapters(FIXTURE_EXPORT);
  assert.deepEqual(
    chapters.map((chapter) => chapter.chapterNo),
    [1, 2, 3],
  );
  assert.equal(findChapter(chapters, 2)?.rowKey, "Ch2");
  assert.equal(findChapter(chapters, 99), null);

  const scenario = await loadScenarioData(FIXTURE_EXPORT);
  const ch1Default = await getChapterName(scenario, 1);
  assert.equal(ch1Default.text, "起始之森");
  assert.equal(ch1Default.lang, "zh-Hans");

  const ch1Ja = await getChapterName(scenario, 1, "ja");
  assert.equal(ch1Ja.text, "はじまりの森");
  assert.equal(ch1Ja.lang, "ja");

  const ch1En = await getChapterName(scenario, 1, "en");
  assert.equal(ch1En.text, "Forest of Beginnings");
  assert.equal(ch1En.lang, "en");

  const ch3Zh = await getChapterName(scenario, 3, "zh-Hans");
  assert.equal(ch3Zh.text, null);
  assert.equal(ch3Zh.lang, null);
  assert.match(ch3Zh.reason ?? "", /Title_003/u);

  const missingCh = await getChapterName(scenario, 99, "ja");
  assert.equal(missingCh.text, null);
  assert.equal(missingCh.lang, null);
  assert.match(missingCh.reason ?? "", /找不到第 99 章/u);

  const enScenario = await loadScenarioData(FIXTURE_EXPORT, { language: "en" });
  const ch2EnDefault = await getChapterName(enScenario, 2);
  assert.equal(ch2EnDefault.text, "Rippling Harbor");
  assert.equal(ch2EnDefault.lang, "en");
});

await test("loadMainScenario 与 getChapterMainScenario 按章筛选、按进度排序并解析目的地与文本", async () => {
  const entries = await loadMainScenario(FIXTURE_EXPORT);
  assert.equal(entries.length, 4);
  const placeholder = entries.find((entry) => entry.rowKey === "NewRow");
  assert.ok(placeholder !== undefined);
  assert.equal(placeholder.minProgress, null);
  assert.equal(placeholder.chapterNo, null);

  const ch1 = getChapterMainScenario(entries, 1);
  assert.deepEqual(
    ch1.map((entry) => [entry.rowKey, entry.minProgress]),
    [
      ["Step_1010", 1010],
      ["Step_1050", 1050],
    ],
  );
  const firstCh1 = ch1[0];
  const secondCh1 = ch1[1];
  assert.ok(firstCh1 !== undefined && secondCh1 !== undefined);
  const firstDest = firstCh1.destinations[0];
  assert.ok(firstDest !== undefined);
  assert.equal(firstDest.destinationId, 2001);
  assert.equal(firstDest.objectType, "eMapObjectType::NPC");
  assert.equal(firstDest.objectId, 12);
  assert.equal(secondCh1.destinations[0]?.condition, "!{flag:101}");

  const ch2 = getChapterMainScenario(entries, 2);
  assert.equal(ch2.length, 1);
  assert.equal(ch2[0]?.destinations[0]?.destinationId, 3001);

  assert.deepEqual(getChapterMainScenario(entries, 3), []);

  const scenario = await loadScenarioData(FIXTURE_EXPORT);
  // Default resolves in zh-Hans.
  const textsDefault = await resolveMainScenarioTexts(scenario, firstCh1);
  assert.equal(textsDefault.text.text, "与村口向导交谈");
  assert.equal(textsDefault.text.lang, "zh-Hans");
  assert.equal(textsDefault.summaryText.text, "旅人抵达了森林入口。");
  assert.equal(textsDefault.summaryText.lang, "zh-Hans");

  const textsEn = await resolveMainScenarioTexts(scenario, firstCh1, "en");
  assert.equal(textsEn.text.text, "Speak with the village guide");
  assert.equal(textsEn.summaryText.text, "The travelers arrived at the forest entrance.");

  const textsJa = await resolveMainScenarioTexts(scenario, firstCh1, "ja");
  assert.equal(textsJa.text.text, "村の案内人に話しかける");
  assert.equal(textsJa.summaryText.text, "旅人は森の入口にたどり着いた。");

  const placeholderTexts = await resolveMainScenarioTexts(scenario, placeholder, "ja");
  assert.equal(placeholderTexts.text.text, null);
  assert.match(placeholderTexts.text.reason ?? "", /为空/u);
});
