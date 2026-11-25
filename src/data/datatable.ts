/**
 * Generic readers for CUE4Parse DataTable, CompositeDataTable and StringTable JSON exports
 * (docs/plan.md §3 R2). External JSON enters as unknown and is narrowed with Chinese error
 * messages that name the file and failing field.
 */

import { readFile } from "node:fs/promises";
import { describeError, isRecord } from "../shared/index.ts";

export type DataTableType = "DataTable" | "CompositeDataTable";

export interface DataTable {
  readonly type: DataTableType;
  readonly name: string;
  readonly packagePath: string;
  readonly rowStruct: string;
  readonly rows: Map<string, Record<string, unknown>>;
}

export interface StringTable {
  readonly name: string;
  readonly packagePath: string;
  readonly tableNamespace: string;
  readonly entries: Map<string, string>;
}

const QUOTED_STRUCT_NAME = /^[A-Za-z0-9_]+'([^']+)'$/u;

/** Extract "ChapterData" from CUE4Parse's "Class'ChapterData'" representation. */
export function normalizeRowStructName(objectName: string): string {
  const match = QUOTED_STRUCT_NAME.exec(objectName);
  return match?.[1] ?? objectName;
}

function firstExportObject(document: unknown, sourceLabel: string): Record<string, unknown> {
  if (!Array.isArray(document) || document.length === 0) {
    throw new Error(`导出文件 ${sourceLabel} 不是非空 JSON 数组`);
  }
  const first: unknown = document[0];
  if (!isRecord(first)) {
    throw new Error(`导出文件 ${sourceLabel} 的首项不是对象`);
  }
  return first;
}

function requiredString(
  object: Record<string, unknown>,
  field: string,
  sourceLabel: string,
): string {
  const value = object[field];
  if (typeof value !== "string" || value === "") {
    throw new Error(`导出文件 ${sourceLabel} 缺少非空字符串字段 ${field}`);
  }
  return value;
}

/** Narrow a parsed CUE4Parse JSON document into a generic DataTable (or CompositeDataTable). */
export function parseDataTableDocument(document: unknown, sourceLabel: string): DataTable {
  const entry = firstExportObject(document, sourceLabel);
  const type = entry.Type;
  if (type !== "DataTable" && type !== "CompositeDataTable") {
    throw new Error(
      `导出文件 ${sourceLabel} 的 Type 是 ${String(type)}，不是 DataTable 或 CompositeDataTable`,
    );
  }
  const name = requiredString(entry, "Name", sourceLabel);
  const packagePath = requiredString(entry, "Package", sourceLabel);
  const properties = entry.Properties;
  if (!isRecord(properties)) {
    throw new Error(`数据表 ${sourceLabel} 缺少 Properties 对象`);
  }
  const rowStructObj = properties.RowStruct;
  if (!isRecord(rowStructObj) || typeof rowStructObj.ObjectName !== "string") {
    throw new Error(`数据表 ${sourceLabel} 缺少 Properties.RowStruct.ObjectName`);
  }
  const rawRows = entry.Rows;
  if (!isRecord(rawRows)) {
    throw new Error(`数据表 ${sourceLabel} 缺少 Rows 对象`);
  }
  const rows = new Map<string, Record<string, unknown>>();
  for (const [rowKey, rowValue] of Object.entries(rawRows)) {
    if (!isRecord(rowValue)) {
      throw new Error(`数据表 ${sourceLabel} 的行 ${rowKey} 不是对象`);
    }
    rows.set(rowKey, rowValue);
  }
  return {
    type,
    name,
    packagePath,
    rowStruct: normalizeRowStructName(rowStructObj.ObjectName),
    rows,
  };
}

/** Load one CUE4Parse DataTable or CompositeDataTable JSON file from disk. */
export async function loadDataTable(filePath: string): Promise<DataTable> {
  let rawText: string;
  try {
    rawText = await readFile(filePath, "utf8");
  } catch (error) {
    throw new Error(`读取数据表失败（${filePath}）：${describeError(error)}`, { cause: error });
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(rawText) as unknown;
  } catch (error) {
    throw new Error(`数据表 JSON 格式错误（${filePath}）：${describeError(error)}`, {
      cause: error,
    });
  }
  return parseDataTableDocument(parsed, filePath);
}

/** Narrow a parsed CUE4Parse JSON document into a StringTable. */
export function parseStringTableDocument(document: unknown, sourceLabel: string): StringTable {
  const entry = firstExportObject(document, sourceLabel);
  if (entry.Type !== "StringTable") {
    throw new Error(`导出文件 ${sourceLabel} 的 Type 是 ${String(entry.Type)}，不是 StringTable`);
  }
  const name = requiredString(entry, "Name", sourceLabel);
  const packagePath = requiredString(entry, "Package", sourceLabel);
  const tableBody = entry.StringTable;
  if (!isRecord(tableBody)) {
    throw new Error(`字符串表 ${sourceLabel} 缺少 StringTable 对象`);
  }
  const tableNamespace = requiredString(tableBody, "TableNamespace", sourceLabel);
  const rawEntries = tableBody.KeysToEntries;
  if (!isRecord(rawEntries)) {
    throw new Error(`字符串表 ${sourceLabel} 缺少 StringTable.KeysToEntries 对象`);
  }
  const entries = new Map<string, string>();
  for (const [key, value] of Object.entries(rawEntries)) {
    if (typeof value !== "string") {
      throw new Error(`字符串表 ${sourceLabel} 的键 ${key} 对应的值不是字符串`);
    }
    entries.set(key, value);
  }
  return { name, packagePath, tableNamespace, entries };
}

/** Load one CUE4Parse StringTable JSON file from disk. */
export async function loadStringTable(filePath: string): Promise<StringTable> {
  let rawText: string;
  try {
    rawText = await readFile(filePath, "utf8");
  } catch (error) {
    throw new Error(`读取字符串表失败（${filePath}）：${describeError(error)}`, { cause: error });
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(rawText) as unknown;
  } catch (error) {
    throw new Error(`字符串表 JSON 格式错误（${filePath}）：${describeError(error)}`, {
      cause: error,
    });
  }
  return parseStringTableDocument(parsed, filePath);
}
