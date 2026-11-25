/**
 * Smoke tests against the real unpacked export tree (<FFR_DEPS or .deps>/ffr/export/).
 * Skips cleanly when unpacked game data or locres exports are absent (CI or fresh checkouts).
 * Asserts only structural invariants so no game text or row content enters the public repository.
 */

import assert from "node:assert/strict";
import { access } from "node:fs/promises";
import { join } from "node:path";
import test from "node:test";
import {
  datatableDir,
  exportDir,
  getChapterMainScenario,
  getChapterName,
  loadDataTable,
  loadScenarioData,
  localizationFile,
  resolveMainScenarioTexts,
  resolveModelText,
  surveyExport,
} from "../../src/data/index.ts";

const REAL_EXPORT = exportDir();
const CHAPTER_FILE = join(datatableDir(REAL_EXPORT), "Scenario", "DT_ChapterData.json");

async function fileExists(path: string): Promise<boolean> {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}

await test("真实导出数据：章节、主线剧情与复合地图表结构核对", async (t) => {
  if (!(await fileExists(CHAPTER_FILE))) {
    t.skip("未找到导出的游戏数据表（设置 FFR_DEPS 或先运行 tools/ffr-export export）");
    return;
  }

  const scenario = await loadScenarioData(REAL_EXPORT);
  assert.equal(scenario.chapters.length, 9);
  assert.deepEqual(
    scenario.chapters.map((chapter) => chapter.chapterNo),
    [1, 2, 3, 4, 5, 6, 7, 8, 9],
  );

  const ch1Ja = await getChapterName(scenario, 1, "ja");
  assert.equal(ch1Ja.lang, "ja");
  assert.ok(ch1Ja.source === "localization" || ch1Ja.source === "string-table");
  assert.ok(ch1Ja.text !== null && ch1Ja.text.length > 0);

  // Unexported language must report missing strictly, and tag lang: "ja" when fallbackToNative is enabled.
  const ch1Missing = await getChapterName(scenario, 1, "xx");
  assert.equal(ch1Missing.text, null);
  assert.equal(ch1Missing.lang, null);
  assert.equal(ch1Missing.source, "missing");
  assert.ok(ch1Missing.reason?.includes("xx"));

  const ch1Fallback = await scenario.textResolver.resolve(scenario.chapters[0]?.name, "xx", {
    fallbackToNative: true,
  });
  assert.equal(ch1Fallback.lang, "ja");
  assert.equal(ch1Fallback.requestedLanguage, "xx");
  assert.equal(ch1Fallback.text, ch1Ja.text);

  assert.equal(scenario.mainScenario.length, 162);
  for (let chapterNo = 1; chapterNo <= 8; chapterNo++) {
    const entries = getChapterMainScenario(scenario.mainScenario, chapterNo);
    assert.ok(entries.length > 0, `第 ${chapterNo} 章应有主线剧情条目`);
    for (const entry of entries) {
      assert.ok(entry.minProgress !== null);
      assert.ok(
        entry.minProgress >= chapterNo * 1000 && entry.minProgress < (chapterNo + 1) * 1000,
      );
    }
  }

  const firstCh1 = getChapterMainScenario(scenario.mainScenario, 1)[0];
  assert.ok(firstCh1 !== undefined);
  const textsJa = await resolveMainScenarioTexts(scenario, firstCh1, "ja");
  assert.ok(textsJa.text.text !== null && textsJa.text.text.length > 0);
  assert.ok(textsJa.summaryText.text !== null && textsJa.summaryText.text.length > 0);

  const mapTable = await loadDataTable(
    join(datatableDir(REAL_EXPORT), "Map", "CDT_MapData_Prod.json"),
  );
  assert.equal(mapTable.type, "CompositeDataTable");
  assert.equal(mapTable.rowStruct, "MAP_DATA");
  assert.equal(mapTable.rows.size, 186);

  const scenarioSurvey = await surveyExport({ exportRoot: REAL_EXPORT, category: "Scenario" });
  assert.equal(scenarioSurvey.totalFiles, 2);
  assert.deepEqual(scenarioSurvey.strings.missingTableIds, []);
  assert.equal(scenarioSurvey.strings.missingKeyCount, 0);
});

await test("真实导出多语言 locres：默认 zh-Hans、en、ja 可按语言解析且与源语言区分", async (t) => {
  const hasAll =
    (await fileExists(CHAPTER_FILE)) &&
    (await fileExists(localizationFile("zh-Hans", REAL_EXPORT))) &&
    (await fileExists(localizationFile("en", REAL_EXPORT))) &&
    (await fileExists(localizationFile("ja", REAL_EXPORT)));
  if (!hasAll) {
    t.skip(
      "未找到导出的 Localization/{zh-Hans,en,ja}.json（可运行 tools/ffr-export/run.sh locres）",
    );
    return;
  }

  const scenario = await loadScenarioData(REAL_EXPORT);
  const ch1Zh = await getChapterName(scenario, 1);
  const ch1En = await getChapterName(scenario, 1, "en");
  const ch1Ja = await getChapterName(scenario, 1, "ja");

  assert.equal(ch1Zh.lang, "zh-Hans");
  assert.equal(ch1Zh.source, "localization");
  assert.ok(ch1Zh.text !== null && ch1Zh.text.length > 0);

  assert.equal(ch1En.lang, "en");
  assert.equal(ch1En.source, "localization");
  assert.ok(ch1En.text !== null && ch1En.text.length > 0);

  assert.equal(ch1Ja.lang, "ja");
  assert.equal(ch1Ja.source, "localization");
  assert.ok(ch1Ja.text !== null && ch1Ja.text.length > 0);

  // Confirm translations are genuinely distinct from Japanese source text.
  assert.notEqual(ch1Zh.text, ch1Ja.text);
  assert.notEqual(ch1En.text, ch1Ja.text);

  const firstCh1 = getChapterMainScenario(scenario.mainScenario, 1)[0];
  assert.ok(firstCh1 !== undefined);
  const modelText = await resolveModelText(firstCh1.text, { resolver: scenario.textResolver });
  assert.equal(modelText.lang, "zh-Hans");
  assert.equal(modelText.source, "localization");
  assert.ok(modelText.text !== null && modelText.text.length > 0);

  const scenarioSurvey = await surveyExport({ exportRoot: REAL_EXPORT, category: "Scenario" });
  for (const lang of ["en", "ja", "zh-Hans"]) {
    assert.ok(
      scenarioSurvey.strings.exportedLocalizationLanguages.includes(lang),
      `surveyExport 应识别到已导出语言 ${lang}`,
    );
  }
});
