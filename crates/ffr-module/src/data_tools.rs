//! Data-category (`ToolCategory::Data`) tool descriptors and handlers for `ffr-module`.
//!
//! Every successful query returns two `Content` items:
//! 1. A concise Chinese summary text (`Content::Text`) for model comprehension.
//! 2. Structured JSON (`Content::Json`) with numeric IDs and key fields for exact citations.

use std::collections::BTreeSet;

use agp_game::{Content, ToolCategory, ToolDescriptor, ToolOutput};
use agp_platform::FileChannel;
use ffr_data::battle::{
    AvailableLocation, BattleData, Element, EnemyProfile, Resistance, ResolvedBattleGroup, Skill,
    SkillAttributeType, SkillCostType, SkillDamageType, SkillTargetType, TargetRelation,
    TargetState, Unit, UnitCategory,
};
use ffr_data::world::{
    Item, ItemCategory, MapInfo, Quest, Shop, WorldData, item_summary, map_summary, quest_summary,
    shop_summary,
};
use ffr_data::{
    FTextValue, MODEL_LANGUAGE, MainScenarioEntry, ResolveOptions, TextResolver,
    main_quest_id_for_chapter,
};
use serde_json::{Map, Value, json};

use crate::data_cache::{GameDataCache, GameDataSource};
use crate::data_tools_db::call_data_tool_db;

pub(crate) const SEARCH_MAX_RESULTS: usize = 20;

/// Names of all data-category tools handled by this module (`ffr-3`).
pub const DATA_TOOL_NAMES: [&str; 15] = [
    "read_chapter",
    "read_map",
    "read_enemy",
    "read_battle_group",
    "read_skill",
    "read_item",
    "read_quest",
    "read_shop",
    "search_data",
    "find_route",
    "where_to_get",
    "read_npc",
    "read_talk",
    "whats_near",
    "query_game_db",
];

/// Returns `true` if `name` is one of the static game-data tools.
pub fn is_data_tool(name: &str) -> bool {
    DATA_TOOL_NAMES.contains(&name)
}

/// Build descriptors for all data-category tools.
pub fn data_tool_descriptors() -> Vec<ToolDescriptor> {
    vec![
        ToolDescriptor {
            name: "read_chapter".to_owned(),
            description: "读取章节名称和主线剧情条目（含进度阈值与目的地地图编号）。省略 chapter 时列出全部章节概览。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "chapter": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "可选章节号（从 1 开始）；省略时列出全部章节。"
                    }
                },
                "additionalProperties": false
            }),
            category: ToolCategory::Data,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "read_map".to_owned(),
            description: "按地图编号或运行时关卡名读取地图详情，包含出口与目标地图、NPC、商店、宝箱内容与坐标、存档点位置/能力、传送地标及遇敌区域。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "map_id": {
                        "type": "integer",
                        "description": "地图数字编号（与 level_name 二选一）。"
                    },
                    "level_name": {
                        "type": "string",
                        "minLength": 1,
                        "description": "运行时关卡名或 UE 关卡路径，例如 Dng_01Gra_43_PL（与 map_id 二选一）。"
                    }
                },
                "additionalProperties": false
            }),
            category: ToolCategory::Data,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "read_enemy".to_owned(),
            description: "按单位编号或简体中文名读取敌人完整资料（HP、破防槽、弱点和抗性、掉落与偷窃、出没章节、所在战斗组、是否 Boss、高危招式）。按名称匹配多个时列出候选供选择。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "unit_id": {
                        "type": "integer",
                        "description": "单位数字编号（与 name 二选一）。"
                    },
                    "name": {
                        "type": "string",
                        "minLength": 1,
                        "description": "敌人简体中文名（与 unit_id 二选一）。"
                    }
                },
                "additionalProperties": false
            }),
            category: ToolCategory::Data,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "read_battle_group".to_owned(),
            description: "按战斗组编号读取一场遭遇中的敌人构成、是否含 Boss、逃跑规则及各成员弱点与破防槽。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "group_id": {
                        "type": "integer",
                        "description": "战斗组数字编号（DT_BattleGroup.ID）。"
                    }
                },
                "required": ["group_id"],
                "additionalProperties": false
            }),
            category: ToolCategory::Data,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "read_skill".to_owned(),
            description: "按技能编号或简体中文名读取招式属性、威力、破防削减、消耗、目标范围与说明。按名称匹配多个时列出候选。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "skill_id": {
                        "type": "integer",
                        "description": "技能数字编号（与 name 二选一）。"
                    },
                    "name": {
                        "type": "string",
                        "minLength": 1,
                        "description": "技能简体中文名（与 skill_id 二选一）。"
                    }
                },
                "additionalProperties": false
            }),
            category: ToolCategory::Data,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "read_item".to_owned(),
            description: "按道具编号或简体中文名读取消耗品、装备、幻兵、贵重品或圣遗物的价格、属性与效果说明。按名称匹配多个时列出候选。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "item_id": {
                        "type": "integer",
                        "description": "全局道具编号（与 name 二选一）。"
                    },
                    "name": {
                        "type": "string",
                        "minLength": 1,
                        "description": "道具简体中文名（与 item_id 二选一）。"
                    }
                },
                "additionalProperties": false
            }),
            category: ToolCategory::Data,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "read_quest".to_owned(),
            description: "按任务编号、简体中文标题或章节号读取支线任务详情或列表（含委托人、目标与奖励）。按名称匹配多个时列出候选。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "quest_id": {
                        "type": "integer",
                        "description": "支线任务数字编号（与 name、chapter 三选一）。"
                    },
                    "name": {
                        "type": "string",
                        "minLength": 1,
                        "description": "支线任务简体中文标题（与 quest_id、chapter 三选一）。"
                    },
                    "chapter": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "按章节号筛选该章可接取的支线任务（与 quest_id、name 三选一）。"
                    }
                },
                "additionalProperties": false
            }),
            category: ToolCategory::Data,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "read_shop".to_owned(),
            description: "按商店编号、简体中文店名或所售道具编号读取商店目录与价格。按店名匹配多个时列出候选；按 item_id 反查出售该道具的全部商店。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "shop_id": {
                        "type": "integer",
                        "description": "商店数字编号（与 name、item_id 三选一）。"
                    },
                    "name": {
                        "type": "string",
                        "minLength": 1,
                        "description": "商店简体中文名（与 shop_id、item_id 三选一）。"
                    },
                    "item_id": {
                        "type": "integer",
                        "description": "按道具编号反查所有出售该道具的商店（与 shop_id、name 三选一）。"
                    }
                },
                "additionalProperties": false
            }),
            category: ToolCategory::Data,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "search_data".to_owned(),
            description: "按简体中文关键词模糊查找游戏数据编号，支持跨章节、地图、敌人、技能、道具、任务、商店检索，最多返回 20 条。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "minLength": 1,
                        "description": "简体中文搜索关键词。"
                    },
                    "kinds": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "enum": ["chapter", "map", "enemy", "skill", "item", "quest", "shop"]
                        },
                        "description": "可选，限定检索的类别列表；省略时检索全部类别。"
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            category: ToolCategory::Data,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "find_route".to_owned(),
            description: "规划从起点地图到终点地图的最短换图路线（支持普通出口与存档点传送），返回每步出口坐标、目标地图与落点坐标、旗标/进度条件及是否仅事件触发。起点和终点支持地图编号或简体中文名；重名时列出候选。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "from": {
                        "oneOf": [
                            { "type": "integer" },
                            { "type": "string", "minLength": 1 }
                        ],
                        "description": "起点地图编号（整数）或简体中文地图名（字符串）。"
                    },
                    "to": {
                        "oneOf": [
                            { "type": "integer" },
                            { "type": "string", "minLength": 1 }
                        ],
                        "description": "终点地图编号（整数）或简体中文地图名（字符串）。"
                    }
                },
                "required": ["from", "to"],
                "additionalProperties": false
            }),
            category: ToolCategory::Data,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "where_to_get".to_owned(),
            description: "查询指定道具的全部静态获取途径：宝箱（地图、坐标、出现条件）、商店（地图、NPC、价格、条件）、敌人掉落和偷窃（概率、出没章节）与任务奖励。重名时列出候选。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "item_id": {
                        "type": "integer",
                        "description": "全局道具数字编号（与 name 二选一）。"
                    },
                    "name": {
                        "type": "string",
                        "minLength": 1,
                        "description": "道具简体中文名（与 item_id 二选一）。"
                    }
                },
                "additionalProperties": false
            }),
            category: ToolCategory::Data,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "read_npc".to_owned(),
            description: "按「地图编号 + unique_id」、地图单位编号（map_unit_id）或名字查询 NPC 详情，返回名字、所在地图和坐标、出现条件、对话事件列表、售卖商品与价格及是否旅店。按名字匹配多个时列出候选。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "map_id": {
                        "type": "integer",
                        "description": "地图数字编号。按 unique_id 查询时必填；按 map_unit_id 或 name 查询时可选，用于限定地图。"
                    },
                    "unique_id": {
                        "type": "integer",
                        "description": "关卡内 NPC 对象 unique_id（须与 map_id 同时传入）。"
                    },
                    "map_unit_id": {
                        "type": "integer",
                        "description": "地图单位编号（DT_MapUnitAsset.ID，与 unique_id、name 三选一）。"
                    },
                    "name": {
                        "type": "string",
                        "minLength": 1,
                        "description": "NPC 中文名或内部名关键词（与 unique_id、map_unit_id 三选一）。"
                    }
                },
                "additionalProperties": false
            }),
            category: ToolCategory::Data,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "read_talk".to_owned(),
            description: "按对话事件编号读取完整对话内容，返回台词（说话人与简体中文文本）、后续事件与选项分支、关联过场（时长、能否跳过）以及触发的战斗组。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "event_id": {
                        "type": "string",
                        "minLength": 1,
                        "description": "对话事件编号（CDT_TalkEventData_Prod 行键，如 C01_001_01）。"
                    }
                },
                "required": ["event_id"],
                "additionalProperties": false
            }),
            category: ToolCategory::Data,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "whats_near".to_owned(),
            description: "查询指定位置半径内的 NPC、宝箱、出口、存档点、事件触发与遇敌区域，按距离排序并给出东南西北方向。默认从文件桥读取当前地图和角色坐标，也可显式传入 map_id 与 x/y（可选 z）。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "map_id": {
                        "type": "integer",
                        "description": "可选，地图数字编号；省略时从文件桥读取当前地图。"
                    },
                    "x": {
                        "type": "number",
                        "description": "可选，查询中心世界坐标 X（须与 y 同时传入）；省略时从文件桥读取当前角色坐标。"
                    },
                    "y": {
                        "type": "number",
                        "description": "可选，查询中心世界坐标 Y（须与 x 同时传入）；省略时从文件桥读取当前角色坐标。"
                    },
                    "z": {
                        "type": "number",
                        "description": "可选，查询中心世界坐标 Z；显式传 x/y 且省略 z 时按平面 2D 距离检索。"
                    },
                    "radius": {
                        "type": "number",
                        "exclusiveMinimum": 0,
                        "description": "可选，搜索半径（UE 坐标单位，默认 2000）。"
                    },
                    "kinds": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "enum": ["npc", "chest", "exit", "save_point", "event", "encounter_zone"]
                        },
                        "description": "可选，限定返回的对象类别列表；省略时返回全部 6 类。"
                    }
                },
                "additionalProperties": false
            }),
            category: ToolCategory::Data,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "query_game_db".to_owned(),
            description: "对本地游戏数据库执行单条只读 SQL（SELECT/WITH/VALUES），最多返回 200 行、2 秒超时、输出超过约 20 KB 自动截断。可先读取资源 ffr://db/schema 了解全部表和列。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "sql": {
                        "type": "string",
                        "minLength": 1,
                        "description": "单条只读 SQL 查询语句（仅允许 SELECT / WITH / VALUES，禁止写语句和多条语句）。"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "description": "可选，最大返回行数（1..=200，默认 200）。"
                    }
                },
                "required": ["sql"],
                "additionalProperties": false
            }),
            category: ToolCategory::Data,
            side_effects: false,
            timeout_ms: None,
        },
    ]
}

/// Dispatch one data-category tool call.
pub async fn call_data_tool(
    channel: &FileChannel,
    source: &GameDataSource,
    name: &str,
    args: Value,
) -> ToolOutput {
    let Some(obj) = args.as_object() else {
        return tool_error(format!("工具 `{name}` 的参数必须是 JSON 对象"));
    };

    let result = match source {
        GameDataSource::Db(db_cache) => call_data_tool_db(channel, db_cache, name, obj).await,
        GameDataSource::Json(cache) => match name {
            "read_chapter" => handle_read_chapter(cache, obj),
            "read_map" => handle_read_map(cache, obj),
            "read_enemy" => handle_read_enemy(cache, obj),
            "read_battle_group" => handle_read_battle_group(cache, obj),
            "read_skill" => handle_read_skill(cache, obj),
            "read_item" => handle_read_item(cache, obj),
            "read_quest" => handle_read_quest(cache, obj),
            "read_shop" => handle_read_shop(cache, obj),
            "search_data" => handle_search_data(cache, obj),
            "find_route" | "where_to_get" | "read_npc" | "read_talk" | "whats_near"
            | "query_game_db" => Err(format!(
                "工具 `{name}` 需要本地 SQLite 游戏数据库（ffr-db），当前以 --data-dir 导出目录模式运行；请改用 --db 或默认 $FFR_DEPS/ffr/db/ffr.sqlite（可先运行 `ffr-db build` 构建数据库）"
            )),
            _ => Err(format!("未知数据工具 `{name}`")),
        },
    };

    match result {
        Ok(output) => output,
        Err(message) => tool_error(message),
    }
}

// ---- Tool handlers ----

fn handle_read_chapter(
    cache: &GameDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("read_chapter", args, &["chapter"])?;
    let chapter = optional_i64("read_chapter", args, "chapter")?;
    if let Some(ch) = chapter
        && ch < 1
    {
        return Err("工具 `read_chapter` 的参数 `chapter` 必须大于等于 1".to_owned());
    }

    let scenario = cache.scenario()?;
    let world = cache.world().ok();

    let Some(chapter_no) = chapter else {
        let mut lines = vec![format!("共 {} 个章节：", scenario.chapters.len())];
        let mut items = Vec::with_capacity(scenario.chapters.len());
        for ch in &scenario.chapters {
            let name = scenario.chapter_name(ch.chapter_no).text;
            let entries = scenario.chapter_main_scenario(ch.chapter_no);
            let name_display = name.as_deref().unwrap_or("无中文译名");
            lines.push(format!(
                "- 第 {} 章「{}」（主线 {} 条）",
                ch.chapter_no,
                name_display,
                entries.len()
            ));
            let main_quest_id = main_quest_id_for_chapter(ch.chapter_no);
            let main_quest_title =
                main_quest_id.and_then(|qid| scenario.main_quest_title(qid).text);
            items.push(json!({
                "chapter_no": ch.chapter_no,
                "main_quest_id": main_quest_id,
                "main_quest_title": main_quest_title,
                "row_key": ch.row_key,
                "name": name,
                "main_scenario_count": entries.len(),
            }));
        }
        return Ok(tool_ok(lines.join("\n"), json!({ "chapters": items })));
    };

    let Some(ch) = scenario.find_chapter(chapter_no) else {
        return Err(format!("章节表中找不到第 {chapter_no} 章"));
    };
    let name = scenario.chapter_name(chapter_no).text;
    let name_display = name.as_deref().unwrap_or("无中文译名");
    let entries = scenario.chapter_main_scenario(chapter_no);

    let mut lines = vec![format!(
        "第 {} 章「{}」（共 {} 条主线剧情）：",
        chapter_no,
        name_display,
        entries.len()
    )];
    let mut json_entries = Vec::with_capacity(entries.len());

    for entry in &entries {
        let texts = scenario.resolve_main_scenario_texts(entry);
        let goal_text = clean_optional(texts.text.text);
        let summary_text = clean_optional(texts.summary_text.text);
        let (dest_labels, dest_json) = format_scenario_destinations(entry, world.as_deref());

        let progress_str = entry
            .min_progress
            .map_or_else(|| "无进度阈值".to_owned(), |p| format!("进度>={p}"));
        let goal_display = goal_text.as_deref().unwrap_or("无中文目标");
        let mut line = format!("- [{progress_str}] {goal_display}");
        if !dest_labels.is_empty() {
            line.push_str(&format!("（目的地：{}）", dest_labels.join("、")));
        }
        if let Some(summary) = &summary_text {
            line.push_str(&format!(" — {summary}"));
        }
        lines.push(line);

        json_entries.push(json!({
            "row_key": entry.row_key,
            "condition": entry.condition,
            "min_progress": entry.min_progress,
            "text": goal_text,
            "summary": summary_text,
            "destinations": dest_json,
        }));
    }

    let main_quest_id = main_quest_id_for_chapter(ch.chapter_no);
    let main_quest_title = main_quest_id.and_then(|qid| scenario.main_quest_title(qid).text);
    Ok(tool_ok(
        lines.join("\n"),
        json!({
            "chapter_no": ch.chapter_no,
            "main_quest_id": main_quest_id,
            "main_quest_title": main_quest_title,
            "row_key": ch.row_key,
            "name": name,
            "main_scenario": json_entries,
        }),
    ))
}

fn handle_read_map(cache: &GameDataCache, args: &Map<String, Value>) -> Result<ToolOutput, String> {
    reject_unknown_keys("read_map", args, &["map_id", "level_name"])?;
    let map_id = optional_i64("read_map", args, "map_id")?;
    let level_name = optional_non_empty_str("read_map", args, "level_name")?;

    let map_query = match (map_id, level_name) {
        (Some(id), None) => MapLookup::ById(id),
        (None, Some(level)) => MapLookup::ByLevel(level),
        (None, None) => {
            return Err("工具 `read_map` 必须提供 `map_id` 或 `level_name` 之一".to_owned());
        }
        (Some(_), Some(_)) => {
            return Err(
                "工具 `read_map` 的 `map_id` 与 `level_name` 不能同时传入，请二选一".to_owned(),
            );
        }
    };

    let world = cache.world()?;
    let map = match map_query {
        MapLookup::ById(id) => world
            .map_by_id(id)
            .ok_or_else(|| format!("找不到编号为 {id} 的地图"))?,
        MapLookup::ByLevel(level) => world
            .map_by_level_name(level)
            .ok_or_else(|| format!("找不到关卡名为「{level}」的地图"))?,
    };

    Ok(format_map_output(&world, map))
}

enum MapLookup<'a> {
    ById(i64),
    ByLevel(&'a str),
}

fn format_map_output(world: &WorldData, map: &MapInfo) -> ToolOutput {
    let base_summary = map_summary(world, map);
    let name = resolve_zh_clean(world.text_resolver(), &map.name);
    let island_name = world
        .island_by_type(&map.island)
        .and_then(|island| resolve_zh_clean(world.text_resolver(), &island.name));

    let save_point = world.save_point_for_map_type(&map.map_type);
    let teleport_points = world.teleport_points_for_map(map.id);
    let treasure_boxes = world.treasure_boxes_in_map(map.id);

    let shop_facilities: Vec<&str> = map
        .facilities
        .iter()
        .filter(|fac| fac.interact_type.starts_with("eMapInteractType::Shop"))
        .map(|fac| facility_zh(&fac.interact_type))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut summary_lines = vec![base_summary];
    if !shop_facilities.is_empty() {
        summary_lines.push(format!(
            "商店设施：{}（静态地图表仅记录设施类型，不含 ShopID；实际商店编号位于关卡 .umap 的 NPC 属性中，进图后可用 read_game_state 读取 npcs[].shop_id，或用 read_shop / search_data 查询）",
            shop_facilities.join("、")
        ));
    }
    if !teleport_points.is_empty() {
        let landing_ids: Vec<String> = teleport_points
            .iter()
            .filter_map(|tp| tp.airship_landing_point_id.map(|id| format!("#{id}")))
            .collect();
        if landing_ids.is_empty() {
            summary_lines.push(format!("传送地标：{} 处", teleport_points.len()));
        } else {
            summary_lines.push(format!(
                "传送地标：{} 处（飞空艇降落点 {}）",
                teleport_points.len(),
                landing_ids.join("、")
            ));
        }
    }
    if !treasure_boxes.is_empty() {
        let mut chest_parts = Vec::with_capacity(treasure_boxes.len());
        for chest in &treasure_boxes {
            let content = describe_treasure_box(world, chest);
            chest_parts.push(format!("#{} {content}", chest.id));
        }
        summary_lines.push(format!("宝箱明细：{}", chest_parts.join("；")));
    }

    let save_point_json = save_point.map(|sp| {
        json!({
            "map_type": sp.map_type,
            "can_save": sp.can_save,
            "can_use_tent": sp.can_use_tent,
            "can_teleport": sp.can_teleport,
        })
    });

    let teleport_json: Vec<Value> = teleport_points
        .iter()
        .map(|tp| {
            json!({
                "index": tp.index,
                "airship_landing_point_id": tp.airship_landing_point_id,
                "location": tp.location,
                "targets": tp.targets.iter().map(|t| json!({
                    "map_id": t.map_id,
                    "condition": t.condition,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    let treasures_json: Vec<Value> = treasure_boxes
        .iter()
        .map(|chest| {
            let item_name = chest
                .item_id
                .and_then(|id| world.item_by_id(id))
                .and_then(|item| resolve_zh_clean(world.text_resolver(), &item.name));
            json!({
                "box_id": chest.id,
                "unique_id": null,
                "item_id": chest.item_id,
                "item_name": item_name,
                "item_num": chest.item_num,
                "money": chest.money,
                "is_trap": chest.is_trap,
                "battle_group_id": chest.battle_group_id,
                "battle_finish_event_id": chest.battle_finish_event_id,
                "location": null,
                "visible_condition": null,
            })
        })
        .collect();

    let facilities_json: Vec<Value> = map
        .facilities
        .iter()
        .map(|fac| {
            json!({
                "interact_type": fac.interact_type,
                "label": facility_zh(&fac.interact_type),
                "condition": fac.condition,
            })
        })
        .collect();

    tool_ok(
        summary_lines.join("\n"),
        json!({
            "map_id": map.id,
            "row_key": map.row_key,
            "name": name,
            "level_name": map.level_name,
            "level_asset_path": map.level_asset_path,
            "map_type": map.map_type,
            "dungeon_type": map.dungeon_type,
            "island": map.island,
            "island_name": island_name,
            "region_id": map.region_id,
            "danger_level": map.danger_level,
            "do_movement_encount": map.do_movement_encount,
            "encount_group_id": map.encount_group_id,
            "save_point": save_point_json,
            "save_points": [],
            "teleportable": !teleport_points.is_empty(),
            "teleport_points": teleport_json,
            "exits": [],
            "npcs": [],
            "shops": [],
            "treasure_boxes": treasures_json,
            "encounter_zones": [],
            "shop_facilities": shop_facilities,
            "facilities": facilities_json,
        }),
    )
}

fn handle_read_enemy(
    cache: &GameDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("read_enemy", args, &["unit_id", "name"])?;
    let unit_id = optional_i64("read_enemy", args, "unit_id")?;
    let name = optional_non_empty_str("read_enemy", args, "name")?;

    let battle = cache.battle()?;
    let world = cache.world().ok();
    let chapter_index = cache.enemy_chapters().ok();
    match (unit_id, name) {
        (Some(id), None) => {
            let unit = battle
                .unit_by_id(id)
                .ok_or_else(|| format!("单位表中找不到编号为 {id} 的单位"))?;
            format_enemy_output(&battle, unit, world.as_deref(), chapter_index.as_deref())
        }
        (None, Some(query_name)) => {
            let matches = battle
                .enemies_by_name(query_name)
                .map_err(|error| format!("按名称查询敌人失败：{error}"))?;
            match matches.as_slice() {
                [] => {
                    let similar = fuzzy_enemy_hints(&battle, query_name, 5);
                    if similar.is_empty() {
                        Err(format!(
                            "找不到简体中文名为「{query_name}」的敌人，请用 search_data 模糊查找编号"
                        ))
                    } else {
                        Err(format!(
                            "找不到完全名为「{query_name}」的敌人；相近候选：{}（请改用 unit_id 或 search_data 查询）",
                            similar.join("、")
                        ))
                    }
                }
                [single] => {
                    let unit = battle
                        .unit_by_id(single.unit_id)
                        .ok_or_else(|| format!("单位表中找不到编号为 {} 的单位", single.unit_id))?;
                    format_enemy_output(&battle, unit, world.as_deref(), chapter_index.as_deref())
                }
                many => Ok(format_enemy_candidates(query_name, many)),
            }
        }
        (None, None) => Err("工具 `read_enemy` 必须提供 `unit_id` 或 `name` 之一".to_owned()),
        (Some(_), Some(_)) => {
            Err("工具 `read_enemy` 的 `unit_id` 与 `name` 不能同时传入，请二选一".to_owned())
        }
    }
}

fn format_enemy_output(
    battle: &BattleData,
    unit: &Unit,
    world: Option<&WorldData>,
    chapter_index: Option<&ffr_data::battle::EnemyChapterIndex>,
) -> Result<ToolOutput, String> {
    let profile = unit.to_enemy_profile(battle.unit_ai_by_id(unit.id));
    let resolved_name = battle.unit_name_in(unit.id, MODEL_LANGUAGE);
    let mut summary = match battle.enemy_summary(unit.id, MODEL_LANGUAGE) {
        Ok(text) => text,
        Err(ffr_data::DataError::InvalidSchema { .. }) => {
            // Unit exists but lacks a zh-Hans localization entry; report honestly without faking ja.
            format_fallback_enemy_summary(battle, &profile, resolved_name.text.as_deref())
        }
        Err(error) => return Err(format!("生成敌人摘要失败：{error}")),
    };
    let chapters = chapter_index.map(|idx| idx.chapters_for_enemy(unit.id).to_vec());
    if let Some(ch_list) = &chapters
        && !ch_list.is_empty()
    {
        let joined: Vec<String> = ch_list.iter().map(|ch| format!("第 {ch} 章")).collect();
        summary.push_str(&format!("\n出没章节：{}", joined.join("、")));
    }

    let lookup_item_name = |item_id: i64| -> Option<String> {
        let w = world?;
        let it = w.item_by_id(item_id)?;
        resolve_zh_clean(w.text_resolver(), &it.name)
    };
    let (drops_json, steals_json, drop_line, steal_line) =
        build_enemy_drops_and_steals(unit, lookup_item_name);
    if let Some(line) = drop_line {
        summary.push_str(&format!("\n{line}"));
    }
    if let Some(line) = steal_line {
        summary.push_str(&format!("\n{line}"));
    }

    let mut groups: Vec<&ffr_data::battle::BattleGroup> = battle
        .battle_groups
        .iter()
        .filter(|g| g.unit_ids.contains(&unit.id))
        .collect();
    groups.sort_by_key(|g| g.id);
    let (battle_groups_json, groups_line) = format_enemy_battle_groups_slice(&groups);
    if let Some(line) = groups_line {
        summary.push_str(&format!("\n{line}"));
    }

    let high_risk_skills: Vec<Value> = profile
        .high_risk_skill_ids
        .iter()
        .map(|&skill_id| {
            let skill_name = battle
                .skill_name_in(skill_id, MODEL_LANGUAGE)
                .text
                .and_then(|t| collapse_ws(&t));
            json!({
                "skill_id": skill_id,
                "name": skill_name,
            })
        })
        .collect();

    let payload = json!({
        "unit_id": profile.unit_id,
        "row_key": profile.row_key,
        "name": resolved_name.text,
        "category": unit_category_code(profile.category),
        "level": profile.level,
        "is_boss": profile.is_boss,
        "max_hp": profile.max_hp,
        "max_mp": profile.max_mp,
        "max_break_point": profile.max_break_point,
        "stats": {
            "attack": profile.attack,
            "defence": profile.defence,
            "intelligence": profile.intelligence,
            "mind": profile.mind,
            "divine": profile.divine,
            "agility": profile.agility,
        },
        "weaknesses": elements_zh_vec(&profile.weaknesses),
        "weakness_elements": elements_code_vec(&profile.weaknesses),
        "resistances": {
            "halved": elements_zh_vec(&profile.halved),
            "invalid": elements_zh_vec(&profile.invalid),
            "absorb": elements_zh_vec(&profile.absorb),
        },
        "break_calc": {
            "weakness_multiplier": battle
                .break_calc
                .break_element_multipliers
                .get(&Resistance::Weakness)
                .copied(),
            "break_damage_magnification": battle.break_calc.break_damage_magnification,
        },
        "high_risk_skills": high_risk_skills,
        "drops": drops_json,
        "steals": steals_json,
        "chapters": chapters,
        "battle_groups": battle_groups_json,
    });

    Ok(tool_ok(summary, payload))
}

pub(crate) fn build_enemy_drops_and_steals<F>(
    unit: &Unit,
    mut resolve_item_name: F,
) -> (Value, Value, Option<String>, Option<String>)
where
    F: FnMut(i64) -> Option<String>,
{
    let mut make_slot = |item_id: i64, rate: i64, label: &str| -> (Value, Option<String>) {
        if item_id < 0 {
            return (Value::Null, None);
        }
        let item_name = resolve_item_name(item_id);
        let display = match &item_name {
            Some(n) => format!("{n}（#{item_id}）"),
            None => format!("道具 #{item_id}"),
        };
        (
            json!({
                "item_id": item_id,
                "item_name": item_name,
                "rate": rate,
            }),
            Some(format!("{label} {display} {rate}%")),
        )
    };

    let (dn_json, dn_str) = make_slot(unit.drop_normal_item, unit.drop_normal_item_rate, "普通");
    let (dr_json, dr_str) = make_slot(unit.drop_rare_item, unit.drop_rare_item_rate, "稀有");
    let (sn_json, sn_str) = make_slot(unit.steal_normal_item, unit.steal_item_rate, "普通");
    let (sr_json, sr_str) = make_slot(unit.steal_rare_item, unit.steal_item_rate, "稀有");

    let drop_parts: Vec<String> = [dn_str, dr_str].into_iter().flatten().collect();
    let steal_parts: Vec<String> = [sn_str, sr_str].into_iter().flatten().collect();

    let drop_line = (!drop_parts.is_empty()).then(|| format!("掉落：{}", drop_parts.join("、")));
    let steal_line = (!steal_parts.is_empty()).then(|| format!("偷窃：{}", steal_parts.join("、")));

    (
        json!({ "normal": dn_json, "rare": dr_json }),
        json!({ "normal": sn_json, "rare": sr_json }),
        drop_line,
        steal_line,
    )
}

pub(crate) fn format_enemy_battle_groups_slice(
    groups: &[&ffr_data::battle::BattleGroup],
) -> (Vec<Value>, Option<String>) {
    let json_list: Vec<Value> = groups
        .iter()
        .map(|g| {
            json!({
                "group_id": g.id,
                "is_boss": g.is_boss,
                "can_escape": g.can_escape,
                "is_bounty": g.is_bounty,
                "ap": g.ap,
            })
        })
        .collect();
    if groups.is_empty() {
        return (json_list, None);
    }
    let preview: Vec<String> = groups
        .iter()
        .take(12)
        .map(|g| format!("#{}", g.id))
        .collect();
    let line = if groups.len() > 12 {
        format!(
            "所在战斗组：{} 等共 {} 组",
            preview.join("、"),
            groups.len()
        )
    } else {
        format!("所在战斗组：{}", preview.join("、"))
    };
    (json_list, Some(line))
}

pub(crate) fn format_enemy_candidates(query: &str, matches: &[EnemyProfile]) -> ToolOutput {
    let mut lines = vec![format!(
        "名称「{query}」匹配到 {} 个敌人候选，请改用 `unit_id` 精确查询：",
        matches.len()
    )];
    let mut candidates = Vec::with_capacity(matches.len());
    for enemy in matches {
        let kind = if enemy.is_boss {
            "Boss"
        } else {
            "普通敌人"
        };
        let weak = join_elements_zh(&enemy.weaknesses);
        lines.push(format!(
            "- unit_id={}「{query}」（Lv.{}，{kind}，HP {}，破防槽 {}，弱点：{}）",
            enemy.unit_id, enemy.level, enemy.max_hp, enemy.max_break_point, weak
        ));
        candidates.push(json!({
            "unit_id": enemy.unit_id,
            "row_key": enemy.row_key,
            "name": query,
            "level": enemy.level,
            "is_boss": enemy.is_boss,
            "max_hp": enemy.max_hp,
            "max_break_point": enemy.max_break_point,
            "weaknesses": elements_zh_vec(&enemy.weaknesses),
        }));
    }
    tool_ok(
        lines.join("\n"),
        json!({
            "ambiguous": true,
            "query": query,
            "candidates": candidates,
        }),
    )
}

fn handle_read_battle_group(
    cache: &GameDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("read_battle_group", args, &["group_id"])?;
    let group_id = required_i64("read_battle_group", args, "group_id")?;

    let battle = cache.battle()?;
    let resolved = battle
        .resolve_battle_group(group_id)
        .map_err(|error| format!("读取战斗组 {group_id} 失败：{error}"))?;
    Ok(format_battle_group_output(&battle, &resolved))
}

fn format_battle_group_output(battle: &BattleData, resolved: &ResolvedBattleGroup) -> ToolOutput {
    let group = &resolved.group;
    let kind_label = if resolved.is_boss {
        "Boss 战"
    } else {
        "普通遭遇"
    };
    let escape_label = if group.can_escape {
        "可逃跑"
    } else {
        "不可逃跑"
    };
    let bounty_label = if group.is_bounty { "，赏金战" } else { "" };

    let mut lines = vec![format!(
        "战斗组 #{}（{kind_label}，{escape_label}{bounty_label}，AP {}，共 {} 名敌人）：",
        group.id,
        group.ap,
        resolved.members.len()
    )];
    let mut members_json = Vec::with_capacity(resolved.members.len());

    for member in &resolved.members {
        let name = battle
            .unit_name_in(member.unit_id, MODEL_LANGUAGE)
            .text
            .and_then(|t| collapse_ws(&t));
        let name_display = name.as_deref().unwrap_or("无中文译名");
        let role = if member.is_boss { "Boss" } else { "普通" };
        let weak = join_elements_zh(&member.weaknesses);
        lines.push(format!(
            "- unit_id={}「{}」（Lv.{}，{}，HP {}，破防槽 {}，弱点：{}）",
            member.unit_id,
            name_display,
            member.level,
            role,
            member.max_hp,
            member.max_break_point,
            weak
        ));

        members_json.push(json!({
            "unit_id": member.unit_id,
            "name": name,
            "level": member.level,
            "is_boss": member.is_boss,
            "max_hp": member.max_hp,
            "max_break_point": member.max_break_point,
            "weaknesses": elements_zh_vec(&member.weaknesses),
            "resistances": {
                "halved": elements_zh_vec(&member.halved),
                "invalid": elements_zh_vec(&member.invalid),
                "absorb": elements_zh_vec(&member.absorb),
            },
            "high_risk_skill_ids": member.high_risk_skill_ids,
        }));
    }

    tool_ok(
        lines.join("\n"),
        json!({
            "group_id": group.id,
            "row_key": group.row_key,
            "is_boss": resolved.is_boss,
            "can_escape": group.can_escape,
            "escape_probability": group.escape_probability,
            "is_bounty": group.is_bounty,
            "ap": group.ap,
            "unit_ids": group.unit_ids,
            "members": members_json,
        }),
    )
}

fn handle_read_skill(
    cache: &GameDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("read_skill", args, &["skill_id", "name"])?;
    let skill_id = optional_i64("read_skill", args, "skill_id")?;
    let name = optional_non_empty_str("read_skill", args, "name")?;

    let battle = cache.battle()?;
    match (skill_id, name) {
        (Some(id), None) => {
            let skill = battle
                .skill_by_id(id)
                .ok_or_else(|| format!("技能表中找不到编号为 {id} 的技能"))?;
            Ok(format_skill_output(&battle, skill))
        }
        (None, Some(query_name)) => {
            let matches = battle
                .skills_by_name(query_name)
                .map_err(|error| format!("按名称查询技能失败：{error}"))?;
            match matches.as_slice() {
                [] => {
                    let similar = fuzzy_skill_hints(&battle, query_name, 5);
                    if similar.is_empty() {
                        Err(format!(
                            "找不到简体中文名为「{query_name}」的技能，请用 search_data 模糊查找编号"
                        ))
                    } else {
                        Err(format!(
                            "找不到完全名为「{query_name}」的技能；相近候选：{}（请改用 skill_id 或 search_data 查询）",
                            similar.join("、")
                        ))
                    }
                }
                [single] => Ok(format_skill_output(&battle, single)),
                many => Ok(format_skill_candidates(query_name, many)),
            }
        }
        (None, None) => Err("工具 `read_skill` 必须提供 `skill_id` 或 `name` 之一".to_owned()),
        (Some(_), Some(_)) => {
            Err("工具 `read_skill` 的 `skill_id` 与 `name` 不能同时传入，请二选一".to_owned())
        }
    }
}

fn format_skill_output(battle: &BattleData, skill: &Skill) -> ToolOutput {
    let name = battle
        .skill_name_in(skill.id, MODEL_LANGUAGE)
        .text
        .and_then(|t| collapse_ws(&t));
    let description = resolve_zh_clean(&battle.text_resolver, &skill.description);
    let name_display = name.as_deref().unwrap_or("无中文译名");
    let element_zh = element_zh(skill.element);
    let damage_zh = damage_type_zh(skill.damage_type);
    let attr_zh = skill_attr_zh(skill.skill_attr_type);
    let cost_label = format_skill_cost(skill.cost, skill.cost_type);
    let target_label = format_skill_target(
        skill.default_target_relation,
        skill.target_type,
        skill.default_target_state,
    );
    let loc_zh = available_location_zh(skill.available_location);

    let mut summary = format!(
        "技能 #{}「{}」（{}/{}）；属性：{}；威力：{}；破防削减：{}；消耗：{}；目标：{}；场合：{}",
        skill.id,
        name_display,
        attr_zh,
        damage_zh,
        element_zh,
        skill.power,
        skill.break_damage,
        cost_label,
        target_label,
        loc_zh,
    );
    if skill.hit_count > 1 {
        summary.push_str(&format!("；段数：{}", skill.hit_count));
    }
    if let Some(desc) = &description {
        summary.push_str(&format!("；说明：{desc}"));
    }

    tool_ok(
        summary,
        json!({
            "skill_id": skill.id,
            "row_key": skill.row_key,
            "name": name,
            "description": description,
            "element": element_zh,
            "element_code": element_code(skill.element),
            "damage_type": damage_type_code(skill.damage_type),
            "skill_attr_type": skill_attr_code(skill.skill_attr_type),
            "power": skill.power,
            "break_damage": skill.break_damage,
            "cost": skill.cost,
            "cost_type": cost_type_code(skill.cost_type),
            "cost_label": cost_label,
            "target_type": target_type_code(skill.target_type),
            "target_relation": target_relation_code(skill.default_target_relation),
            "target_state": target_state_code(skill.default_target_state),
            "target_label": target_label,
            "hit_count": skill.hit_count,
            "accuracy": skill.accuracy,
            "critical_hit_rate": skill.critical_hit_rate,
            "available_location": available_location_code(skill.available_location),
            "effect_ids": skill.effect_ids,
        }),
    )
}

pub(crate) fn format_skill_candidates(query: &str, matches: &[&Skill]) -> ToolOutput {
    let mut lines = vec![format!(
        "名称「{query}」匹配到 {} 个技能候选，请改用 `skill_id` 精确查询：",
        matches.len()
    )];
    let mut candidates = Vec::with_capacity(matches.len());
    for skill in matches {
        let element = element_zh(skill.element);
        let cost = format_skill_cost(skill.cost, skill.cost_type);
        let target = format_skill_target(
            skill.default_target_relation,
            skill.target_type,
            skill.default_target_state,
        );
        lines.push(format!(
            "- skill_id={}「{query}」（属性：{}，威力 {}，破防 {}，消耗 {}，目标：{}）",
            skill.id, element, skill.power, skill.break_damage, cost, target
        ));
        candidates.push(json!({
            "skill_id": skill.id,
            "row_key": skill.row_key,
            "name": query,
            "element": element,
            "power": skill.power,
            "break_damage": skill.break_damage,
            "cost": skill.cost,
            "cost_type": cost_type_code(skill.cost_type),
            "target_label": target,
        }));
    }
    tool_ok(
        lines.join("\n"),
        json!({
            "ambiguous": true,
            "query": query,
            "candidates": candidates,
        }),
    )
}

fn handle_read_item(
    cache: &GameDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("read_item", args, &["item_id", "name"])?;
    let item_id = optional_i64("read_item", args, "item_id")?;
    let name = optional_non_empty_str("read_item", args, "name")?;

    let world = cache.world()?;
    match (item_id, name) {
        (Some(id), None) => {
            let item = world
                .item_by_id(id)
                .ok_or_else(|| format!("道具表中找不到编号为 {id} 的道具"))?;
            Ok(format_item_output(&world, item))
        }
        (None, Some(query_name)) => {
            let matches = world.find_items_by_name(query_name);
            match matches.as_slice() {
                [] => {
                    let similar: Vec<String> = world
                        .search_items_by_name(query_name)
                        .into_iter()
                        .take(5)
                        .filter_map(|item| {
                            resolve_zh_clean(world.text_resolver(), &item.name)
                                .map(|n| format!("{}（#{}）", n, item.id))
                        })
                        .collect();
                    if similar.is_empty() {
                        Err(format!(
                            "找不到简体中文名为「{query_name}」的道具，请用 search_data 模糊查找编号"
                        ))
                    } else {
                        Err(format!(
                            "找不到完全名为「{query_name}」的道具；相近候选：{}（请改用 item_id 或 search_data 查询）",
                            similar.join("、")
                        ))
                    }
                }
                [single] => Ok(format_item_output(&world, single)),
                many => Ok(format_item_candidates(query_name, many)),
            }
        }
        (None, None) => Err("工具 `read_item` 必须提供 `item_id` 或 `name` 之一".to_owned()),
        (Some(_), Some(_)) => {
            Err("工具 `read_item` 的 `item_id` 与 `name` 不能同时传入，请二选一".to_owned())
        }
    }
}

fn format_item_output(world: &WorldData, item: &Item) -> ToolOutput {
    let mut summary = item_summary(world, item);
    let name = resolve_zh_clean(world.text_resolver(), &item.name);
    let description = resolve_zh_clean(world.text_resolver(), &item.description);
    let shops = world.shops_selling_item(item.id);
    let sold_in_shop_ids: Vec<i64> = shops.iter().map(|shop| shop.id).collect();
    if !sold_in_shop_ids.is_empty() {
        let shop_ids_str: Vec<String> =
            sold_in_shop_ids.iter().map(|id| format!("#{id}")).collect();
        summary.push_str(&format!("；在售商店：{}", shop_ids_str.join("、")));
    }

    tool_ok(
        summary,
        json!({
            "item_id": item.id,
            "row_key": item.row_key,
            "name": name,
            "description": description,
            "category": item.category,
            "category_label": item_category_zh(item.category),
            "buying_price": item.buying_price,
            "selling_price": item.selling_price,
            "is_available_sale": item.is_available_sale,
            "detail": item.detail,
            "sold_in_shop_ids": sold_in_shop_ids,
        }),
    )
}

pub(crate) fn format_item_candidates(query: &str, matches: &[&Item]) -> ToolOutput {
    let mut lines = vec![format!(
        "名称「{query}」匹配到 {} 个道具候选，请改用 `item_id` 精确查询：",
        matches.len()
    )];
    let mut candidates = Vec::with_capacity(matches.len());
    for item in matches {
        let cat = item_category_zh(item.category);
        lines.push(format!(
            "- item_id={}「{query}」（{cat}，买价 {}）",
            item.id, item.buying_price
        ));
        candidates.push(json!({
            "item_id": item.id,
            "row_key": item.row_key,
            "name": query,
            "category": item.category,
            "category_label": cat,
            "buying_price": item.buying_price,
        }));
    }
    tool_ok(
        lines.join("\n"),
        json!({
            "ambiguous": true,
            "query": query,
            "candidates": candidates,
        }),
    )
}

fn handle_read_quest(
    cache: &GameDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("read_quest", args, &["quest_id", "name", "chapter"])?;
    let quest_id = optional_i64("read_quest", args, "quest_id")?;
    let name = optional_non_empty_str("read_quest", args, "name")?;
    let chapter = optional_i64("read_quest", args, "chapter")?;
    if let Some(ch) = chapter
        && ch < 1
    {
        return Err("工具 `read_quest` 的参数 `chapter` 必须大于等于 1".to_owned());
    }

    let provided = usize::from(quest_id.is_some())
        + usize::from(name.is_some())
        + usize::from(chapter.is_some());
    if provided != 1 {
        return Err(
            "工具 `read_quest` 必须在 `quest_id`、`name`、`chapter` 中恰好提供一个参数".to_owned(),
        );
    }

    let world = cache.world()?;
    if let Some(id) = quest_id {
        let quest = world
            .quest_by_id(id)
            .ok_or_else(|| format!("任务表中找不到编号为 {id} 的支线任务"))?;
        return Ok(format_quest_output(&world, quest));
    }
    if let Some(query_name) = name {
        let matches = world.find_quests_by_name(query_name);
        return match matches.as_slice() {
            [] => {
                let similar: Vec<String> = world
                    .search_quests_by_name(query_name)
                    .into_iter()
                    .take(5)
                    .filter_map(|quest| {
                        resolve_zh_clean(world.text_resolver(), &quest.title)
                            .map(|t| format!("{}（#{}）", t, quest.id))
                    })
                    .collect();
                if similar.is_empty() {
                    Err(format!(
                        "找不到简体中文标题为「{query_name}」的支线任务，请用 search_data 模糊查找编号"
                    ))
                } else {
                    Err(format!(
                        "找不到完全名为「{query_name}」的支线任务；相近候选：{}（请改用 quest_id 或 search_data 查询）",
                        similar.join("、")
                    ))
                }
            }
            [single] => Ok(format_quest_output(&world, single)),
            many => Ok(format_quest_candidates(query_name, many)),
        };
    }

    let ch = chapter.expect("exactly one parameter was provided");
    let quests = world.quests_by_chapter(ch);
    if quests.is_empty() {
        return Ok(tool_ok(
            format!("第 {ch} 章没有推导出的支线任务。"),
            json!({ "chapter": ch, "quests": [] }),
        ));
    }

    let mut lines = vec![format!("第 {ch} 章支线任务（共 {} 个）：", quests.len())];
    let mut items = Vec::with_capacity(quests.len());
    for quest in &quests {
        let title = resolve_zh_clean(world.text_resolver(), &quest.title);
        lines.push(format!("- {}", quest_summary(&world, quest)));
        items.push(json!({
            "quest_id": quest.id,
            "row_key": quest.row_key,
            "title": title,
            "chapter_no": quest.chapter_no,
            "requester_map_id": quest.requester_map_id,
            "report_map_id": quest.report_map_id,
            "accept_condition": quest.accept_condition,
        }));
    }
    Ok(tool_ok(
        lines.join("\n"),
        json!({ "chapter": ch, "quests": items }),
    ))
}

fn format_quest_output(world: &WorldData, quest: &Quest) -> ToolOutput {
    let summary = quest_summary(world, quest);
    let title = resolve_zh_clean(world.text_resolver(), &quest.title);
    let requester = resolve_zh_clean(world.text_resolver(), &quest.requester);

    let objectives: Vec<Value> = quest
        .objectives
        .iter()
        .map(|stage| {
            let goal_text = resolve_zh_clean(world.text_resolver(), &stage.goal_text);
            let summary_text = resolve_zh_clean(world.text_resolver(), &stage.summary_text);
            let destinations: Vec<Value> = stage
                .destinations
                .iter()
                .map(|dest| {
                    let map_name = world
                        .map_by_id(dest.destination_id)
                        .and_then(|m| resolve_zh_clean(world.text_resolver(), &m.name));
                    json!({
                        "map_id": dest.destination_id,
                        "map_name": map_name,
                        "point_type": dest.point_type,
                        "destination_type": dest.destination_type,
                        "object_type": dest.object_type,
                        "object_id": dest.object_id,
                        "condition": dest.condition,
                    })
                })
                .collect();
            json!({
                "condition": stage.condition,
                "goal_text": goal_text,
                "summary_text": summary_text,
                "destinations": destinations,
                "quest_board_ids": stage.quest_board_ids,
            })
        })
        .collect();

    let prizes: Vec<Value> = quest
        .prizes
        .iter()
        .map(|prize| {
            let item_name = prize
                .item_id
                .and_then(|id| world.item_by_id(id))
                .and_then(|item| resolve_zh_clean(world.text_resolver(), &item.name));
            let text = resolve_zh_clean(world.text_resolver(), &prize.text);
            json!({
                "item_id": prize.item_id,
                "item_name": item_name,
                "count": prize.count,
                "text": text,
            })
        })
        .collect();

    tool_ok(
        summary,
        json!({
            "quest_id": quest.id,
            "row_key": quest.row_key,
            "title": title,
            "chapter_no": quest.chapter_no,
            "min_main_progress": quest.min_main_progress,
            "category": quest.category,
            "requester": requester,
            "requester_map_id": quest.requester_map_id,
            "report_map_id": quest.report_map_id,
            "accept_condition": quest.accept_condition,
            "occurrence_condition": quest.occurrence_condition,
            "achievement_condition": quest.achievement_condition,
            "completion_condition": quest.completion_condition,
            "objectives": objectives,
            "prizes": prizes,
        }),
    )
}

pub(crate) fn format_quest_candidates(query: &str, matches: &[&Quest]) -> ToolOutput {
    let mut lines = vec![format!(
        "标题「{query}」匹配到 {} 个支线任务候选，请改用 `quest_id` 精确查询：",
        matches.len()
    )];
    let mut candidates = Vec::with_capacity(matches.len());
    for quest in matches {
        let ch = quest
            .chapter_no
            .map_or_else(|| "章节未定".to_owned(), |c| format!("第 {c} 章"));
        lines.push(format!("- quest_id={}「{query}」（{ch}）", quest.id));
        candidates.push(json!({
            "quest_id": quest.id,
            "row_key": quest.row_key,
            "title": query,
            "chapter_no": quest.chapter_no,
            "requester_map_id": quest.requester_map_id,
        }));
    }
    tool_ok(
        lines.join("\n"),
        json!({
            "ambiguous": true,
            "query": query,
            "candidates": candidates,
        }),
    )
}

fn handle_read_shop(
    cache: &GameDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("read_shop", args, &["shop_id", "name", "item_id"])?;
    let shop_id = optional_i64("read_shop", args, "shop_id")?;
    let name = optional_non_empty_str("read_shop", args, "name")?;
    let item_id = optional_i64("read_shop", args, "item_id")?;

    let provided = usize::from(shop_id.is_some())
        + usize::from(name.is_some())
        + usize::from(item_id.is_some());
    if provided != 1 {
        return Err(
            "工具 `read_shop` 必须在 `shop_id`、`name`、`item_id` 中恰好提供一个参数".to_owned(),
        );
    }

    let world = cache.world()?;
    if let Some(id) = shop_id {
        let shop = world
            .shop_by_id(id)
            .ok_or_else(|| format!("商店表中找不到编号为 {id} 的商店"))?;
        return Ok(format_shop_output(&world, shop));
    }

    if let Some(query_name) = name {
        let matches = world.find_shops_by_name(query_name);
        return match matches.as_slice() {
            [] => {
                let similar: Vec<String> = world
                    .shops()
                    .iter()
                    .filter_map(|shop| {
                        let zh = resolve_zh_clean(world.text_resolver(), &shop.name)?;
                        zh.contains(query_name)
                            .then(|| format!("{zh}（#{}）", shop.id))
                    })
                    .take(5)
                    .collect();
                if similar.is_empty() {
                    Err(format!(
                        "找不到简体中文店名为「{query_name}」的商店，请用 search_data 模糊查找编号"
                    ))
                } else {
                    Err(format!(
                        "找不到完全名为「{query_name}」的商店；相近候选：{}（请改用 shop_id 或 search_data 查询）",
                        similar.join("、")
                    ))
                }
            }
            [single] => Ok(format_shop_output(&world, single)),
            many => Ok(format_shop_candidates(query_name, many)),
        };
    }

    let target_item_id = item_id.expect("exactly one parameter was provided");
    let item_name = world
        .item_by_id(target_item_id)
        .and_then(|item| resolve_zh_clean(world.text_resolver(), &item.name));
    let item_label = match &item_name {
        Some(n) => format!("道具 #{target_item_id}「{n}」"),
        None if world.item_by_id(target_item_id).is_some() => {
            format!("道具 #{target_item_id}（无中文译名）")
        }
        None => format!("编号 #{target_item_id}（道具表未收录）"),
    };

    let shops = world.shops_selling_item(target_item_id);
    if shops.is_empty() {
        return Ok(tool_ok(
            format!("没有商店出售{item_label}。"),
            json!({
                "item_id": target_item_id,
                "item_name": item_name,
                "shops": [],
            }),
        ));
    }

    let mut lines = vec![format!("出售{item_label}的商店（共 {} 家）：", shops.len())];
    let mut shops_json = Vec::with_capacity(shops.len());
    for shop in &shops {
        let shop_name = resolve_zh_clean(world.text_resolver(), &shop.name);
        let shop_display = shop_name
            .as_deref()
            .map_or_else(|| "无中文店名".to_owned(), |n| format!("「{n}」"));
        let offers: Vec<Value> = shop
            .items
            .iter()
            .filter(|entry| entry.item_id == target_item_id)
            .map(|entry| {
                json!({
                    "price": world.shop_item_price(entry),
                    "max_order_num": entry.max_order_num,
                    "price_ratio": entry.price_ratio,
                    "condition": entry.condition,
                })
            })
            .collect();
        let price_desc = shop
            .items
            .iter()
            .find(|entry| entry.item_id == target_item_id)
            .and_then(|entry| world.shop_item_price(entry))
            .map_or_else(|| "价格未知".to_owned(), |p| format!("{p}G"));
        lines.push(format!(
            "- shop_id={}{shop_display}（{price_desc}）",
            shop.id
        ));
        shops_json.push(json!({
            "shop_id": shop.id,
            "row_key": shop.row_key,
            "name": shop_name,
            "offers": offers,
        }));
    }

    Ok(tool_ok(
        lines.join("\n"),
        json!({
            "item_id": target_item_id,
            "item_name": item_name,
            "shops": shops_json,
        }),
    ))
}

fn format_shop_output(world: &WorldData, shop: &Shop) -> ToolOutput {
    let summary = shop_summary(world, shop);
    let name = resolve_zh_clean(world.text_resolver(), &shop.name);
    let items: Vec<Value> = shop
        .items
        .iter()
        .map(|entry| {
            let item_name = world
                .item_by_id(entry.item_id)
                .and_then(|item| resolve_zh_clean(world.text_resolver(), &item.name));
            json!({
                "item_id": entry.item_id,
                "item_name": item_name,
                "price": world.shop_item_price(entry),
                "max_order_num": entry.max_order_num,
                "price_ratio": entry.price_ratio,
                "condition": entry.condition,
            })
        })
        .collect();
    let sell_items: Vec<Value> = shop
        .sell_items
        .iter()
        .map(|entry| {
            let item_name = world
                .item_by_id(entry.item_id)
                .and_then(|item| resolve_zh_clean(world.text_resolver(), &item.name));
            json!({
                "item_id": entry.item_id,
                "item_name": item_name,
                "price_ratio": entry.price_ratio,
            })
        })
        .collect();

    tool_ok(
        summary,
        json!({
            "shop_id": shop.id,
            "row_key": shop.row_key,
            "name": name,
            "items": items,
            "sell_items": sell_items,
        }),
    )
}

pub(crate) fn format_shop_candidates(query: &str, matches: &[&Shop]) -> ToolOutput {
    let mut lines = vec![format!(
        "店名「{query}」匹配到 {} 家商店候选，请改用 `shop_id` 精确查询：",
        matches.len()
    )];
    let mut candidates = Vec::with_capacity(matches.len());
    for shop in matches {
        lines.push(format!(
            "- shop_id={}「{query}」（商品 {} 件）",
            shop.id,
            shop.items.len()
        ));
        candidates.push(json!({
            "shop_id": shop.id,
            "row_key": shop.row_key,
            "name": query,
            "item_count": shop.items.len(),
        }));
    }
    tool_ok(
        lines.join("\n"),
        json!({
            "ambiguous": true,
            "query": query,
            "candidates": candidates,
        }),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SearchKind {
    Chapter,
    Map,
    Enemy,
    Skill,
    Item,
    Quest,
    Shop,
}

impl SearchKind {
    const ALL: [Self; 7] = [
        Self::Chapter,
        Self::Map,
        Self::Enemy,
        Self::Skill,
        Self::Item,
        Self::Quest,
        Self::Shop,
    ];

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "chapter" => Some(Self::Chapter),
            "map" => Some(Self::Map),
            "enemy" => Some(Self::Enemy),
            "skill" => Some(Self::Skill),
            "item" => Some(Self::Item),
            "quest" => Some(Self::Quest),
            "shop" => Some(Self::Shop),
            _ => None,
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Chapter => "chapter",
            Self::Map => "map",
            Self::Enemy => "enemy",
            Self::Skill => "skill",
            Self::Item => "item",
            Self::Quest => "quest",
            Self::Shop => "shop",
        }
    }

    pub(crate) const fn label_zh(self) -> &'static str {
        match self {
            Self::Chapter => "章节",
            Self::Map => "地图",
            Self::Enemy => "敌人",
            Self::Skill => "技能",
            Self::Item => "道具",
            Self::Quest => "任务",
            Self::Shop => "商店",
        }
    }
}

pub(crate) struct SearchHit {
    pub(crate) rank: u8,
    pub(crate) kind: SearchKind,
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) summary: String,
}

fn handle_search_data(
    cache: &GameDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("search_data", args, &["query", "kinds"])?;
    let query = required_non_empty_str("search_data", args, "query")?;
    let (kinds, explicit_kinds) = parse_search_kinds(args)?;

    cache.require_data_dir()?;

    let needs_scenario = kinds.contains(&SearchKind::Chapter);
    let needs_world = kinds.iter().any(|k| {
        matches!(
            k,
            SearchKind::Map | SearchKind::Item | SearchKind::Quest | SearchKind::Shop
        )
    });
    let needs_battle = kinds
        .iter()
        .any(|k| matches!(k, SearchKind::Enemy | SearchKind::Skill));

    let mut warnings = Vec::new();
    let scenario = if needs_scenario {
        match cache.scenario() {
            Ok(data) => Some(data),
            Err(err) if explicit_kinds => return Err(err),
            Err(err) => {
                warnings.push(err);
                None
            }
        }
    } else {
        None
    };
    let world = if needs_world {
        match cache.world() {
            Ok(data) => Some(data),
            Err(err) if explicit_kinds => return Err(err),
            Err(err) => {
                warnings.push(err);
                None
            }
        }
    } else {
        None
    };
    let battle = if needs_battle {
        match cache.battle() {
            Ok(data) => Some(data),
            Err(err) if explicit_kinds => return Err(err),
            Err(err) => {
                warnings.push(err);
                None
            }
        }
    } else {
        None
    };

    if scenario.is_none() && world.is_none() && battle.is_none() {
        return Err(format!(
            "无法加载任何可检索的游戏数据：{}",
            warnings.join("；")
        ));
    }

    let mut hits = Vec::new();
    if let Some(scenario) = &scenario
        && kinds.contains(&SearchKind::Chapter)
    {
        for ch in &scenario.chapters {
            if let Some(name) = scenario
                .chapter_name(ch.chapter_no)
                .text
                .and_then(|t| collapse_ws(&t))
                && let Some(rank) = match_rank(&name, query)
            {
                let count = scenario.chapter_main_scenario(ch.chapter_no).len();
                hits.push(SearchHit {
                    rank,
                    kind: SearchKind::Chapter,
                    id: ch.chapter_no,
                    summary: format!("第 {} 章，主线 {} 条", ch.chapter_no, count),
                    name,
                });
            }
        }
    }

    if let Some(world) = &world {
        if kinds.contains(&SearchKind::Map) {
            for map in world.maps() {
                if let Some(name) = resolve_zh_clean(world.text_resolver(), &map.name)
                    && let Some(rank) = match_rank(&name, query)
                {
                    hits.push(SearchHit {
                        rank,
                        kind: SearchKind::Map,
                        id: map.id,
                        summary: format!("关卡 {}", map.level_name),
                        name,
                    });
                }
            }
        }
        if kinds.contains(&SearchKind::Item) {
            for item in world.items() {
                if let Some(name) = resolve_zh_clean(world.text_resolver(), &item.name)
                    && let Some(rank) = match_rank(&name, query)
                {
                    hits.push(SearchHit {
                        rank,
                        kind: SearchKind::Item,
                        id: item.id,
                        summary: format!(
                            "{}，买价 {}",
                            item_category_zh(item.category),
                            item.buying_price
                        ),
                        name,
                    });
                }
            }
        }
        if kinds.contains(&SearchKind::Quest) {
            for quest in world.quests() {
                if let Some(name) = resolve_zh_clean(world.text_resolver(), &quest.title)
                    && let Some(rank) = match_rank(&name, query)
                {
                    let ch_label = quest
                        .chapter_no
                        .map_or_else(|| "章节未定".to_owned(), |c| format!("第 {c} 章"));
                    hits.push(SearchHit {
                        rank,
                        kind: SearchKind::Quest,
                        id: quest.id,
                        summary: ch_label,
                        name,
                    });
                }
            }
        }
        if kinds.contains(&SearchKind::Shop) {
            for shop in world.shops() {
                if let Some(name) = resolve_zh_clean(world.text_resolver(), &shop.name)
                    && let Some(rank) = match_rank(&name, query)
                {
                    hits.push(SearchHit {
                        rank,
                        kind: SearchKind::Shop,
                        id: shop.id,
                        summary: format!("商品 {} 件", shop.items.len()),
                        name,
                    });
                }
            }
        }
    }

    if let Some(battle) = &battle {
        if kinds.contains(&SearchKind::Enemy) {
            for unit in &battle.units {
                if !unit.category.is_enemy_category() {
                    continue;
                }
                if let Some(name) = battle
                    .unit_name_in(unit.id, MODEL_LANGUAGE)
                    .text
                    .and_then(|t| collapse_ws(&t))
                    && let Some(rank) = match_rank(&name, query)
                {
                    let role = if unit.is_boss { "Boss" } else { "普通敌人" };
                    hits.push(SearchHit {
                        rank,
                        kind: SearchKind::Enemy,
                        id: unit.id,
                        summary: format!("Lv.{}，{role}，HP {}", unit.level, unit.max_hp),
                        name,
                    });
                }
            }
        }
        if kinds.contains(&SearchKind::Skill) {
            for skill in &battle.skills {
                if let Some(name) = battle
                    .skill_name_in(skill.id, MODEL_LANGUAGE)
                    .text
                    .and_then(|t| collapse_ws(&t))
                    && let Some(rank) = match_rank(&name, query)
                {
                    hits.push(SearchHit {
                        rank,
                        kind: SearchKind::Skill,
                        id: skill.id,
                        summary: format!(
                            "{}属性，威力 {}，破防 {}",
                            element_zh(skill.element),
                            skill.power,
                            skill.break_damage
                        ),
                        name,
                    });
                }
            }
        }
    }

    hits.sort_by_key(|a| (a.rank, a.kind, a.id));
    let total_matches = hits.len();
    let truncated = total_matches > SEARCH_MAX_RESULTS;
    hits.truncate(SEARCH_MAX_RESULTS);

    let mut lines = if total_matches == 0 {
        vec![format!("未找到包含「{query}」的简体中文数据条目。")]
    } else if truncated {
        vec![format!(
            "搜索「{query}」共命中 {total_matches} 条，仅展示前 {SEARCH_MAX_RESULTS} 条："
        )]
    } else {
        vec![format!("搜索「{query}」共命中 {total_matches} 条：")]
    };

    let mut results_json = Vec::with_capacity(hits.len());
    for hit in &hits {
        lines.push(format!(
            "- [{}] #{}「{}」（{}）",
            hit.kind.label_zh(),
            hit.id,
            hit.name,
            hit.summary
        ));
        results_json.push(json!({
            "kind": hit.kind.as_str(),
            "id": hit.id,
            "name": hit.name,
            "summary": hit.summary,
        }));
    }
    if !warnings.is_empty() {
        lines.push(format!("提示：部分数据表未加载（{}）", warnings.join("；")));
    }

    let kind_names: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
    let mut payload = json!({
        "query": query,
        "kinds": kind_names,
        "total_matches": total_matches,
        "truncated": truncated,
        "results": results_json,
    });
    if !warnings.is_empty()
        && let Some(obj) = payload.as_object_mut()
    {
        obj.insert("warnings".to_owned(), json!(warnings));
    }

    Ok(tool_ok(lines.join("\n"), payload))
}

pub(crate) fn parse_search_kinds(
    args: &Map<String, Value>,
) -> Result<(Vec<SearchKind>, bool), String> {
    let Some(raw) = args.get("kinds") else {
        return Ok((SearchKind::ALL.to_vec(), false));
    };
    if raw.is_null() {
        return Ok((SearchKind::ALL.to_vec(), false));
    }
    let Some(arr) = raw.as_array() else {
        return Err("工具 `search_data` 的参数 `kinds` 必须是字符串数组".to_owned());
    };
    if arr.is_empty() {
        return Err(
            "工具 `search_data` 的参数 `kinds` 不能为空数组，请至少指定一个类别或省略该参数"
                .to_owned(),
        );
    }
    let mut parsed = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(text) = item.as_str() else {
            return Err("工具 `search_data` 的参数 `kinds` 只能包含字符串".to_owned());
        };
        let Some(kind) = SearchKind::parse(text) else {
            return Err(format!(
                "工具 `search_data` 不支持类别 `{text}`，可选值为：chapter、map、enemy、skill、item、quest、shop"
            ));
        };
        if !parsed.contains(&kind) {
            parsed.push(kind);
        }
    }
    Ok((parsed, true))
}

pub(crate) fn match_rank(candidate: &str, needle: &str) -> Option<u8> {
    if candidate == needle {
        Some(0)
    } else if candidate.starts_with(needle) {
        Some(1)
    } else if candidate.contains(needle) {
        Some(2)
    } else {
        None
    }
}

// ---- Shared formatting & argument validation helpers ----

pub(crate) fn tool_ok(summary: String, payload: Value) -> ToolOutput {
    ToolOutput::new(vec![Content::text(summary), Content::json(payload)])
}

pub(crate) fn tool_error(message: String) -> ToolOutput {
    let mut output = ToolOutput::new(vec![Content::text(message)]);
    output.is_error = true;
    output
}

pub(crate) fn reject_unknown_keys(
    tool: &str,
    args: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(), String> {
    let unknown: Vec<&str> = args
        .keys()
        .map(String::as_str)
        .filter(|key| !allowed.contains(key))
        .collect();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "工具 `{tool}` 收到未知参数：{}",
            unknown.join("、")
        ))
    }
}

pub(crate) fn optional_i64(
    tool: &str,
    args: &Map<String, Value>,
    key: &str,
) -> Result<Option<i64>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_i64()
        .map(Some)
        .ok_or_else(|| format!("工具 `{tool}` 的参数 `{key}` 必须是整数"))
}

pub(crate) fn required_i64(
    tool: &str,
    args: &Map<String, Value>,
    key: &str,
) -> Result<i64, String> {
    optional_i64(tool, args, key)?
        .ok_or_else(|| format!("工具 `{tool}` 缺少必需的整数参数 `{key}`"))
}

pub(crate) fn optional_non_empty_str<'a>(
    tool: &str,
    args: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(text) = value.as_str() else {
        return Err(format!("工具 `{tool}` 的参数 `{key}` 必须是字符串"));
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(format!("工具 `{tool}` 的参数 `{key}` 不能为空字符串"));
    }
    Ok(Some(trimmed))
}

pub(crate) fn required_non_empty_str<'a>(
    tool: &str,
    args: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    optional_non_empty_str(tool, args, key)?
        .ok_or_else(|| format!("工具 `{tool}` 缺少必需的非空字符串参数 `{key}`"))
}

pub(crate) fn resolve_zh_clean(resolver: &TextResolver, value: &FTextValue) -> Option<String> {
    let resolved = resolver.resolve_with(value, Some(MODEL_LANGUAGE), ResolveOptions::strict());
    clean_optional(resolved.text)
}

fn clean_optional(raw: Option<String>) -> Option<String> {
    raw.and_then(|text| collapse_ws(&text))
}

pub(crate) fn collapse_ws(text: &str) -> Option<String> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn format_scenario_destinations(
    entry: &MainScenarioEntry,
    world: Option<&WorldData>,
) -> (Vec<String>, Vec<Value>) {
    let mut labels = Vec::with_capacity(entry.destinations.len());
    let mut json_list = Vec::with_capacity(entry.destinations.len());
    for dest in &entry.destinations {
        let map_name = world.and_then(|w| {
            w.map_by_id(dest.destination_id)
                .and_then(|m| resolve_zh_clean(w.text_resolver(), &m.name))
        });
        let label = match &map_name {
            Some(name) => format!("地图 #{}「{}」", dest.destination_id, name),
            None => format!("地图 #{}", dest.destination_id),
        };
        labels.push(label);
        json_list.push(json!({
            "map_id": dest.destination_id,
            "map_name": map_name,
            "point_type": dest.point_type,
            "destination_type": dest.destination_type,
            "is_point_to_object": dest.is_point_to_object,
            "object_type": dest.object_type,
            "object_id": dest.object_id,
            "condition": dest.condition,
        }));
    }
    (labels, json_list)
}

fn describe_treasure_box(world: &WorldData, chest: &ffr_data::world::TreasureBox) -> String {
    let mut parts = Vec::new();
    if let Some(item_id) = chest.item_id {
        let name = world
            .item_by_id(item_id)
            .and_then(|item| resolve_zh_clean(world.text_resolver(), &item.name))
            .unwrap_or_else(|| format!("道具 #{item_id}"));
        parts.push(format!("{name}（#{item_id}）×{}", chest.item_num));
    }
    if let Some(money) = chest.money {
        parts.push(format!("{money}G"));
    }
    if chest.is_trap {
        match chest.battle_group_id {
            Some(group_id) => parts.push(format!("陷阱箱/战斗组 #{group_id}")),
            None => parts.push("陷阱箱".to_owned()),
        }
    }
    if parts.is_empty() {
        "空宝箱".to_owned()
    } else {
        parts.join("，")
    }
}

fn format_fallback_enemy_summary(
    battle: &BattleData,
    profile: &EnemyProfile,
    name: Option<&str>,
) -> String {
    let name_display = name.unwrap_or("无中文译名");
    let boss_label = if profile.is_boss {
        "Boss"
    } else {
        match profile.category {
            UnitCategory::Boss => "非Boss（大型/稀有分类）",
            UnitCategory::Enemy => "普通敌人",
            UnitCategory::Playable => "可操控角色",
            UnitCategory::Beast => "召唤兽",
            UnitCategory::Vision => "幻兵",
        }
    };
    let weak_text = join_elements_zh(&profile.weaknesses);
    let mut resist_parts = Vec::new();
    if !profile.halved.is_empty() {
        resist_parts.push(format!("减半({})", join_elements_zh(&profile.halved)));
    }
    if !profile.invalid.is_empty() {
        resist_parts.push(format!("无效({})", join_elements_zh(&profile.invalid)));
    }
    if !profile.absorb.is_empty() {
        resist_parts.push(format!("吸收({})", join_elements_zh(&profile.absorb)));
    }
    let resist_text = if resist_parts.is_empty() {
        "无".to_owned()
    } else {
        resist_parts.join("；")
    };
    let mult = battle
        .break_calc
        .break_element_multipliers
        .get(&Resistance::Weakness)
        .copied()
        .unwrap_or(1.0);
    format!(
        "{name_display}（编号 {}，Lv.{}，{boss_label}）：HP {}，破防槽（盾值）{}（弱点破防倍率 {:.1}x，破防增伤 {:.0}%）；弱点：{}；抗性：{}。",
        profile.unit_id,
        profile.level,
        profile.max_hp,
        profile.max_break_point,
        mult,
        battle.break_calc.break_damage_magnification,
        weak_text,
        resist_text,
    )
}

fn fuzzy_enemy_hints(battle: &BattleData, query: &str, limit: usize) -> Vec<String> {
    battle
        .units
        .iter()
        .filter(|unit| unit.category.is_enemy_category())
        .filter_map(|unit| {
            let zh = battle
                .unit_name_in(unit.id, MODEL_LANGUAGE)
                .text
                .and_then(|t| collapse_ws(&t))?;
            zh.contains(query).then(|| format!("{zh}（#{}）", unit.id))
        })
        .take(limit)
        .collect()
}

fn fuzzy_skill_hints(battle: &BattleData, query: &str, limit: usize) -> Vec<String> {
    battle
        .skills
        .iter()
        .filter_map(|skill| {
            let zh = battle
                .skill_name_in(skill.id, MODEL_LANGUAGE)
                .text
                .and_then(|t| collapse_ws(&t))?;
            zh.contains(query).then(|| format!("{zh}（#{}）", skill.id))
        })
        .take(limit)
        .collect()
}

pub(crate) fn element_zh(element: Element) -> &'static str {
    element.localized_name(MODEL_LANGUAGE).unwrap_or("未知属性")
}

pub(crate) fn element_code(element: Element) -> &'static str {
    match element {
        Element::Fire => "Fire",
        Element::Ice => "Ice",
        Element::Wind => "Wind",
        Element::Earth => "Earth",
        Element::Thunder => "Thunder",
        Element::Water => "Water",
        Element::Light => "Light",
        Element::Dark => "Dark",
        Element::None => "None",
    }
}

pub(crate) fn elements_zh_vec(elements: &[Element]) -> Vec<&'static str> {
    elements.iter().copied().map(element_zh).collect()
}

pub(crate) fn elements_code_vec(elements: &[Element]) -> Vec<&'static str> {
    elements.iter().copied().map(element_code).collect()
}

pub(crate) fn join_elements_zh(elements: &[Element]) -> String {
    if elements.is_empty() {
        "无".to_owned()
    } else {
        elements_zh_vec(elements).join("、")
    }
}

pub(crate) fn unit_category_code(category: UnitCategory) -> &'static str {
    match category {
        UnitCategory::Playable => "Playable",
        UnitCategory::Enemy => "Enemy",
        UnitCategory::Boss => "Boss",
        UnitCategory::Beast => "Beast",
        UnitCategory::Vision => "Vision",
    }
}

pub(crate) fn damage_type_zh(kind: SkillDamageType) -> &'static str {
    match kind {
        SkillDamageType::Physic => "物理伤害",
        SkillDamageType::Magic => "魔法伤害",
        SkillDamageType::None => "无伤害类型",
    }
}

pub(crate) fn damage_type_code(kind: SkillDamageType) -> &'static str {
    match kind {
        SkillDamageType::Physic => "Physic",
        SkillDamageType::Magic => "Magic",
        SkillDamageType::None => "None",
    }
}

pub(crate) fn skill_attr_zh(attr: SkillAttributeType) -> &'static str {
    match attr {
        SkillAttributeType::Fight => "普通攻击",
        SkillAttributeType::ChaseAttack => "追击",
        SkillAttributeType::Ability => "特技",
        SkillAttributeType::Magic => "魔法",
        SkillAttributeType::LimitBurst => "极限技",
        SkillAttributeType::Item => "道具",
        SkillAttributeType::Summon => "召唤",
        SkillAttributeType::MixItem => "调合",
        SkillAttributeType::Throw => "投掷",
        SkillAttributeType::ThrowMoney => "投钱",
        SkillAttributeType::Dualcast => "连续魔法",
        SkillAttributeType::Mimic => "模仿",
        SkillAttributeType::MagicSword => "魔法剑",
        SkillAttributeType::Escape => "逃跑",
        SkillAttributeType::Mechabo => "机械陆行鸟",
        SkillAttributeType::FinishBlow => "终结技",
    }
}

pub(crate) fn skill_attr_code(attr: SkillAttributeType) -> &'static str {
    match attr {
        SkillAttributeType::Fight => "Fight",
        SkillAttributeType::ChaseAttack => "ChaseAttack",
        SkillAttributeType::Ability => "Ability",
        SkillAttributeType::Magic => "Magic",
        SkillAttributeType::LimitBurst => "LimitBurst",
        SkillAttributeType::Item => "Item",
        SkillAttributeType::Summon => "Summon",
        SkillAttributeType::MixItem => "MixItem",
        SkillAttributeType::Throw => "Throw",
        SkillAttributeType::ThrowMoney => "ThrowMoney",
        SkillAttributeType::Dualcast => "Dualcast",
        SkillAttributeType::Mimic => "Mimic",
        SkillAttributeType::MagicSword => "MagicSword",
        SkillAttributeType::Escape => "Escape",
        SkillAttributeType::Mechabo => "Mechabo",
        SkillAttributeType::FinishBlow => "FinishBlow",
    }
}

pub(crate) fn cost_type_code(cost_type: SkillCostType) -> &'static str {
    match cost_type {
        SkillCostType::Mp => "Mp",
        SkillCostType::Lb => "Lb",
        SkillCostType::Item => "Item",
        SkillCostType::ThrowItem => "ThrowItem",
        SkillCostType::ThrowMoney => "ThrowMoney",
        SkillCostType::HpRatio => "HpRatio",
        SkillCostType::GoodLuck => "GoodLuck",
        SkillCostType::OverClock => "OverClock",
    }
}

pub(crate) fn format_skill_cost(cost: i64, cost_type: SkillCostType) -> String {
    if cost == 0 && cost_type == SkillCostType::Mp {
        return "无消耗".to_owned();
    }
    match cost_type {
        SkillCostType::Mp => format!("{cost} MP"),
        SkillCostType::Lb => format!("{cost} LB"),
        SkillCostType::Item => format!("消耗道具 {cost}"),
        SkillCostType::ThrowItem => format!("投掷道具 {cost}"),
        SkillCostType::ThrowMoney => format!("{cost}G"),
        SkillCostType::HpRatio => format!("{cost}% HP"),
        SkillCostType::GoodLuck => format!("好运 {cost}"),
        SkillCostType::OverClock => format!("超频 {cost}"),
    }
}

pub(crate) fn target_type_code(target_type: SkillTargetType) -> &'static str {
    match target_type {
        SkillTargetType::Single => "Single",
        SkillTargetType::Group => "Group",
        SkillTargetType::All => "All",
        SkillTargetType::Random => "Random",
        SkillTargetType::SelfTarget => "Self",
        SkillTargetType::SingleAndGroup => "SingleAndGroup",
    }
}

pub(crate) fn target_relation_code(relation: TargetRelation) -> &'static str {
    match relation {
        TargetRelation::Enemies => "Enemies",
        TargetRelation::Friendlies => "Friendlies",
    }
}

pub(crate) fn target_state_code(state: TargetState) -> &'static str {
    match state {
        TargetState::Alive => "Alive",
        TargetState::Dead => "Dead",
    }
}

pub(crate) fn format_skill_target(
    relation: TargetRelation,
    target_type: SkillTargetType,
    state: TargetState,
) -> String {
    let rel = match relation {
        TargetRelation::Enemies => "敌方",
        TargetRelation::Friendlies => "我方",
    };
    let scope = match target_type {
        SkillTargetType::Single => "单体",
        SkillTargetType::Group => "群体",
        SkillTargetType::All => "全体",
        SkillTargetType::Random => "随机",
        SkillTargetType::SelfTarget => "自身",
        SkillTargetType::SingleAndGroup => "单体或群体",
    };
    let st = match state {
        TargetState::Alive => "存活",
        TargetState::Dead => "无法战斗",
    };
    if target_type == SkillTargetType::SelfTarget {
        format!("自身（{st}）")
    } else {
        format!("{rel}{scope}（{st}）")
    }
}

pub(crate) fn available_location_zh(loc: AvailableLocation) -> &'static str {
    match loc {
        AvailableLocation::Anywhere => "任意场合",
        AvailableLocation::BattleOnly => "仅战斗",
    }
}

pub(crate) fn available_location_code(loc: AvailableLocation) -> &'static str {
    match loc {
        AvailableLocation::Anywhere => "Anywhere",
        AvailableLocation::BattleOnly => "BattleOnly",
    }
}

pub(crate) fn item_category_zh(category: ItemCategory) -> &'static str {
    match category {
        ItemCategory::Consumable => "消耗品",
        ItemCategory::Seed => "种子",
        ItemCategory::Equipment => "装备",
        ItemCategory::Vision => "幻兵",
        ItemCategory::Valuable => "贵重品",
        ItemCategory::Artifact => "圣遗物",
    }
}

pub(crate) fn facility_zh(raw: &str) -> &str {
    match raw {
        "eMapInteractType::Inn" => "旅店",
        "eMapInteractType::Shop" => "道具店",
        "eMapInteractType::Shop_Weapon" => "武器店",
        "eMapInteractType::Shop_Armor" => "防具店",
        "eMapInteractType::Shop_Accessory" => "饰品店",
        "eMapInteractType::Shop_Vision" => "幻兵店",
        other => other.rsplit_once("::").map_or(other, |(_, suffix)| suffix),
    }
}
