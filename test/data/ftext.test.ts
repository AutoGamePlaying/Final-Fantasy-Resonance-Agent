/**
 * Tests for FText parsing, locres loading, and language-aware resolution against synthetic
 * StringTable and Localization/<lang>.json fixtures. Verifies locres-first lookup, "zh-Hans"
 * model defaults, strict missing reporting, and explicit fallbackToNative tagging.
 */

import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  createTextResolver,
  loadLocalization,
  localizationFile,
  MODEL_LANGUAGE,
  NATIVE_LANGUAGE,
  parseFText,
  parseLocalizationDocument,
  resolveFText,
  resolveModelText,
} from "../../src/data/index.ts";

const FIXTURE_EXPORT = join(import.meta.dirname, "..", "fixtures", "data", "export");
const CHAPTER_TABLE_ID = "/Game/Datatable/Strings/ChapterTitle/ST_ChapterTitle.ST_ChapterTitle";

await test("parseFText 识别三种 FText 形态并拒绝非法输入", () => {
  assert.deepEqual(
    parseFText({
      TableId: CHAPTER_TABLE_ID,
      Key: "Title_001",
      SourceString: "はじまりの森",
      LocalizedString: "はじまりの森",
    }),
    {
      kind: "string-table",
      tableId: CHAPTER_TABLE_ID,
      key: "Title_001",
      sourceString: "はじまりの森",
      localizedString: "はじまりの森",
    },
  );
  assert.deepEqual(parseFText({ CultureInvariantString: null }), {
    kind: "culture-invariant",
    value: null,
  });
  assert.deepEqual(parseFText({ CultureInvariantString: "HP 100" }), {
    kind: "culture-invariant",
    value: "HP 100",
  });
  assert.deepEqual(
    parseFText({
      Namespace: "DemoInline",
      Key: "Inline_001",
      SourceString: "テスト",
      LocalizedString: "テスト",
    }),
    {
      kind: "inline",
      namespace: "DemoInline",
      key: "Inline_001",
      sourceString: "テスト",
      localizedString: "テスト",
    },
  );
  assert.equal(parseFText("plain string"), null);
  assert.equal(parseFText({ foo: 1 }), null);
});

await test("createTextResolver 支持 language 参数、默认 zh-Hans，并按 locres 优先、源文本回落顺序查找", async () => {
  const defaultResolver = createTextResolver({ exportRoot: FIXTURE_EXPORT });
  assert.equal(defaultResolver.language, MODEL_LANGUAGE);
  assert.equal(defaultResolver.nativeLanguage, NATIVE_LANGUAGE);

  // Deliberately omit SourceString to prove lookup reads exported Localization/StringTable files.
  const title1 = { TableId: CHAPTER_TABLE_ID, Key: "Title_001" };
  const title2 = { TableId: CHAPTER_TABLE_ID, Key: "Title_002" };

  // Default language is zh-Hans.
  const defaultZh = await defaultResolver.resolve(title1);
  assert.equal(defaultZh.text, "起始之森");
  assert.equal(defaultZh.lang, "zh-Hans");
  assert.equal(defaultZh.requestedLanguage, "zh-Hans");
  assert.equal(defaultZh.reason, null);
  assert.equal(defaultZh.source, "localization");

  // Configured language option ("en").
  const enResolver = createTextResolver({ exportRoot: FIXTURE_EXPORT, language: "en" });
  const enTitle = await enResolver.resolve(title1);
  assert.equal(enTitle.text, "Forest of Beginnings");
  assert.equal(enTitle.lang, "en");
  assert.equal(enTitle.requestedLanguage, "en");
  assert.equal(enTitle.source, "localization");

  // ja: Title_001 exists in Localization/ja.json (locres first), Title_002 falls back to StringTable.
  const jaLocres = await defaultResolver.resolve(title1, "ja");
  assert.equal(jaLocres.text, "はじまりの森");
  assert.equal(jaLocres.lang, "ja");
  assert.equal(jaLocres.source, "localization");

  const jaSource = await defaultResolver.resolve(title2, "ja");
  assert.equal(jaSource.text, "さざなみの港");
  assert.equal(jaSource.lang, "ja");
  assert.equal(jaSource.reason, null);
  assert.equal(jaSource.source, "string-table");

  // Model-facing convenience helpers default to zh-Hans even when sharing an "en" resolver.
  const viaResolveFText = await resolveFText(title1, { exportRoot: FIXTURE_EXPORT });
  assert.equal(viaResolveFText.text, "起始之森");
  assert.equal(viaResolveFText.lang, "zh-Hans");

  const viaModelHelper = await resolveModelText(title1, { resolver: enResolver });
  assert.equal(viaModelHelper.text, "起始之森");
  assert.equal(viaModelHelper.lang, "zh-Hans");
  assert.equal(viaModelHelper.requestedLanguage, "zh-Hans");
});

await test("查不到表、键或目标语言时默认返回 null 与中文原因，绝不把日文冒充中文", async () => {
  const resolver = createTextResolver({ exportRoot: FIXTURE_EXPORT });

  // Missing key in ja StringTable (even when SourceString is present on the FText object).
  const missingKeyJa = await resolver.resolve(
    {
      TableId: CHAPTER_TABLE_ID,
      Key: "Title_999",
      SourceString: "不应该被回退返回",
    },
    "ja",
  );
  assert.equal(missingKeyJa.text, null);
  assert.equal(missingKeyJa.lang, null);
  assert.equal(missingKeyJa.requestedLanguage, "ja");
  assert.equal(missingKeyJa.source, "missing");
  assert.match(missingKeyJa.reason ?? "", /不存在键 Title_999/u);

  // Key exists in ja, but missing in zh-Hans localization fixture.
  const missingKeyZh = await resolver.resolve(
    { TableId: CHAPTER_TABLE_ID, Key: "Title_003" },
    "zh-Hans",
  );
  assert.equal(missingKeyZh.text, null);
  assert.equal(missingKeyZh.lang, null);
  assert.equal(missingKeyZh.requestedLanguage, "zh-Hans");
  assert.equal(missingKeyZh.source, "missing");
  assert.match(missingKeyZh.reason ?? "", /zh-Hans.*ST_ChapterTitle\/Title_003/u);

  // Target language has no exported localization file at all.
  const missingLang = await resolver.resolve({ TableId: CHAPTER_TABLE_ID, Key: "Title_001" }, "ko");
  assert.equal(missingLang.text, null);
  assert.equal(missingLang.lang, null);
  assert.equal(missingLang.requestedLanguage, "ko");
  assert.equal(missingLang.source, "missing");
  assert.match(missingLang.reason ?? "", /缺少语言 ko/u);

  // Missing StringTable file.
  const missingTable = await resolver.resolve(
    { TableId: "/Game/Datatable/Strings/NoSuch/ST_None.ST_None", Key: "K1" },
    "ja",
  );
  assert.equal(missingTable.text, null);
  assert.equal(missingTable.lang, null);
  assert.match(missingTable.reason ?? "", /找不到字符串表文件/u);

  // TableId outside /Game/.
  const badPath = await resolver.resolve({ TableId: "/Engine/ST_Bad.ST_Bad", Key: "K1" }, "ja");
  assert.equal(badPath.text, null);
  assert.equal(badPath.lang, null);
  assert.match(badPath.reason ?? "", /无法把字符串表路径映射到导出目录/u);
});

await test("显式开启 fallbackToNative 时回落到源语言并如实标明 lang: ja", async () => {
  const resolver = createTextResolver({
    exportRoot: FIXTURE_EXPORT,
    language: "zh-Hans",
    fallbackToNative: true,
  });

  // Missing key in zh-Hans falls back to ja StringTable, reporting lang: "ja".
  const title3Fallback = await resolver.resolve({ TableId: CHAPTER_TABLE_ID, Key: "Title_003" });
  assert.equal(title3Fallback.text, "しずかな塔");
  assert.equal(title3Fallback.lang, "ja");
  assert.equal(title3Fallback.requestedLanguage, "zh-Hans");
  assert.equal(title3Fallback.source, "string-table");
  assert.match(title3Fallback.reason ?? "", /已回落到源语言 ja/u);

  // Missing language file ("ko") also reports lang: "ja" when fallbackToNative is enabled.
  const koFallback = await resolver.resolve({ TableId: CHAPTER_TABLE_ID, Key: "Title_001" }, "ko");
  assert.equal(koFallback.text, "はじまりの森");
  assert.equal(koFallback.lang, "ja");
  assert.equal(koFallback.requestedLanguage, "ko");
  assert.equal(koFallback.source, "string-table");
  assert.match(koFallback.reason ?? "", /缺少语言 ko.*已回落到源语言 ja/u);

  // Per-call override can disable or enable fallback independently.
  const strictResolver = createTextResolver({ exportRoot: FIXTURE_EXPORT, language: "zh-Hans" });
  const perCallFallback = await strictResolver.resolve(
    { TableId: CHAPTER_TABLE_ID, Key: "Title_003" },
    "zh-Hans",
    { fallbackToNative: true },
  );
  assert.equal(perCallFallback.text, "しずかな塔");
  assert.equal(perCallFallback.lang, "ja");
  assert.equal(perCallFallback.requestedLanguage, "zh-Hans");
});

await test("CultureInvariantString、内联 FText、非 FText 输入与损坏本地化文件校验", async () => {
  const resolver = createTextResolver({ exportRoot: FIXTURE_EXPORT });

  const invariant = await resolver.resolve({ CultureInvariantString: "LV 99" }, "en");
  assert.equal(invariant.text, "LV 99");
  assert.equal(invariant.lang, "en");
  assert.equal(invariant.source, "culture-invariant");

  const empty = await resolver.resolve({ CultureInvariantString: null }, "ja");
  assert.equal(empty.text, null);
  assert.equal(empty.lang, null);
  assert.match(empty.reason ?? "", /为空/u);

  const inlineRef = {
    Namespace: "DemoInline",
    Key: "Inline_001",
    SourceString: "デモ内联",
  };
  const inlineJa = await resolver.resolve(inlineRef, "ja");
  assert.equal(inlineJa.text, "デモ内联");
  assert.equal(inlineJa.lang, "ja");
  assert.equal(inlineJa.source, "inline-source");

  const inlineZh = await resolver.resolve(inlineRef, "zh-Hans");
  assert.equal(inlineZh.text, "中文内联提示");
  assert.equal(inlineZh.lang, "zh-Hans");
  assert.equal(inlineZh.source, "localization");

  const inlineEnStrict = await resolver.resolve(inlineRef, "en");
  assert.equal(inlineEnStrict.text, null);
  assert.equal(inlineEnStrict.lang, null);
  assert.match(inlineEnStrict.reason ?? "", /en.*DemoInline\/Inline_001/u);

  const inlineEnFallback = await resolver.resolve(inlineRef, "en", { fallbackToNative: true });
  assert.equal(inlineEnFallback.text, "デモ内联");
  assert.equal(inlineEnFallback.lang, "ja");
  assert.equal(inlineEnFallback.requestedLanguage, "en");
  assert.equal(inlineEnFallback.source, "inline-source");

  const inlineKo = await resolver.resolve(inlineRef, "ko");
  assert.equal(inlineKo.text, null);
  assert.match(inlineKo.reason ?? "", /缺少语言 ko/u);

  const invalid = await resolveFText(12345, { exportRoot: FIXTURE_EXPORT });
  assert.equal(invalid.text, null);
  assert.equal(invalid.lang, null);
  assert.match(invalid.reason ?? "", /不是合法的 FText/u);

  // loadLocalization and parseLocalizationDocument validate format.
  const zhCatalog = await loadLocalization(localizationFile("zh-Hans", FIXTURE_EXPORT));
  assert.equal(zhCatalog.get("ST_ChapterTitle")?.get("Title_001"), "起始之森");
  assert.throws(() => parseLocalizationDocument([], "bad.json"), /不是 JSON 对象/u);
  assert.throws(
    () => parseLocalizationDocument({ NS: { K: 123 } }, "bad-entry.json"),
    /不是字符串/u,
  );

  // Corrupt ja.json must surface as an error rather than silently falling back.
  const tempRoot = await mkdtemp(join(tmpdir(), "ffr-locres-bad-"));
  try {
    await mkdir(join(tempRoot, "Localization"), { recursive: true });
    await writeFile(join(tempRoot, "Localization", "ja.json"), "{ broken", "utf8");
    const badResolver = createTextResolver({ exportRoot: tempRoot, language: "ja" });
    const badResult = await badResolver.resolve(inlineRef, "ja");
    assert.equal(badResult.text, null);
    assert.equal(badResult.source, "missing");
    assert.match(badResult.reason ?? "", /加载语言 ja 的本地化表失败/u);
  } finally {
    await rm(tempRoot, { recursive: true, force: true });
  }
});
