/**
 * Parsing and language-aware resolution for Unreal Engine FText values exported by CUE4Parse.
 *
 * Lookup follows UE's localization order: look up (namespace, key) in the requested language's
 * exported locres JSON (<exportRoot>/Localization/<lang>.json) first; only when absent do we
 * consider the source text (exported StringTable or inline SourceString).
 *
 * Non-native lookups never silently return Japanese source text labeled as the target language:
 * by default they return { text: null, lang: null, source: "missing", reason }; with
 * fallbackToNative: true they return { text, lang: nativeLanguage, reason } so callers always
 * know the true language of the text. Model-facing helpers default to "zh-Hans".
 */

import { readFile } from "node:fs/promises";
import { describeError, isMissingFile, isRecord } from "../shared/index.ts";
import { loadStringTable, type StringTable } from "./datatable.ts";
import { exportDir, localizationFile, ueObjectPathToExportJson } from "./paths.ts";

export const NATIVE_LANGUAGE = "ja";
export const MODEL_LANGUAGE = "zh-Hans";
export const DEFAULT_LOCRES_LANGUAGES = ["zh-Hans", "en", "ja"] as const;

export interface StringTableFText {
  readonly kind: "string-table";
  readonly tableId: string;
  readonly key: string;
  readonly sourceString: string | null;
  readonly localizedString: string | null;
}

export interface InlineFText {
  readonly kind: "inline";
  readonly namespace: string;
  readonly key: string;
  readonly sourceString: string | null;
  readonly localizedString: string | null;
}

export interface CultureInvariantFText {
  readonly kind: "culture-invariant";
  readonly value: string | null;
}

export type FTextValue = StringTableFText | InlineFText | CultureInvariantFText;

export type TextSource =
  "localization" | "string-table" | "culture-invariant" | "inline-source" | "missing";

export interface ResolvedText {
  readonly text: string | null;
  /** True language of `text`, or null when `text` is null (missing). */
  readonly lang: string | null;
  /** Language requested by the caller. */
  readonly requestedLanguage: string;
  readonly reason: string | null;
  readonly source: TextSource;
}

/** Map of UE localization namespace -> (key -> localized text) for one language. */
export type LocalizationCatalog = ReadonlyMap<string, ReadonlyMap<string, string>>;

export interface TextResolverOptions {
  readonly exportRoot?: string;
  /** Default target language when resolve() is called without an explicit language (default "zh-Hans"). */
  readonly language?: string;
  /** Source language of StringTables / inline SourceString (default "ja"). */
  readonly nativeLanguage?: string;
  /**
   * When true, missing non-native translations fall back to source text while reporting
   * `lang: nativeLanguage` and recording the fallback in `reason`. Defaults to false.
   */
  readonly fallbackToNative?: boolean;
  readonly localizations?: ReadonlyMap<string, LocalizationCatalog>;
}

export interface ResolveCallOptions {
  readonly fallbackToNative?: boolean;
}

export interface TextResolver {
  readonly exportRoot: string;
  readonly language: string;
  readonly nativeLanguage: string;
  readonly fallbackToNative: boolean;
  resolve(
    value: unknown,
    language?: string,
    callOptions?: ResolveCallOptions,
  ): Promise<ResolvedText>;
  loadStringTableById(tableId: string): Promise<StringTable | null>;
  loadLocalizationCatalog(language: string): Promise<LocalizationCatalog | null>;
}

interface CachedTableResult {
  readonly table: StringTable | null;
  readonly errorReason: string | null;
}

type CachedCatalogResult =
  | { readonly status: "loaded"; readonly catalog: LocalizationCatalog }
  | { readonly status: "not-found"; readonly errorReason: string }
  | { readonly status: "error"; readonly errorReason: string };

function optionalString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function isParsedFText(value: unknown): value is FTextValue {
  if (!isRecord(value)) return false;
  if (
    value.kind === "string-table" &&
    typeof value.tableId === "string" &&
    typeof value.key === "string"
  ) {
    return true;
  }
  if (
    value.kind === "inline" &&
    typeof value.namespace === "string" &&
    typeof value.key === "string"
  ) {
    return true;
  }
  if (
    value.kind === "culture-invariant" &&
    (value.value === null || typeof value.value === "string")
  ) {
    return true;
  }
  return false;
}

/** Narrow a raw CUE4Parse FText JSON object (or an already parsed FTextValue) into FTextValue. */
export function parseFText(value: unknown): FTextValue | null {
  if (isParsedFText(value)) return value;
  if (!isRecord(value)) return null;
  if (typeof value.TableId === "string" && typeof value.Key === "string") {
    return {
      kind: "string-table",
      tableId: value.TableId,
      key: value.Key,
      sourceString: optionalString(value.SourceString),
      localizedString: optionalString(value.LocalizedString),
    };
  }
  if (Object.hasOwn(value, "CultureInvariantString")) {
    const raw = value.CultureInvariantString;
    if (raw === null || typeof raw === "string") {
      return { kind: "culture-invariant", value: raw };
    }
    return null;
  }
  if (typeof value.Namespace === "string" && typeof value.Key === "string") {
    return {
      kind: "inline",
      namespace: value.Namespace,
      key: value.Key,
      sourceString: optionalString(value.SourceString),
      localizedString: optionalString(value.LocalizedString),
    };
  }
  return null;
}

/** Narrow a parsed `{ namespace: { key: text } }` locres JSON document into LocalizationCatalog. */
export function parseLocalizationDocument(
  document: unknown,
  sourceLabel: string,
): LocalizationCatalog {
  if (!isRecord(document)) {
    throw new Error(`本地化文件 ${sourceLabel} 不是 JSON 对象`);
  }
  const catalog = new Map<string, Map<string, string>>();
  for (const [namespace, rawBucket] of Object.entries(document)) {
    if (!isRecord(rawBucket)) {
      throw new Error(`本地化文件 ${sourceLabel} 的命名空间 ${namespace} 不是对象`);
    }
    const bucket = new Map<string, string>();
    for (const [key, entry] of Object.entries(rawBucket)) {
      if (typeof entry !== "string") {
        throw new Error(`本地化文件 ${sourceLabel} 中 ${namespace}/${key} 不是字符串`);
      }
      bucket.set(key, entry);
    }
    catalog.set(namespace, bucket);
  }
  return catalog;
}

/** Load one exported locres JSON file (`<exportRoot>/Localization/<lang>.json`) from disk. */
export async function loadLocalization(filePath: string): Promise<LocalizationCatalog> {
  let rawText: string;
  try {
    rawText = await readFile(filePath, "utf8");
  } catch (error) {
    throw new Error(`读取本地化文件失败（${filePath}）：${describeError(error)}`, { cause: error });
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(rawText) as unknown;
  } catch (error) {
    throw new Error(`本地化文件 JSON 格式错误（${filePath}）：${describeError(error)}`, {
      cause: error,
    });
  }
  return parseLocalizationDocument(parsed, filePath);
}

function missing(requestedLanguage: string, reason: string): ResolvedText {
  return {
    text: null,
    lang: null,
    requestedLanguage,
    reason,
    source: "missing",
  };
}

function isSafeLanguageTag(language: string): boolean {
  return /^[A-Za-z0-9-]+$/u.test(language);
}

/**
 * Create a cached, language-aware FText resolver over one export tree.
 * Defaults to `language = "zh-Hans"`, looking up `<exportRoot>/Localization/<lang>.json`
 * first and falling back to StringTable/inline source text only for the native language or
 * when `fallbackToNative: true` is explicitly enabled (in which case `lang` reports `"ja"`).
 */
export function createTextResolver(options: TextResolverOptions = {}): TextResolver {
  const exportRoot = options.exportRoot ?? exportDir();
  const defaultLanguage = options.language ?? MODEL_LANGUAGE;
  const nativeLanguage = options.nativeLanguage ?? NATIVE_LANGUAGE;
  const defaultFallbackToNative = options.fallbackToNative ?? false;
  const tableCache = new Map<string, CachedTableResult>();
  const catalogCache = new Map<string, CachedCatalogResult>();

  if (options.localizations !== undefined) {
    for (const [language, catalog] of options.localizations) {
      catalogCache.set(language, { status: "loaded", catalog });
    }
  }

  async function loadTableCached(tableId: string): Promise<CachedTableResult> {
    const cached = tableCache.get(tableId);
    if (cached !== undefined) return cached;
    const jsonPath = ueObjectPathToExportJson(tableId, exportRoot);
    if (jsonPath === null) {
      const result: CachedTableResult = {
        table: null,
        errorReason: `无法把字符串表路径映射到导出目录：${tableId}`,
      };
      tableCache.set(tableId, result);
      return result;
    }
    try {
      const table = await loadStringTable(jsonPath);
      const result: CachedTableResult = { table, errorReason: null };
      tableCache.set(tableId, result);
      return result;
    } catch (error) {
      const reason = isMissingFile(error instanceof Error ? error.cause : error)
        ? `找不到字符串表文件：${tableId}`
        : `加载字符串表 ${tableId} 失败：${describeError(error)}`;
      const result: CachedTableResult = { table: null, errorReason: reason };
      tableCache.set(tableId, result);
      return result;
    }
  }

  async function loadCatalogCached(language: string): Promise<CachedCatalogResult> {
    const cached = catalogCache.get(language);
    if (cached !== undefined) return cached;
    if (!isSafeLanguageTag(language)) {
      const result: CachedCatalogResult = {
        status: "error",
        errorReason: `语言代码不合法：${language}`,
      };
      catalogCache.set(language, result);
      return result;
    }
    const jsonPath = localizationFile(language, exportRoot);
    try {
      const catalog = await loadLocalization(jsonPath);
      const result: CachedCatalogResult = { status: "loaded", catalog };
      catalogCache.set(language, result);
      return result;
    } catch (error) {
      const missingFile = isMissingFile(error instanceof Error ? error.cause : error);
      const result: CachedCatalogResult = missingFile
        ? {
            status: "not-found",
            errorReason: `缺少语言 ${language} 的本地化表（可运行 ../final-fantasy-resonance-export/scripts/run.sh locres --lang ${language} 导出到 Localization/${language}.json）`,
          }
        : {
            status: "error",
            errorReason: `加载语言 ${language} 的本地化表失败：${describeError(error)}`,
          };
      catalogCache.set(language, result);
      return result;
    }
  }

  async function resolveStringTableText(
    ftext: StringTableFText,
    language: string,
    fallbackToNative: boolean,
  ): Promise<ResolvedText> {
    const loaded = await loadTableCached(ftext.tableId);
    if (loaded.table === null) {
      return missing(language, loaded.errorReason ?? `找不到字符串表：${ftext.tableId}`);
    }

    const ns = loaded.table.tableNamespace;
    const loc = await loadCatalogCached(language);
    if (loc.status === "error") {
      return missing(language, loc.errorReason);
    }
    if (loc.status === "loaded") {
      const translated = loc.catalog.get(ns)?.get(ftext.key);
      if (translated !== undefined) {
        return {
          text: translated,
          lang: language,
          requestedLanguage: language,
          reason: null,
          source: "localization",
        };
      }
    }

    const missingReason =
      loc.status === "not-found"
        ? loc.errorReason
        : `语言 ${language} 的本地化表中缺少 ${ns}/${ftext.key}`;

    if (language === nativeLanguage) {
      const found = loaded.table.entries.get(ftext.key);
      if (found === undefined) {
        return missing(language, `字符串表 ${ns}（${ftext.tableId}）中不存在键 ${ftext.key}`);
      }
      return {
        text: found,
        lang: nativeLanguage,
        requestedLanguage: language,
        reason: null,
        source: "string-table",
      };
    }

    if (!fallbackToNative) {
      return missing(language, missingReason);
    }

    const fallbackText = loaded.table.entries.get(ftext.key);
    if (fallbackText === undefined) {
      return missing(language, `${missingReason}；且源语言字符串表 ${ns} 中不存在键 ${ftext.key}`);
    }
    return {
      text: fallbackText,
      lang: nativeLanguage,
      requestedLanguage: language,
      reason: `${missingReason}，已回落到源语言 ${nativeLanguage}`,
      source: "string-table",
    };
  }

  async function resolveInlineText(
    ftext: InlineFText,
    language: string,
    fallbackToNative: boolean,
  ): Promise<ResolvedText> {
    const nsLabel = ftext.namespace || "(空命名空间)";
    const loc = await loadCatalogCached(language);
    if (loc.status === "error") {
      return missing(language, loc.errorReason);
    }
    if (loc.status === "loaded") {
      const translated = loc.catalog.get(ftext.namespace)?.get(ftext.key);
      if (translated !== undefined) {
        return {
          text: translated,
          lang: language,
          requestedLanguage: language,
          reason: null,
          source: "localization",
        };
      }
    }

    const missingReason =
      loc.status === "not-found"
        ? loc.errorReason
        : `语言 ${language} 的本地化表中缺少内联文本 ${nsLabel}/${ftext.key}`;

    if (language === nativeLanguage) {
      if (ftext.sourceString === null) {
        return missing(language, `内联 FText（${nsLabel}/${ftext.key}）缺少 SourceString`);
      }
      return {
        text: ftext.sourceString,
        lang: nativeLanguage,
        requestedLanguage: language,
        reason: null,
        source: "inline-source",
      };
    }

    if (!fallbackToNative) {
      return missing(language, missingReason);
    }

    if (ftext.sourceString === null) {
      return missing(language, `${missingReason}；且内联 FText 缺少 SourceString`);
    }
    return {
      text: ftext.sourceString,
      lang: nativeLanguage,
      requestedLanguage: language,
      reason: `${missingReason}，已回落到源语言 ${nativeLanguage}`,
      source: "inline-source",
    };
  }

  return {
    exportRoot,
    language: defaultLanguage,
    nativeLanguage,
    fallbackToNative: defaultFallbackToNative,
    async loadStringTableById(tableId: string): Promise<StringTable | null> {
      const loaded = await loadTableCached(tableId);
      return loaded.table;
    },
    async loadLocalizationCatalog(language: string): Promise<LocalizationCatalog | null> {
      const loaded = await loadCatalogCached(language);
      return loaded.status === "loaded" ? loaded.catalog : null;
    },
    async resolve(
      value: unknown,
      language: string = defaultLanguage,
      callOptions: ResolveCallOptions = {},
    ): Promise<ResolvedText> {
      const fallbackToNative = callOptions.fallbackToNative ?? defaultFallbackToNative;
      const parsed = parseFText(value);
      if (parsed === null) {
        return missing(language, "不是合法的 FText 对象");
      }
      switch (parsed.kind) {
        case "culture-invariant":
          if (parsed.value === null) {
            return missing(language, "FText 为空（CultureInvariantString 为 null）");
          }
          return {
            text: parsed.value,
            lang: language,
            requestedLanguage: language,
            reason: null,
            source: "culture-invariant",
          };
        case "string-table":
          return resolveStringTableText(parsed, language, fallbackToNative);
        case "inline":
          return resolveInlineText(parsed, language, fallbackToNative);
      }
    },
  };
}

export interface ResolveFTextOptions {
  readonly language?: string;
  readonly exportRoot?: string;
  readonly resolver?: TextResolver;
  readonly fallbackToNative?: boolean;
}

/**
 * One-shot helper for resolving a single FText value (defaults to "zh-Hans", or `resolver.language`
 * when a resolver is passed). Prefer createTextResolver for batch work.
 */
export async function resolveFText(
  value: unknown,
  options: ResolveFTextOptions = {},
): Promise<ResolvedText> {
  const resolver =
    options.resolver ??
    createTextResolver({
      exportRoot: options.exportRoot,
      language: options.language ?? MODEL_LANGUAGE,
      fallbackToNative: options.fallbackToNative,
    });
  return resolver.resolve(value, options.language, {
    fallbackToNative: options.fallbackToNative,
  });
}

export interface ResolveModelTextOptions {
  readonly exportRoot?: string;
  readonly resolver?: TextResolver;
  readonly fallbackToNative?: boolean;
}

/**
 * Convenience helper for model-facing text: always resolves in Simplified Chinese ("zh-Hans"),
 * reusing an existing TextResolver's caches when provided.
 */
export async function resolveModelText(
  value: unknown,
  options: ResolveModelTextOptions = {},
): Promise<ResolvedText> {
  const resolver =
    options.resolver ??
    createTextResolver({
      exportRoot: options.exportRoot,
      language: MODEL_LANGUAGE,
      fallbackToNative: options.fallbackToNative,
    });
  return resolver.resolve(value, MODEL_LANGUAGE, {
    fallbackToNative: options.fallbackToNative,
  });
}
