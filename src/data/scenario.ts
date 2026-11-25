/**
 * Typed loaders and minimal queries for Scenario tables: DT_ChapterData and DT_MainScenario
 * (docs/plan.md §3 R2, docs/data-tables.md).
 *
 * Chapter membership of main scenario entries comes from DT_MainScenario.Condition
 * ("{progress:0}>=<N>"), where progress 1000..1999 is chapter 1, 2000..2999 is chapter 2,
 * and so on through chapter 8 (cross-verified against Activity/DT_ActivityData).
 */

import { join } from "node:path";
import { isRecord } from "../shared/index.ts";
import { loadDataTable } from "./datatable.ts";
import {
  createTextResolver,
  parseFText,
  type FTextValue,
  type ResolvedText,
  type TextResolver,
  type TextResolverOptions,
} from "./ftext.ts";
import { datatableDir, exportDir } from "./paths.ts";

export interface Chapter {
  readonly rowKey: string;
  readonly chapterNo: number;
  readonly name: FTextValue;
}

export interface ScenarioDestination {
  readonly pointType: string;
  readonly destinationType: string;
  readonly destinationId: number;
  readonly isPointToObject: boolean;
  readonly objectType: string;
  readonly objectId: number;
  readonly condition: string;
}

export interface MainScenarioEntry {
  readonly rowKey: string;
  readonly condition: string;
  readonly minProgress: number | null;
  readonly chapterNo: number | null;
  readonly text: FTextValue;
  readonly summaryText: FTextValue;
  readonly destinations: readonly ScenarioDestination[];
}

export interface ScenarioData {
  readonly chapters: readonly Chapter[];
  readonly mainScenario: readonly MainScenarioEntry[];
  readonly textResolver: TextResolver;
}

export interface ResolvedMainScenarioTexts {
  readonly text: ResolvedText;
  readonly summaryText: ResolvedText;
}

const MAIN_PROGRESS_CONDITION = /^\{progress:0\}>=(\d+)$/u;

/** Parse "{progress:0}>=<N>" into the numeric progress threshold N, or null if not in that form. */
export function parseMainProgressCondition(condition: string): number | null {
  const match = MAIN_PROGRESS_CONDITION.exec(condition.trim());
  if (match?.[1] === undefined) return null;
  const parsed = Number.parseInt(match[1], 10);
  return Number.isSafeInteger(parsed) ? parsed : null;
}

/** Derive chapter number from main progress: 1000..1999 -> 1, 2000..2999 -> 2, etc. */
export function chapterNoFromProgress(progress: number): number | null {
  if (!Number.isFinite(progress) || progress < 1000) return null;
  return Math.floor(progress / 1000);
}

function expectInteger(row: Record<string, unknown>, field: string, context: string): number {
  const value = row[field];
  if (typeof value !== "number" || !Number.isInteger(value)) {
    throw new Error(`${context} 的 ${field} 字段不是整数`);
  }
  return value;
}

function expectString(row: Record<string, unknown>, field: string, context: string): string {
  const value = row[field];
  if (typeof value !== "string") {
    throw new Error(`${context} 的 ${field} 字段不是字符串`);
  }
  return value;
}

function expectBoolean(row: Record<string, unknown>, field: string, context: string): boolean {
  const value = row[field];
  if (typeof value !== "boolean") {
    throw new Error(`${context} 的 ${field} 字段不是布尔值`);
  }
  return value;
}

function expectFText(row: Record<string, unknown>, field: string, context: string): FTextValue {
  const parsed = parseFText(row[field]);
  if (parsed === null) {
    throw new Error(`${context} 的 ${field} 字段不是合法的 FText`);
  }
  return parsed;
}

function parseDestination(value: unknown, context: string): ScenarioDestination {
  if (!isRecord(value)) {
    throw new Error(`${context} 不是对象`);
  }
  const destKey = value.DestinationKey;
  if (!isRecord(destKey)) {
    throw new Error(`${context} 缺少 DestinationKey 对象`);
  }
  const objectRef = value.Object;
  if (!isRecord(objectRef)) {
    throw new Error(`${context} 缺少 Object 对象`);
  }
  return {
    pointType: expectString(value, "PointType", context),
    destinationType: expectString(destKey, "Type", `${context}.DestinationKey`),
    destinationId: expectInteger(destKey, "DestinationId", `${context}.DestinationKey`),
    isPointToObject: expectBoolean(value, "IsPointToObject", context),
    objectType: expectString(objectRef, "Type", `${context}.Object`),
    objectId: expectInteger(objectRef, "ID", `${context}.Object`),
    condition: expectString(value, "Condition", context),
  };
}

/** Load Scenario/DT_ChapterData.json into typed Chapter records sorted by chapterNo. */
export async function loadChapters(exportRoot: string = exportDir()): Promise<readonly Chapter[]> {
  const path = join(datatableDir(exportRoot), "Scenario", "DT_ChapterData.json");
  const table = await loadDataTable(path);
  if (table.rowStruct !== "ChapterData") {
    throw new Error(`章节表 RowStruct 应为 ChapterData，收到 ${table.rowStruct}`);
  }
  const chapters: Chapter[] = [];
  for (const [rowKey, row] of table.rows) {
    const context = `DT_ChapterData 行 ${rowKey}`;
    chapters.push({
      rowKey,
      chapterNo: expectInteger(row, "ChapterNo", context),
      name: expectFText(row, "Name", context),
    });
  }
  chapters.sort((a, b) => a.chapterNo - b.chapterNo);
  return chapters;
}

/** Load Scenario/DT_MainScenario.json into typed MainScenarioEntry records. */
export async function loadMainScenario(
  exportRoot: string = exportDir(),
): Promise<readonly MainScenarioEntry[]> {
  const path = join(datatableDir(exportRoot), "Scenario", "DT_MainScenario.json");
  const table = await loadDataTable(path);
  if (table.rowStruct !== "ScenarioSection") {
    throw new Error(`主线剧情表 RowStruct 应为 ScenarioSection，收到 ${table.rowStruct}`);
  }
  const entries: MainScenarioEntry[] = [];
  for (const [rowKey, row] of table.rows) {
    const context = `DT_MainScenario 行 ${rowKey}`;
    const condition = expectString(row, "Condition", context);
    const minProgress = parseMainProgressCondition(condition);
    const chapterNo = minProgress === null ? null : chapterNoFromProgress(minProgress);
    const rawDestinations = row.DestinationList;
    if (!Array.isArray(rawDestinations)) {
      throw new Error(`${context} 的 DestinationList 字段不是数组`);
    }
    const destinations = rawDestinations.map((item, index) =>
      parseDestination(item, `${context}.DestinationList[${index}]`),
    );
    entries.push({
      rowKey,
      condition,
      minProgress,
      chapterNo,
      text: expectFText(row, "Text", context),
      summaryText: expectFText(row, "SummaryText", context),
      destinations,
    });
  }
  return entries;
}

/** Load both Scenario tables together with a shared FText resolver. */
export async function loadScenarioData(
  exportRoot: string = exportDir(),
  resolverOptions: Omit<TextResolverOptions, "exportRoot"> = {},
): Promise<ScenarioData> {
  const [chapters, mainScenario] = await Promise.all([
    loadChapters(exportRoot),
    loadMainScenario(exportRoot),
  ]);
  const textResolver = createTextResolver({ ...resolverOptions, exportRoot });
  return { chapters, mainScenario, textResolver };
}

/** Find chapter N in the loaded chapter list, or null if no such chapter exists. */
export function findChapter(chapters: readonly Chapter[], chapterNo: number): Chapter | null {
  return chapters.find((chapter) => chapter.chapterNo === chapterNo) ?? null;
}

/** Resolve the display title of chapter N in the requested language (default resolver.language, "zh-Hans"). */
export async function getChapterName(
  data: ScenarioData,
  chapterNo: number,
  language: string = data.textResolver.language,
): Promise<ResolvedText> {
  const chapter = findChapter(data.chapters, chapterNo);
  if (chapter === null) {
    return {
      text: null,
      lang: null,
      requestedLanguage: language,
      reason: `章节表中找不到第 ${chapterNo} 章`,
      source: "missing",
    };
  }
  return data.textResolver.resolve(chapter.name, language);
}

/** Return all main scenario entries belonging to chapter N, sorted by minProgress ascending. */
export function getChapterMainScenario(
  entries: readonly MainScenarioEntry[],
  chapterNo: number,
): readonly MainScenarioEntry[] {
  return entries
    .filter((entry) => entry.chapterNo === chapterNo)
    .sort((a, b) => (a.minProgress ?? 0) - (b.minProgress ?? 0));
}

/** Resolve both the objective text and summary text of one main scenario entry (default resolver.language, "zh-Hans"). */
export async function resolveMainScenarioTexts(
  data: ScenarioData,
  entry: MainScenarioEntry,
  language: string = data.textResolver.language,
): Promise<ResolvedMainScenarioTexts> {
  const [text, summaryText] = await Promise.all([
    data.textResolver.resolve(entry.text, language),
    data.textResolver.resolve(entry.summaryText, language),
  ]);
  return { text, summaryText };
}
