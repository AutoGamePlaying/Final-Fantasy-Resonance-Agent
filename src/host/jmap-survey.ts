/**
 * Survey `/Script/mucha` reflection entries (classes, structs, enums) and `/Game/` blueprint
 * generated classes from a UE4SS `.jmap` file, classify them into R1 state-map topics, and render
 * JSON and Markdown reports.
 */

import { readdir, stat } from "node:fs/promises";
import { basename, join } from "node:path";
import { isRecord, list, num, text } from "../shared/index.ts";
import {
  streamJmap,
  streamJmapFile,
  type JmapMetadata,
  type JmapStreamResult,
} from "./jmap-stream.ts";

export const MUCHA_PREFIX = "/Script/mucha.";
export const GAME_PREFIX = "/Game/";

export type TopicId =
  "party" | "battle" | "field" | "map" | "dialog" | "menu" | "shop" | "story" | "save" | "input";

export interface TopicDefinition {
  readonly id: TopicId;
  readonly title: string;
  readonly pattern: RegExp;
}

export const TOPICS: readonly TopicDefinition[] = [
  {
    id: "party",
    title: "队伍与角色能力值",
    pattern:
      /(?:Party|Chara|Character|UnitParam|StatusParam|Growth|Equip|Equipment|Job|Ability|SkillTree|Vision|Member|Hero|PlayerStatus|ParamData|LevelUp)/iu,
  },
  {
    id: "battle",
    title: "战斗（回合、行动顺序、指令菜单、目标）",
    pattern:
      /(?:Btl|Battle|Turn|Timeline|ActOrder|ActionOrder|Target|Enemy|Encount|Encounter|SkillAction|Break|Command|FinishCmd|Chain|Damage|Buff|Debuff|StatusEffect)/iu,
  },
  {
    id: "field",
    title: "野外（玩家角色、坐标、朝向、移动方式）",
    pattern:
      /(?:Field|Fld|PlayerController|PlayerPawn|PlayerChara|Move|Movement|Walk|Run|Dash|Interact|Gimmick|Treasure|Chest|Npc|Camera|Symbol|Follow)/iu,
  },
  {
    id: "map",
    title: "地图和场景切换",
    pattern:
      /(?:Map|Level|Area|Zone|Stage|World|Transition|Travel|Warp|Teleport|FastTravel|Portal|EntryPoint|Scene|LoadLevel|SubLevel)/iu,
  },
  {
    id: "dialog",
    title: "对话和选项",
    pattern:
      /(?:Dialog|Dialogue|Talk|Msg|Message|Choice|Select|Subtitle|Speech|Caption|Balloon|Conversation|EventText|TextWindow)/iu,
  },
  {
    id: "menu",
    title: "菜单和光标",
    pattern:
      /(?:Menu|Cursor|Focus|Tab|ListView|ScrollBox|Popup|Modal|CommonMenu|MainMenu|PauseMenu|Camp|Inventory|ItemMenu|WidgetManager|Hud|Title)/iu,
  },
  {
    id: "shop",
    title: "商店",
    pattern: /(?:Shop|Store|Vendor|Merchant|Trade|Buy|Sell|Price|GuildShop)/iu,
  },
  {
    id: "story",
    title: "剧情进度和旗标",
    pattern:
      /(?:Story|Chapter|Quest|Scenario|Flag|Progress|Phase|Event|Cutscene|Sequence|Mission|Objective|Episode|MainQuest|SubQuest)/iu,
  },
  {
    id: "save",
    title: "存档和读档",
    pattern: /(?:Save|Load|Slot|AutoSave|SystemData|SaveData|SaveGame|Checkpoint|Resume)/iu,
  },
  {
    id: "input",
    title: "输入（Enhanced Input 映射上下文和动作）",
    pattern:
      /(?:Input|IMC|KeyBind|KeyConfig|ControllerData|ActionGuide|KeyGuide|Gamepad|ButtonPath|EnhancedInput|Registrant)/iu,
  },
];

export interface SurveyProperty {
  readonly name: string;
  readonly type: string;
  readonly offset: number;
}

export interface BlueprintProperty {
  readonly name: string;
  readonly type: string;
}

export interface MuchaClassEntry {
  readonly path: string;
  readonly name: string;
  readonly superClass: string | null;
  readonly properties: readonly SurveyProperty[];
  readonly functions: readonly string[];
  readonly topics: readonly TopicId[];
}

export interface MuchaStructEntry {
  readonly path: string;
  readonly name: string;
  readonly superStruct: string | null;
  readonly properties: readonly SurveyProperty[];
  readonly topics: readonly TopicId[];
}

export interface MuchaEnumValue {
  readonly name: string;
  readonly value: number;
}

export interface MuchaEnumEntry {
  readonly path: string;
  readonly name: string;
  readonly cppType: string | null;
  readonly values: readonly MuchaEnumValue[];
  readonly topics: readonly TopicId[];
}

export interface BlueprintClassEntry {
  readonly path: string;
  readonly name: string;
  readonly generatedBy: string;
  readonly superClass: string | null;
  readonly properties: readonly BlueprintProperty[];
  readonly topics: readonly TopicId[];
}

export interface TopicBucket {
  readonly id: TopicId;
  readonly title: string;
  readonly muchaClasses: readonly string[];
  readonly muchaStructs: readonly string[];
  readonly muchaEnums: readonly string[];
  readonly blueprints: readonly string[];
}

export interface SurveyStats {
  readonly totalObjects: number;
  readonly parsedObjects: number;
  readonly skippedObjects: number;
  readonly muchaClassesTotal: number;
  readonly muchaStructsTotal: number;
  readonly muchaEnumsTotal: number;
  readonly blueprintClassesTotal: number;
  readonly muchaClasses: number;
  readonly muchaStructs: number;
  readonly muchaEnums: number;
  readonly blueprintClasses: number;
}

export interface JmapSurveyReport {
  readonly sourceFile: string | null;
  readonly filter: string | null;
  readonly metadata: JmapMetadata | null;
  readonly stats: SurveyStats;
  readonly mucha: {
    readonly classes: readonly MuchaClassEntry[];
    readonly structs: readonly MuchaStructEntry[];
    readonly enums: readonly MuchaEnumEntry[];
  };
  readonly blueprints: readonly BlueprintClassEntry[];
  readonly topics: readonly TopicBucket[];
}

export interface SurveyOptions {
  readonly filter?: RegExp;
  readonly sourceFile?: string | null;
}

const SIMPLE_TYPES: Record<string, string> = {
  BoolProperty: "bool",
  ByteProperty: "uint8",
  DoubleProperty: "double",
  FieldPathProperty: "FFieldPath",
  FloatProperty: "float",
  FUtf8StrProperty: "FUtf8String",
  Int16Property: "int16",
  Int64Property: "int64",
  Int8Property: "int8",
  IntProperty: "int32",
  NameProperty: "FName",
  StrProperty: "FString",
  TextProperty: "FText",
  UInt16Property: "uint16",
  UInt32Property: "uint32",
  UInt64Property: "uint64",
};

/** Extract the trailing object or member name from a UE object path. */
export function shortObjectName(objectPath: string): string {
  const colonIndex = objectPath.lastIndexOf(":");
  if (colonIndex >= 0) return objectPath.slice(colonIndex + 1);
  const dotIndex = objectPath.lastIndexOf(".");
  if (dotIndex >= 0) return objectPath.slice(dotIndex + 1);
  const slashIndex = objectPath.lastIndexOf("/");
  if (slashIndex >= 0) return objectPath.slice(slashIndex + 1);
  return objectPath;
}

/** Format a UE4SS jmap property descriptor into a compact readable type string. */
export function formatPropertyType(raw: unknown): string {
  if (!isRecord(raw)) return "unknown";
  const propType = text(raw.type) ?? "unknown";
  let formatted: string;

  switch (propType) {
    case "ByteProperty": {
      const enumPath = text(raw.enum);
      formatted = enumPath ? `TEnumAsByte<${shortObjectName(enumPath)}>` : "uint8";
      break;
    }
    case "EnumProperty": {
      const enumPath = text(raw.enum);
      formatted = enumPath ? shortObjectName(enumPath) : "Enum";
      break;
    }
    case "StructProperty": {
      const structPath = text(raw.struct);
      formatted = structPath ? shortObjectName(structPath) : "Struct";
      break;
    }
    case "ObjectProperty": {
      const cls = text(raw.property_class);
      formatted = cls ? `${shortObjectName(cls)}*` : "UObject*";
      break;
    }
    case "WeakObjectProperty": {
      const cls = text(raw.property_class);
      formatted = `TWeakObjectPtr<${cls ? shortObjectName(cls) : "UObject"}>`;
      break;
    }
    case "SoftObjectProperty": {
      const cls = text(raw.property_class);
      formatted = `TSoftObjectPtr<${cls ? shortObjectName(cls) : "UObject"}>`;
      break;
    }
    case "LazyObjectProperty": {
      const cls = text(raw.property_class);
      formatted = `TLazyObjectPtr<${cls ? shortObjectName(cls) : "UObject"}>`;
      break;
    }
    case "ClassProperty": {
      const target = text(raw.meta_class) ?? text(raw.property_class);
      formatted = `TSubclassOf<${target ? shortObjectName(target) : "UObject"}>`;
      break;
    }
    case "SoftClassProperty": {
      const target = text(raw.meta_class) ?? text(raw.property_class);
      formatted = `TSoftClassPtr<${target ? shortObjectName(target) : "UObject"}>`;
      break;
    }
    case "InterfaceProperty": {
      const iface = text(raw.interface_class);
      formatted = `TScriptInterface<${iface ? shortObjectName(iface) : "IInterface"}>`;
      break;
    }
    case "ArrayProperty": {
      formatted = `TArray<${formatPropertyType(raw.inner)}>`;
      break;
    }
    case "SetProperty": {
      formatted = `TSet<${formatPropertyType(raw.key_prop)}>`;
      break;
    }
    case "MapProperty": {
      formatted = `TMap<${formatPropertyType(raw.key_prop)}, ${formatPropertyType(raw.value_prop)}>`;
      break;
    }
    case "OptionalProperty": {
      formatted = `TOptional<${formatPropertyType(raw.inner)}>`;
      break;
    }
    case "DelegateProperty": {
      const sig = text(raw.signature_function);
      formatted = sig ? `Delegate<${shortObjectName(sig)}>` : "Delegate";
      break;
    }
    case "MulticastInlineDelegateProperty":
    case "MulticastSparseDelegateProperty": {
      const sig = text(raw.signature_function);
      formatted = sig ? `MulticastDelegate<${shortObjectName(sig)}>` : "MulticastDelegate";
      break;
    }
    default: {
      formatted = SIMPLE_TYPES[propType] ?? propType;
      break;
    }
  }

  const arrayDim = num(raw.array_dim);
  if (arrayDim !== undefined && arrayDim > 1) {
    return `${formatted}[${arrayDim}]`;
  }
  return formatted;
}

/** Classify a reflection entry into zero or more R1 state-map topics. */
export function classifyTopics(
  name: string,
  objectPath: string,
  superPath: string | null = null,
): TopicId[] {
  const haystack = `${name} ${objectPath} ${superPath ? shortObjectName(superPath) : ""}`;
  const matched: TopicId[] = [];
  for (const topic of TOPICS) {
    if (topic.pattern.test(haystack)) matched.push(topic.id);
  }
  return matched;
}

/** Compile a user-provided `--filter` regular expression (case-insensitive by default). */
export function compileFilter(pattern: string): RegExp {
  if (pattern.length === 0) {
    throw new Error("--filter 正则不能为空");
  }
  try {
    return new RegExp(pattern, "iu");
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new Error(`--filter 正则无效(${pattern}):${detail}`, { cause: error });
  }
}

function parseSurveyProperties(raw: unknown): SurveyProperty[] {
  const result: SurveyProperty[] = [];
  for (const entry of list(raw)) {
    if (!isRecord(entry)) continue;
    const name = text(entry.name);
    const offset = num(entry.offset);
    if (name === undefined || offset === undefined) continue;
    result.push({ name, type: formatPropertyType(entry), offset });
  }
  result.sort((a, b) => a.offset - b.offset || a.name.localeCompare(b.name));
  return result;
}

function parseBlueprintProperties(raw: unknown): BlueprintProperty[] {
  const result: BlueprintProperty[] = [];
  for (const entry of list(raw)) {
    if (!isRecord(entry)) continue;
    const name = text(entry.name);
    if (name === undefined) continue;
    result.push({ name, type: formatPropertyType(entry) });
  }
  return result;
}

function parseEnumValues(raw: unknown): MuchaEnumValue[] {
  const result: MuchaEnumValue[] = [];
  for (const entry of list(raw)) {
    if (!Array.isArray(entry) || entry.length < 2) continue;
    const name = text(entry[0]);
    const value = num(entry[1]);
    if (name === undefined || value === undefined) continue;
    result.push({ name, value });
  }
  return result;
}

function isTargetPath(objectPath: string): boolean {
  if (objectPath.startsWith(MUCHA_PREFIX)) {
    return !objectPath.startsWith(`${MUCHA_PREFIX}Default__`);
  }
  if (objectPath.startsWith(GAME_PREFIX)) {
    if (objectPath.includes(":")) return false;
    const dotIndex = objectPath.lastIndexOf(".");
    if (dotIndex < 0) return false;
    return !objectPath.startsWith("Default__", dotIndex + 1);
  }
  return false;
}

function isBlueprintGeneratedClass(objectClass: string | undefined): boolean {
  return objectClass?.endsWith("BlueprintGeneratedClass") ?? false;
}

function matchesFilter(filter: RegExp | undefined, name: string, objectPath: string): boolean {
  if (!filter) return true;
  filter.lastIndex = 0;
  return filter.test(name) || filter.test(objectPath);
}

interface PendingClass {
  readonly path: string;
  readonly name: string;
  readonly superClass: string | null;
  readonly properties: readonly SurveyProperty[];
  readonly functions: Set<string>;
}

function buildSurveyReport(
  streamResult: JmapStreamResult,
  pendingClasses: Map<string, PendingClass>,
  structs: MuchaStructEntry[],
  enums: MuchaEnumEntry[],
  blueprints: BlueprintClassEntry[],
  functionsByOuter: Map<string, Set<string>>,
  options: SurveyOptions,
): JmapSurveyReport {
  const allClasses: MuchaClassEntry[] = [];
  for (const entry of pendingClasses.values()) {
    const extraFunctions = functionsByOuter.get(entry.path);
    if (extraFunctions) {
      for (const fn of extraFunctions) entry.functions.add(fn);
    }
    const functions = [...entry.functions].sort((a, b) => a.localeCompare(b));
    allClasses.push({
      path: entry.path,
      name: entry.name,
      superClass: entry.superClass,
      properties: entry.properties,
      functions,
      topics: classifyTopics(entry.name, entry.path, entry.superClass),
    });
  }

  allClasses.sort((a, b) => a.name.localeCompare(b.name));
  structs.sort((a, b) => a.name.localeCompare(b.name));
  enums.sort((a, b) => a.name.localeCompare(b.name));
  blueprints.sort((a, b) => a.path.localeCompare(b.path));

  const filter = options.filter;
  const filteredClasses = allClasses.filter((c) => matchesFilter(filter, c.name, c.path));
  const filteredStructs = structs.filter((s) => matchesFilter(filter, s.name, s.path));
  const filteredEnums = enums.filter((e) => matchesFilter(filter, e.name, e.path));
  const filteredBlueprints = blueprints.filter((b) => matchesFilter(filter, b.name, b.path));

  const topics: TopicBucket[] = TOPICS.map((topic) => ({
    id: topic.id,
    title: topic.title,
    muchaClasses: filteredClasses.filter((c) => c.topics.includes(topic.id)).map((c) => c.name),
    muchaStructs: filteredStructs.filter((s) => s.topics.includes(topic.id)).map((s) => s.name),
    muchaEnums: filteredEnums.filter((e) => e.topics.includes(topic.id)).map((e) => e.name),
    blueprints: filteredBlueprints.filter((b) => b.topics.includes(topic.id)).map((b) => b.name),
  }));

  return {
    sourceFile: options.sourceFile ?? null,
    filter: filter ? filter.source : null,
    metadata: streamResult.metadata,
    stats: {
      totalObjects: streamResult.totalObjects,
      parsedObjects: streamResult.parsedObjects,
      skippedObjects: streamResult.skippedObjects,
      muchaClassesTotal: allClasses.length,
      muchaStructsTotal: structs.length,
      muchaEnumsTotal: enums.length,
      blueprintClassesTotal: blueprints.length,
      muchaClasses: filteredClasses.length,
      muchaStructs: filteredStructs.length,
      muchaEnums: filteredEnums.length,
      blueprintClasses: filteredBlueprints.length,
    },
    mucha: {
      classes: filteredClasses,
      structs: filteredStructs,
      enums: filteredEnums,
    },
    blueprints: filteredBlueprints,
    topics,
  };
}

function createCollector() {
  const pendingClasses = new Map<string, PendingClass>();
  const structs: MuchaStructEntry[] = [];
  const enums: MuchaEnumEntry[] = [];
  const blueprints: BlueprintClassEntry[] = [];
  const functionsByOuter = new Map<string, Set<string>>();

  function onObject(objectPath: string, raw: unknown): void {
    if (!isRecord(raw)) return;
    const objectType = text(raw.type);
    if (objectPath.startsWith(MUCHA_PREFIX)) {
      if (objectType === "Class") {
        const name = shortObjectName(objectPath);
        const superClass = text(raw.super_struct) ?? null;
        const functions = new Set<string>();
        for (const child of list(raw.children)) {
          const childPath = text(child);
          if (childPath?.includes(":")) {
            functions.add(shortObjectName(childPath));
          }
        }
        pendingClasses.set(objectPath, {
          path: objectPath,
          name,
          superClass,
          properties: parseSurveyProperties(raw.properties),
          functions,
        });
      } else if (objectType === "Function") {
        const outer = text(raw.outer);
        if (outer?.startsWith(MUCHA_PREFIX)) {
          let set = functionsByOuter.get(outer);
          if (!set) {
            set = new Set<string>();
            functionsByOuter.set(outer, set);
          }
          set.add(shortObjectName(objectPath));
        }
      } else if (objectType === "ScriptStruct") {
        const name = shortObjectName(objectPath);
        const superStruct = text(raw.super_struct) ?? null;
        structs.push({
          path: objectPath,
          name,
          superStruct,
          properties: parseSurveyProperties(raw.properties),
          topics: classifyTopics(name, objectPath, superStruct),
        });
      } else if (objectType === "Enum") {
        const name = shortObjectName(objectPath);
        enums.push({
          path: objectPath,
          name,
          cppType: text(raw.cpp_type) ?? null,
          values: parseEnumValues(raw.names),
          topics: classifyTopics(name, objectPath),
        });
      }
      return;
    }

    if (objectPath.startsWith(GAME_PREFIX) && objectType === "Class") {
      const objectClass = text(raw.class);
      if (!isBlueprintGeneratedClass(objectClass)) return;
      const name = shortObjectName(objectPath);
      const superClass = text(raw.super_struct) ?? null;
      blueprints.push({
        path: objectPath,
        name,
        generatedBy: objectClass ? shortObjectName(objectClass) : "BlueprintGeneratedClass",
        superClass,
        properties: parseBlueprintProperties(raw.properties),
        topics: classifyTopics(name, objectPath, superClass),
      });
    }
  }

  return { pendingClasses, structs, enums, blueprints, functionsByOuter, onObject };
}

/** Survey a jmap from an async chunk iterable (used directly by tests). */
export async function surveyJmapStream(
  chunks: AsyncIterable<string>,
  options: SurveyOptions = {},
): Promise<JmapSurveyReport> {
  const collector = createCollector();
  const streamResult = await streamJmap(chunks, {
    shouldParseObject: isTargetPath,
    onObject: collector.onObject,
  });
  return buildSurveyReport(
    streamResult,
    collector.pendingClasses,
    collector.structs,
    collector.enums,
    collector.blueprints,
    collector.functionsByOuter,
    options,
  );
}

/** Survey a `.jmap` file on disk via streaming reads. */
export async function surveyJmapFile(
  filePath: string,
  options: SurveyOptions = {},
): Promise<JmapSurveyReport> {
  const collector = createCollector();
  const streamResult = await streamJmapFile(filePath, {
    shouldParseObject: isTargetPath,
    onObject: collector.onObject,
  });
  return buildSurveyReport(
    streamResult,
    collector.pendingClasses,
    collector.structs,
    collector.enums,
    collector.blueprints,
    collector.functionsByOuter,
    {
      ...options,
      sourceFile: options.sourceFile === undefined ? basename(filePath) : options.sourceFile,
    },
  );
}

/** Format a property offset as hex (`0x0040`). */
function formatOffset(offset: number): string {
  return `0x${offset.toString(16).toUpperCase().padStart(4, "0")}`;
}

/** Render a human-readable Markdown survey document. */
export function renderSurveyMarkdown(report: JmapSurveyReport): string {
  const lines: string[] = [];
  lines.push("# jmap 反射调研清单");
  lines.push("");
  lines.push(
    "> 本文件由 `src/host/cli/jmap-survey.ts` 从本地 `.jmap` 转储生成，仅放 `.deps/`，不进版本库。",
  );
  lines.push("");

  lines.push("## 概览");
  lines.push("");
  if (report.sourceFile) lines.push(`- 源文件：\`${report.sourceFile}\``);
  if (report.metadata?.source) lines.push(`- 来源可执行文件：\`${report.metadata.source}\``);
  if (report.metadata?.engineVersion) {
    const v = report.metadata.engineVersion;
    lines.push(`- 引擎版本：UE ${v.major}.${v.minor}`);
  }
  if (report.metadata?.timestamp) lines.push(`- 转储时间：${report.metadata.timestamp}`);
  if (report.filter) lines.push(`- 过滤正则：\`${report.filter}\``);
  lines.push(
    `- 对象总数：${report.stats.totalObjects}（解析 ${report.stats.parsedObjects}，跳过 ${report.stats.skippedObjects}）`,
  );
  lines.push(
    `- \`/Script/mucha\`：类 ${report.stats.muchaClasses}/${report.stats.muchaClassesTotal}，结构体 ${report.stats.muchaStructs}/${report.stats.muchaStructsTotal}，枚举 ${report.stats.muchaEnums}/${report.stats.muchaEnumsTotal}`,
  );
  lines.push(
    `- \`/Game/\` 蓝图生成类：${report.stats.blueprintClasses}/${report.stats.blueprintClassesTotal}`,
  );
  lines.push("");

  lines.push("## 按主题归类索引");
  lines.push("");
  lines.push("| 主题 | mucha 类 | mucha 结构体 | mucha 枚举 | 蓝图类 |");
  lines.push("| --- | ---: | ---: | ---: | ---: |");
  for (const bucket of report.topics) {
    lines.push(
      `| ${bucket.title} (\`${bucket.id}\`) | ${bucket.muchaClasses.length} | ${bucket.muchaStructs.length} | ${bucket.muchaEnums.length} | ${bucket.blueprints.length} |`,
    );
  }
  lines.push("");

  for (const bucket of report.topics) {
    const total =
      bucket.muchaClasses.length +
      bucket.muchaStructs.length +
      bucket.muchaEnums.length +
      bucket.blueprints.length;
    if (total === 0) continue;
    lines.push(`### ${bucket.title} (\`${bucket.id}\`)`);
    lines.push("");
    if (bucket.muchaClasses.length > 0) {
      lines.push(
        `- **mucha 类（${bucket.muchaClasses.length}）**：${bucket.muchaClasses.join(", ")}`,
      );
    }
    if (bucket.muchaStructs.length > 0) {
      lines.push(
        `- **mucha 结构体（${bucket.muchaStructs.length}）**：${bucket.muchaStructs.join(", ")}`,
      );
    }
    if (bucket.muchaEnums.length > 0) {
      lines.push(
        `- **mucha 枚举（${bucket.muchaEnums.length}）**：${bucket.muchaEnums.join(", ")}`,
      );
    }
    if (bucket.blueprints.length > 0) {
      lines.push(`- **蓝图类（${bucket.blueprints.length}）**：${bucket.blueprints.join(", ")}`);
    }
    lines.push("");
  }

  lines.push(`## /Script/mucha 类（${report.mucha.classes.length}）`);
  lines.push("");
  for (const cls of report.mucha.classes) {
    lines.push(`### ${cls.name}`);
    lines.push("");
    lines.push(`- 路径：\`${cls.path}\``);
    lines.push(`- 父类：${cls.superClass ? `\`${cls.superClass}\`` : "无"}`);
    if (cls.topics.length > 0) lines.push(`- 主题：${cls.topics.join(", ")}`);
    if (cls.properties.length > 0) {
      lines.push("- 属性：");
      for (const prop of cls.properties) {
        lines.push(`  - \`${formatOffset(prop.offset)}\` \`${prop.name}\`: \`${prop.type}\``);
      }
    } else {
      lines.push("- 属性：无");
    }
    if (cls.functions.length > 0) {
      lines.push(
        `- 函数（${cls.functions.length}）：${cls.functions.map((f) => `\`${f}\``).join(", ")}`,
      );
    } else {
      lines.push("- 函数：无");
    }
    lines.push("");
  }

  lines.push(`## /Script/mucha 结构体（${report.mucha.structs.length}）`);
  lines.push("");
  for (const struct of report.mucha.structs) {
    lines.push(`### ${struct.name}`);
    lines.push("");
    lines.push(`- 路径：\`${struct.path}\``);
    lines.push(`- 父类：${struct.superStruct ? `\`${struct.superStruct}\`` : "无"}`);
    if (struct.topics.length > 0) lines.push(`- 主题：${struct.topics.join(", ")}`);
    if (struct.properties.length > 0) {
      lines.push("- 属性：");
      for (const prop of struct.properties) {
        lines.push(`  - \`${formatOffset(prop.offset)}\` \`${prop.name}\`: \`${prop.type}\``);
      }
    } else {
      lines.push("- 属性：无");
    }
    lines.push("");
  }

  lines.push(`## /Script/mucha 枚举（${report.mucha.enums.length}）`);
  lines.push("");
  for (const item of report.mucha.enums) {
    lines.push(`### ${item.name}`);
    lines.push("");
    lines.push(`- 路径：\`${item.path}\``);
    if (item.topics.length > 0) lines.push(`- 主题：${item.topics.join(", ")}`);
    if (item.values.length > 0) {
      lines.push(
        `- 枚举值（${item.values.length}）：${item.values.map((v) => `\`${v.name}=${v.value}\``).join(", ")}`,
      );
    } else {
      lines.push("- 枚举值：无");
    }
    lines.push("");
  }

  lines.push(`## /Game/ 蓝图生成类（${report.blueprints.length}）`);
  lines.push("");
  for (const bp of report.blueprints) {
    lines.push(`### ${bp.name}`);
    lines.push("");
    lines.push(`- 路径：\`${bp.path}\``);
    lines.push(`- 父类：${bp.superClass ? `\`${bp.superClass}\`` : "无"}`);
    if (bp.topics.length > 0) lines.push(`- 主题：${bp.topics.join(", ")}`);
    if (bp.properties.length > 0) {
      lines.push("- 属性：");
      for (const prop of bp.properties) {
        lines.push(`  - \`${prop.name}\`: \`${prop.type}\``);
      }
    } else {
      lines.push("- 属性：无");
    }
    lines.push("");
  }

  return `${lines.join("\n")}\n`;
}

/** Find the newest `.jmap` file under `<deps>/r1/` when the caller did not pass a path. */
export async function findLatestJmap(r1Root: string): Promise<string> {
  const candidates: { path: string; mtimeMs: number }[] = [];

  async function walk(directory: string, depth: number): Promise<void> {
    if (depth > 3) return;
    let entries;
    try {
      entries = await readdir(directory, { withFileTypes: true });
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
      throw error;
    }
    for (const entry of entries) {
      if (entry.name === "survey") continue;
      const fullPath = join(directory, entry.name);
      if (entry.isDirectory()) {
        await walk(fullPath, depth + 1);
      } else if (entry.isFile() && entry.name.endsWith(".jmap")) {
        const info = await stat(fullPath);
        candidates.push({ path: fullPath, mtimeMs: info.mtimeMs });
      }
    }
  }

  await walk(r1Root, 0);
  if (candidates.length === 0) {
    throw new Error(`在 ${r1Root} 下没找到 .jmap 文件，请显式传入路径或先用 dump-run jmap 拉取`);
  }
  candidates.sort((a, b) => b.mtimeMs - a.mtimeMs || a.path.localeCompare(b.path));
  const latest = candidates[0];
  if (!latest) {
    throw new Error(`在 ${r1Root} 下没找到 .jmap 文件`);
  }
  return latest.path;
}
