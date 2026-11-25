/**
 * Read-only survey over an exported CUE4Parse Datatable tree. Summarizes categories, per-table
 * RowStruct/row counts/inferred field types, StringTable & FText linkage, available languages,
 * and cross-row reference field names without ever printing raw game text or row content.
 */

import { readdir, readFile } from "node:fs/promises";
import { join, relative, sep } from "node:path";
import { describeError, isMissingFile, isRecord } from "../shared/index.ts";
import { normalizeRowStructName } from "./datatable.ts";
import { parseFText } from "./ftext.ts";
import { datatableDir, exportDir, localizationDir } from "./paths.ts";

export interface FieldSummary {
  readonly name: string;
  readonly types: readonly string[];
}

export interface TableSummary {
  readonly relativePath: string;
  readonly category: string;
  readonly type: string;
  readonly rowStruct: string | null;
  readonly rowCount: number;
  readonly fields: readonly FieldSummary[];
}

export interface CategorySummary {
  readonly name: string;
  readonly fileCount: number;
  readonly dataTableCount: number;
  readonly compositeDataTableCount: number;
  readonly stringTableCount: number;
  readonly otherAssetCount: number;
  readonly totalRows: number;
}

export interface ScriptBreakdown {
  readonly totalEntries: number;
  readonly withKana: number;
  readonly cjkWithoutKana: number;
  readonly asciiOrSymbol: number;
  readonly empty: number;
}

export interface StringSurvey {
  readonly stringTableCount: number;
  readonly totalEntries: number;
  readonly namespaceMatchesNameCount: number;
  readonly subdirs: Readonly<Record<string, number>>;
  readonly ftextShapes: Readonly<Record<string, number>>;
  readonly referencedTableIdCount: number;
  readonly missingTableIds: readonly string[];
  readonly missingKeyCount: number;
  readonly scriptBreakdown: ScriptBreakdown;
  readonly exportedLocalizationLanguages: readonly string[];
}

export interface ReferenceFieldSummary {
  readonly fieldName: string;
  readonly tableCount: number;
  readonly categories: readonly string[];
}

export interface ExportSurveyReport {
  readonly exportRoot: string;
  readonly totalFiles: number;
  readonly objectTypes: Readonly<Record<string, number>>;
  readonly categories: readonly CategorySummary[];
  readonly tables: readonly TableSummary[];
  readonly strings: StringSurvey;
  readonly referenceFields: readonly ReferenceFieldSummary[];
}

export interface SurveyOptions {
  readonly exportRoot?: string;
  readonly sampleRows?: number;
  readonly category?: string;
}

export interface FormatSurveyOptions {
  readonly includeTables?: boolean;
}

const REFERENCE_FIELD_PATTERN =
  /(?:^|_)(?:id|ids)$|(?:Id|ID|IdList|IDList|Ids|IDs)$|FlagList$|ProgressList$|DataTable$|^Condition$/u;

const KANA_PATTERN = /[\u3040-\u30ff]/u;
const CJK_PATTERN = /[\u4e00-\u9fff]/u;

async function collectJsonFiles(directory: string): Promise<string[]> {
  let entries;
  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if (isMissingFile(error)) {
      throw new Error(`找不到数据表导出目录：${directory}`, { cause: error });
    }
    throw new Error(`读取目录失败（${directory}）：${describeError(error)}`, { cause: error });
  }
  const files: string[] = [];
  for (const entry of entries) {
    const fullPath = join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await collectJsonFiles(fullPath)));
    } else if (entry.name.endsWith(".json")) {
      files.push(fullPath);
    }
  }
  files.sort((a, b) => a.localeCompare(b));
  return files;
}

async function detectLocalizationLanguages(root: string): Promise<string[]> {
  try {
    const entries = await readdir(localizationDir(root), { withFileTypes: true });
    return entries
      .filter(
        (entry) => entry.isFile() && entry.name.endsWith(".json") && entry.name !== "manifest.json",
      )
      .map((entry) => entry.name.slice(0, -".json".length))
      .sort((a, b) => a.localeCompare(b));
  } catch (error) {
    if (isMissingFile(error)) return [];
    throw new Error(`检查本地化目录失败：${describeError(error)}`, { cause: error });
  }
}

function inferValueType(value: unknown): string {
  if (value === null) return "null";
  if (Array.isArray(value)) {
    if (value.length === 0) return "array<empty>";
    const itemTypes = new Set<string>();
    for (const item of value.slice(0, 3)) {
      itemTypes.add(inferValueType(item));
    }
    return `array<${[...itemTypes].sort().join("|")}>`;
  }
  if (isRecord(value)) {
    if (parseFText(value) !== null) return "FText";
    if (typeof value.AssetPathName === "string") return "SoftObjectPath";
    if (typeof value.ObjectName === "string" && typeof value.ObjectPath === "string") {
      return "ObjectRef";
    }
    if (Object.hasOwn(value, "DataTable") && typeof value.RowName === "string") {
      return "DataTableRowHandle";
    }
    const keys = Object.keys(value);
    const preview = keys.slice(0, 4).join(",");
    return keys.length > 4 ? `struct{${preview},...}` : `struct{${preview}}`;
  }
  return typeof value;
}

function inferTableFields(
  rows: Record<string, unknown>,
  sampleRows: number,
): readonly FieldSummary[] {
  const fieldTypes = new Map<string, Set<string>>();
  let inspected = 0;
  for (const rowValue of Object.values(rows)) {
    if (!isRecord(rowValue)) continue;
    for (const [fieldName, fieldValue] of Object.entries(rowValue)) {
      let types = fieldTypes.get(fieldName);
      if (types === undefined) {
        types = new Set<string>();
        fieldTypes.set(fieldName, types);
      }
      types.add(inferValueType(fieldValue));
    }
    inspected++;
    if (inspected >= sampleRows) break;
  }
  return [...fieldTypes.entries()].map(([name, types]) => ({
    name,
    types: [...types].sort(),
  }));
}

interface ReferenceTracker {
  readonly tables: Set<string>;
  readonly categories: Set<string>;
}

function collectReferenceFields(
  value: unknown,
  tablePath: string,
  category: string,
  tracker: Map<string, ReferenceTracker>,
  depth = 0,
): void {
  if (depth > 2) return;
  if (Array.isArray(value)) {
    for (const item of value.slice(0, 3)) {
      collectReferenceFields(item, tablePath, category, tracker, depth);
    }
    return;
  }
  if (!isRecord(value)) return;
  if (parseFText(value) !== null || typeof value.AssetPathName === "string") return;
  for (const [fieldName, fieldValue] of Object.entries(value)) {
    if (REFERENCE_FIELD_PATTERN.test(fieldName)) {
      let entry = tracker.get(fieldName);
      if (entry === undefined) {
        entry = { tables: new Set<string>(), categories: new Set<string>() };
        tracker.set(fieldName, entry);
      }
      entry.tables.add(tablePath);
      entry.categories.add(category);
    }
    collectReferenceFields(fieldValue, tablePath, category, tracker, depth + 1);
  }
}

interface FTextScanState {
  readonly shapes: Map<string, number>;
  readonly referencedTableIds: Set<string>;
  readonly tableRefs: { tableId: string; key: string }[];
}

function scanFTexts(value: unknown, state: FTextScanState): void {
  if (Array.isArray(value)) {
    for (const item of value) scanFTexts(item, state);
    return;
  }
  if (!isRecord(value)) return;
  const parsed = parseFText(value);
  if (parsed !== null) {
    const signature = Object.keys(value).sort().join(",");
    state.shapes.set(signature, (state.shapes.get(signature) ?? 0) + 1);
    if (parsed.kind === "string-table") {
      state.referencedTableIds.add(parsed.tableId);
      state.tableRefs.push({ tableId: parsed.tableId, key: parsed.key });
    }
    return;
  }
  for (const child of Object.values(value)) scanFTexts(child, state);
}

function categoryOf(relativePosixPath: string): string {
  const slash = relativePosixPath.indexOf("/");
  return slash >= 0 ? relativePosixPath.slice(0, slash) : "(root)";
}

function toRecordSorted(map: Map<string, number>): Record<string, number> {
  const entries = [...map.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  return Object.fromEntries(entries);
}

interface MutableCategory {
  fileCount: number;
  dataTableCount: number;
  compositeDataTableCount: number;
  stringTableCount: number;
  otherAssetCount: number;
  totalRows: number;
}

/** Run a complete read-only structural survey on an export directory. */
export async function surveyExport(options: SurveyOptions = {}): Promise<ExportSurveyReport> {
  const root = options.exportRoot ?? exportDir();
  const sampleRows = options.sampleRows ?? 5;
  const baseDir = datatableDir(root);
  const allFiles = await collectJsonFiles(baseDir);

  const objectTypes = new Map<string, number>();
  const categoriesMap = new Map<string, MutableCategory>();
  const tables: TableSummary[] = [];
  const referenceTracker = new Map<string, ReferenceTracker>();

  const stringTableKeys = new Map<string, Set<string>>();
  const stringSubdirs = new Map<string, number>();
  let stringTableCount = 0;
  let stringTotalEntries = 0;
  let namespaceMatchesNameCount = 0;
  let withKana = 0;
  let cjkWithoutKana = 0;
  let asciiOrSymbol = 0;
  let emptyStrings = 0;

  const ftextState: FTextScanState = {
    shapes: new Map<string, number>(),
    referencedTableIds: new Set<string>(),
    tableRefs: [],
  };

  for (const filePath of allFiles) {
    const relativePath = relative(baseDir, filePath).split(sep).join("/");
    const category = categoryOf(relativePath);
    if (options.category !== undefined && category !== options.category && category !== "Strings") {
      continue;
    }

    let raw: unknown;
    try {
      raw = JSON.parse(await readFile(filePath, "utf8")) as unknown;
    } catch (error) {
      throw new Error(`解析导出文件失败（${relativePath}）：${describeError(error)}`, {
        cause: error,
      });
    }
    if (!Array.isArray(raw) || raw.length === 0 || !isRecord(raw[0])) {
      throw new Error(`导出文件 ${relativePath} 不是合法的 CUE4Parse 导出数组`);
    }
    const entry = raw[0];
    const type = typeof entry.Type === "string" ? entry.Type : "(unknown)";

    if (type === "StringTable" && isRecord(entry.StringTable)) {
      const packagePath = typeof entry.Package === "string" ? entry.Package : "";
      const assetName = typeof entry.Name === "string" ? entry.Name : "";
      const tableNamespace =
        typeof entry.StringTable.TableNamespace === "string"
          ? entry.StringTable.TableNamespace
          : "";
      const rawEntries = isRecord(entry.StringTable.KeysToEntries)
        ? entry.StringTable.KeysToEntries
        : {};
      const keySet = new Set(Object.keys(rawEntries));
      if (packagePath !== "" && assetName !== "") {
        stringTableKeys.set(`${packagePath}.${assetName}`, keySet);
      }

      if (options.category === undefined || options.category === "Strings") {
        stringTableCount++;
        if (tableNamespace === assetName) namespaceMatchesNameCount++;
        const parts = relativePath.split("/");
        const sub = parts.length > 2 ? (parts[1] ?? "(root)") : "(root)";
        stringSubdirs.set(sub, (stringSubdirs.get(sub) ?? 0) + 1);
        for (const textValue of Object.values(rawEntries)) {
          stringTotalEntries++;
          if (typeof textValue !== "string" || textValue.trim() === "") {
            emptyStrings++;
          } else if (KANA_PATTERN.test(textValue)) {
            withKana++;
          } else if (CJK_PATTERN.test(textValue)) {
            cjkWithoutKana++;
          } else {
            asciiOrSymbol++;
          }
        }
      }
    }

    if (options.category !== undefined && category !== options.category) {
      continue;
    }

    objectTypes.set(type, (objectTypes.get(type) ?? 0) + 1);
    let catStats = categoriesMap.get(category);
    if (catStats === undefined) {
      catStats = {
        fileCount: 0,
        dataTableCount: 0,
        compositeDataTableCount: 0,
        stringTableCount: 0,
        otherAssetCount: 0,
        totalRows: 0,
      };
      categoriesMap.set(category, catStats);
    }
    catStats.fileCount++;

    if (type === "DataTable" || type === "CompositeDataTable") {
      if (type === "DataTable") catStats.dataTableCount++;
      else catStats.compositeDataTableCount++;
      const props = isRecord(entry.Properties) ? entry.Properties : {};
      const rowStructObj = isRecord(props.RowStruct) ? props.RowStruct : {};
      const rowStruct =
        typeof rowStructObj.ObjectName === "string"
          ? normalizeRowStructName(rowStructObj.ObjectName)
          : null;
      const rows = isRecord(entry.Rows) ? entry.Rows : {};
      const rowCount = Object.keys(rows).length;
      catStats.totalRows += rowCount;
      tables.push({
        relativePath,
        category,
        type,
        rowStruct,
        rowCount,
        fields: inferTableFields(rows, sampleRows),
      });
      scanFTexts(rows, ftextState);
      let sampled = 0;
      for (const rowValue of Object.values(rows)) {
        collectReferenceFields(rowValue, relativePath, category, referenceTracker);
        sampled++;
        if (sampled >= sampleRows) break;
      }
    } else if (type === "StringTable") {
      catStats.stringTableCount++;
      const rawEntries =
        isRecord(entry.StringTable) && isRecord(entry.StringTable.KeysToEntries)
          ? entry.StringTable.KeysToEntries
          : {};
      catStats.totalRows += Object.keys(rawEntries).length;
    } else {
      catStats.otherAssetCount++;
    }
  }

  const missingTableIds = new Set<string>();
  let missingKeyCount = 0;
  for (const ref of ftextState.tableRefs) {
    const keys = stringTableKeys.get(ref.tableId);
    if (keys === undefined) {
      missingTableIds.add(ref.tableId);
    } else if (!keys.has(ref.key)) {
      missingKeyCount++;
    }
  }

  const categories: CategorySummary[] = [...categoriesMap.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([name, stats]) => ({ name, ...stats }));

  const referenceFields: ReferenceFieldSummary[] = [...referenceTracker.entries()]
    .map(([fieldName, tracked]) => ({
      fieldName,
      tableCount: tracked.tables.size,
      categories: [...tracked.categories].sort(),
    }))
    .sort((a, b) => b.categories.length - a.categories.length || b.tableCount - a.tableCount);

  const exportedLocalizationLanguages = await detectLocalizationLanguages(root);

  return {
    exportRoot: root,
    totalFiles: categories.reduce((sum, item) => sum + item.fileCount, 0),
    objectTypes: toRecordSorted(objectTypes),
    categories,
    tables,
    strings: {
      stringTableCount,
      totalEntries: stringTotalEntries,
      namespaceMatchesNameCount,
      subdirs: toRecordSorted(stringSubdirs),
      ftextShapes: toRecordSorted(ftextState.shapes),
      referencedTableIdCount: ftextState.referencedTableIds.size,
      missingTableIds: [...missingTableIds].sort(),
      missingKeyCount,
      scriptBreakdown: {
        totalEntries: stringTotalEntries,
        withKana,
        cjkWithoutKana,
        asciiOrSymbol,
        empty: emptyStrings,
      },
      exportedLocalizationLanguages,
    },
    referenceFields,
  };
}

/** Render a human-readable Chinese report from a survey result. */
export function formatSurveyReport(
  report: ExportSurveyReport,
  options: FormatSurveyOptions = {},
): string {
  const includeTables = options.includeTables ?? true;
  const lines: string[] = [];

  lines.push(`# FFR 导出数据表调查报告`);
  lines.push(`文件总数：${report.totalFiles}`);
  lines.push(
    `对象类型：${Object.entries(report.objectTypes)
      .map(([type, count]) => `${type}=${count}`)
      .join("，")}`,
  );
  lines.push("");

  lines.push(`## 一、按分类统计（共 ${report.categories.length} 类）`);
  for (const cat of report.categories) {
    lines.push(
      `  ${cat.name}: 文件=${cat.fileCount}（DT=${cat.dataTableCount}, CDT=${cat.compositeDataTableCount}, ST=${cat.stringTableCount}, 其他=${cat.otherAssetCount}），总行数=${cat.totalRows}`,
    );
  }
  lines.push("");

  lines.push("## 二、字符串表与 FText 引用");
  const st = report.strings;
  lines.push(
    `  StringTable 数量：${st.stringTableCount}，总词条：${st.totalEntries}，TableNamespace 与资源名一致：${st.namespaceMatchesNameCount}/${st.stringTableCount}`,
  );
  lines.push(
    `  文字脚本分布：含假名(日文)=${st.scriptBreakdown.withKana}，纯汉字=${st.scriptBreakdown.cjkWithoutKana}，ASCII/符号=${st.scriptBreakdown.asciiOrSymbol}，空串=${st.scriptBreakdown.empty}`,
  );
  lines.push(
    `  导出目录内多语言本地化目录：${st.exportedLocalizationLanguages.length > 0 ? st.exportedLocalizationLanguages.join("、") : "无（当前导出仅含源语言日文 StringTable，不含中文）"}`,
  );
  lines.push(
    `  FText 形态：${
      Object.entries(st.ftextShapes)
        .map(([shape, count]) => `[${shape}]=${count}`)
        .join("；") || "无"
    }`,
  );
  lines.push(
    `  TableId/Key 解析：引用唯一 TableId=${st.referencedTableIdCount}，缺失 TableId=${st.missingTableIds.length}，缺失 Key=${st.missingKeyCount}`,
  );
  lines.push(
    `  StringTable 子目录分布：${
      Object.entries(st.subdirs)
        .map(([sub, count]) => `${sub}=${count}`)
        .join("，") || "无"
    }`,
  );
  lines.push("");

  lines.push("## 三、行间引用字段（跨分类或高频引用字段）");
  const multiCatRefs = report.referenceFields.filter((ref) => ref.categories.length >= 2);
  const shownRefs = multiCatRefs.length > 0 ? multiCatRefs : report.referenceFields.slice(0, 30);
  for (const ref of shownRefs) {
    lines.push(
      `  ${ref.fieldName}: 出现于 ${ref.tableCount} 张表，分类=[${ref.categories.join(", ")}]`,
    );
  }
  lines.push("");

  if (includeTables) {
    lines.push(`## 四、各表结构（共 ${report.tables.length} 张 DataTable / CompositeDataTable）`);
    for (const table of report.tables) {
      const fieldsText = table.fields
        .map((field) => `${field.name}:${field.types.join("|")}`)
        .join(", ");
      lines.push(
        `  ${table.relativePath} [${table.type}] struct=${table.rowStruct ?? "(无)"} rows=${table.rowCount} | ${fieldsText}`,
      );
    }
  }

  return `${lines.join("\n")}\n`;
}
