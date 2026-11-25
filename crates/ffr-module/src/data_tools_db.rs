//! `ffr-db` SQLite backend implementations for all 9 `ToolCategory::Data` tools.
//!
//! Produces output (both Chinese summary text and structured JSON) strictly identical to the
//! legacy `GameDataCache` JSON backend in [`crate::data_tools`], while querying only indexed
//! records via [`GameDb`] without loading whole tables into memory.

use std::collections::{BTreeSet, HashMap};

use agp_game::ToolOutput;
use agp_platform::FileChannel;
use ffr_data::assets::level::{LevelObjectData, LevelObjectKind, MapEdgeKind};
use ffr_data::battle::{
    BATTLE_GROUP_TABLE, BreakCalcParams, EnemyProfile, Resistance, Skill, Unit, UnitAiProfile,
    UnitCategory,
};
use ffr_data::world::{Item, ItemCategory, ItemDetail, MapInfo, Quest, Shop, TreasureBox};
use ffr_data::{
    DataError, FTextValue, MODEL_LANGUAGE, MainScenarioEntry, QUEST_MENU_STRING_TABLE,
    ResolveOptions, main_quest_id_for_chapter,
};
use ffr_db::GameDb;
use ffr_db::query::world::{MapContents, ResolvedShop};
use serde_json::{Map, Value, json};

use crate::data_cache::DbDataCache;
use crate::data_tools::{
    SEARCH_MAX_RESULTS, SearchHit, SearchKind, available_location_code, available_location_zh,
    build_enemy_drops_and_steals, collapse_ws, cost_type_code, damage_type_code, damage_type_zh,
    element_code, element_zh, elements_code_vec, elements_zh_vec, facility_zh,
    format_enemy_battle_groups_slice, format_enemy_candidates, format_item_candidates,
    format_quest_candidates, format_shop_candidates, format_skill_candidates, format_skill_cost,
    format_skill_target, item_category_zh, join_elements_zh, match_rank, optional_i64,
    optional_non_empty_str, parse_search_kinds, reject_unknown_keys, required_i64,
    required_non_empty_str, skill_attr_code, skill_attr_zh, target_relation_code,
    target_state_code, target_type_code, tool_ok, unit_category_code,
};
use crate::data_tools_ffr3::{
    collect_map_npcs_for_read_map, format_map_id_label, format_vec3_zh, handle_find_route_db,
    handle_query_game_db_db, handle_read_npc_db, handle_read_talk_db, handle_whats_near_db,
    handle_where_to_get_db, map_direction_enum_zh, map_edge_kind_zh, vec3_json,
};

/// Dispatch one data-category tool call against [`DbDataCache`].
pub(crate) async fn call_data_tool_db(
    channel: &FileChannel,
    cache: &DbDataCache,
    name: &str,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    match name {
        "read_chapter" => handle_read_chapter_db(cache, args),
        "read_map" => handle_read_map_db(cache, args),
        "read_enemy" => handle_read_enemy_db(cache, args),
        "read_battle_group" => handle_read_battle_group_db(cache, args),
        "read_skill" => handle_read_skill_db(cache, args),
        "read_item" => handle_read_item_db(cache, args),
        "read_quest" => handle_read_quest_db(cache, args),
        "read_shop" => handle_read_shop_db(cache, args),
        "search_data" => handle_search_data_db(cache, args),
        "find_route" => handle_find_route_db(cache, args),
        "where_to_get" => handle_where_to_get_db(cache, args),
        "read_npc" => handle_read_npc_db(cache, args),
        "read_talk" => handle_read_talk_db(cache, args),
        "whats_near" => handle_whats_near_db(channel, cache, args).await,
        "query_game_db" => handle_query_game_db_db(cache, args),
        _ => Err(format!("未知数据工具 `{name}`")),
    }
}

/// Strictly resolve an [`FTextValue`] to raw `"zh-Hans"` text through `GameDb` without fallback.
pub(crate) fn resolve_db_zh_raw(db: &GameDb, value: &FTextValue) -> Option<String> {
    db.resolve_ftext(value, MODEL_LANGUAGE, ResolveOptions::strict())
        .ok()
        .and_then(|res| res.text)
}

/// Strictly resolve an [`FTextValue`] to whitespace-collapsed `"zh-Hans"` text through `GameDb`.
pub(crate) fn resolve_db_zh_clean(db: &GameDb, value: &FTextValue) -> Option<String> {
    resolve_db_zh_raw(db, value).and_then(|t| collapse_ws(&t))
}

/// Format a main quest title (`ST_UI_QuestMenu/QuestChapter`) matching
/// [`ffr_data::ScenarioData::main_quest_title`].
pub(crate) fn format_db_main_quest_title(
    db: &GameDb,
    chapter_no: i64,
    chapter_name: &str,
) -> String {
    let format_ftext = FTextValue::StringTable {
        table_id: QUEST_MENU_STRING_TABLE.to_owned(),
        key: "QuestChapter".to_owned(),
        source_string: None,
        localized_string: None,
    };
    let template = resolve_db_zh_raw(db, &format_ftext);
    let pattern = template.as_deref().unwrap_or("第{Chapter}章{Name}");
    pattern
        .replace("{Chapter}", &chapter_no.to_string())
        .replace("{Name}", chapter_name)
}

// ---- Tool handlers ----

fn handle_read_chapter_db(
    cache: &DbDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("read_chapter", args, &["chapter"])?;
    let chapter = optional_i64("read_chapter", args, "chapter")?;
    if let Some(ch) = chapter
        && ch < 1
    {
        return Err("工具 `read_chapter` 的参数 `chapter` 必须大于等于 1".to_owned());
    }

    let db = cache.open()?;

    let Some(chapter_no) = chapter else {
        let chapters = db
            .chapters()
            .map_err(|err| format!("查询章节列表失败：{err}"))?;
        let counts = db
            .main_scenario_counts_by_chapter()
            .map_err(|err| format!("统计各章主线条数失败：{err}"))?;
        let mut lines = vec![format!("共 {} 个章节：", chapters.len())];
        let mut items = Vec::with_capacity(chapters.len());
        for ch in &chapters {
            let name = resolve_db_zh_raw(&db, &ch.name);
            let entry_count = counts.get(&ch.chapter_no).copied().unwrap_or(0);
            let name_display = name.as_deref().unwrap_or("无中文译名");
            lines.push(format!(
                "- 第 {} 章「{}」（主线 {} 条）",
                ch.chapter_no, name_display, entry_count
            ));
            let main_quest_id = main_quest_id_for_chapter(ch.chapter_no);
            let main_quest_title = main_quest_id.and_then(|_| {
                name.as_deref()
                    .map(|n| format_db_main_quest_title(&db, ch.chapter_no, n))
            });
            items.push(json!({
                "chapter_no": ch.chapter_no,
                "main_quest_id": main_quest_id,
                "main_quest_title": main_quest_title,
                "row_key": ch.row_key,
                "name": name,
                "main_scenario_count": entry_count,
            }));
        }
        return Ok(tool_ok(lines.join("\n"), json!({ "chapters": items })));
    };

    let Some(ch) = db
        .chapter(chapter_no)
        .map_err(|err| format!("查询第 {chapter_no} 章失败：{err}"))?
    else {
        return Err(format!("章节表中找不到第 {chapter_no} 章"));
    };
    let name = resolve_db_zh_raw(&db, &ch.name);
    let name_display = name.as_deref().unwrap_or("无中文译名");
    let entries = db
        .main_scenario_by_chapter(chapter_no)
        .map_err(|err| format!("查询第 {chapter_no} 章主线剧情失败：{err}"))?;

    let mut lines = vec![format!(
        "第 {} 章「{}」（共 {} 条主线剧情）：",
        chapter_no,
        name_display,
        entries.len()
    )];
    let mut json_entries = Vec::with_capacity(entries.len());
    let mut map_names: HashMap<i64, Option<String>> = HashMap::new();

    for entry in &entries {
        let goal_text = resolve_db_zh_clean(&db, &entry.text);
        let summary_text = resolve_db_zh_clean(&db, &entry.summary_text);
        let (dest_labels, dest_json) = format_scenario_destinations_db(&db, entry, &mut map_names)?;

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
    let main_quest_title = main_quest_id.and_then(|_| {
        name.as_deref()
            .map(|n| format_db_main_quest_title(&db, ch.chapter_no, n))
    });
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

fn handle_read_map_db(
    cache: &DbDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("read_map", args, &["map_id", "level_name"])?;
    let map_id = optional_i64("read_map", args, "map_id")?;
    let level_name = optional_non_empty_str("read_map", args, "level_name")?;

    let db = cache.open()?;
    let contents = match (map_id, level_name) {
        (Some(id), None) => db
            .map_contents(id)
            .map_err(|err| format!("查询地图 {id} 失败：{err}"))?
            .ok_or_else(|| format!("找不到编号为 {id} 的地图"))?,
        (None, Some(level)) => {
            let map = db
                .map_by_level(level)
                .map_err(|err| format!("按关卡名查询地图失败：{err}"))?
                .ok_or_else(|| format!("找不到关卡名为「{level}」的地图"))?;
            db.map_contents(map.id)
                .map_err(|err| format!("查询地图 {} 内容失败：{err}", map.id))?
                .ok_or_else(|| format!("找不到编号为 {} 的地图", map.id))?
        }
        (None, None) => {
            return Err("工具 `read_map` 必须提供 `map_id` 或 `level_name` 之一".to_owned());
        }
        (Some(_), Some(_)) => {
            return Err(
                "工具 `read_map` 的 `map_id` 与 `level_name` 不能同时传入，请二选一".to_owned(),
            );
        }
    };

    format_map_output_db(&db, &contents)
}

fn format_map_output_db(db: &GameDb, contents: &MapContents) -> Result<ToolOutput, String> {
    let map = &contents.map;
    let name = resolve_db_zh_clean(db, &map.name);
    let island_name = contents
        .island
        .as_ref()
        .and_then(|island| resolve_db_zh_clean(db, &island.name));

    let base_summary = build_map_summary_db(map, name.as_deref(), island_name.as_deref(), contents);
    let save_point = contents.save_point.as_ref();
    let teleport_points = &contents.teleport_points;
    let treasure_boxes = &contents.treasure_boxes;

    let shop_facilities: Vec<&str> = map
        .facilities
        .iter()
        .filter(|fac| fac.interact_type.starts_with("eMapInteractType::Shop"))
        .map(|fac| facility_zh(&fac.interact_type))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    // Enrich with `08_levels` data (exits, NPCs, shops, chest placements, save points, encounter zones).
    let exits_raw = db
        .map_exits(map.id)
        .map_err(|err| format!("查询地图 {} 出口失败：{err}", map.id))?;
    let npcs_json = collect_map_npcs_for_read_map(db, map.id)?;
    let shops_raw = db
        .shops_in_map(map.id)
        .map_err(|err| format!("查询地图 {} 商店列表失败：{err}", map.id))?;
    let chest_objs = db
        .map_objects(map.id, Some(LevelObjectKind::TreasureBox))
        .map_err(|err| format!("查询地图 {} 宝箱位置失败：{err}", map.id))?;
    let save_point_objs = db
        .map_objects(map.id, Some(LevelObjectKind::SavePoint))
        .map_err(|err| format!("查询地图 {} 存档点位置失败：{err}", map.id))?;
    let enc_zones = db
        .encounter_zones(map.id)
        .map_err(|err| format!("查询地图 {} 遇敌区域失败：{err}", map.id))?;

    let mut map_names: HashMap<i64, Option<String>> = HashMap::new();
    map_names.insert(map.id, name.clone());

    let mut exits_json = Vec::with_capacity(exits_raw.len());
    let mut exit_labels = Vec::new();
    for edge in &exits_raw {
        let target_name = match edge.target_map_id {
            Some(tid) => cached_map_name_clean(db, tid, &mut map_names)?,
            None => None,
        };
        let label = match (edge.kind, edge.target_map_id) {
            (MapEdgeKind::SavePointTeleport, Some(tid)) => format!(
                "{}（存档点传送）",
                format_map_id_label(tid, target_name.as_deref())
            ),
            (MapEdgeKind::SameMap, _) => "同图传送".to_owned(),
            (_, Some(tid)) => format_map_id_label(tid, target_name.as_deref()),
            (_, None) => "未知目标".to_owned(),
        };
        if !exit_labels.contains(&label) {
            exit_labels.push(label);
        }
        exits_json.push(json!({
            "kind": edge.kind.as_str(),
            "kind_zh": map_edge_kind_zh(edge.kind),
            "target_map_id": edge.target_map_id,
            "target_map_name": target_name,
            "target_point_id": edge.target_point_id,
            "target_location": edge.target_location.map(vec3_json),
            "target_direction": edge.target_direction,
            "target_direction_zh": edge.target_direction.as_deref().and_then(map_direction_enum_zh),
            "is_same_map": edge.is_same_map,
            "is_auto_transition": edge.is_auto_transition,
            "is_event_only": edge.is_event_only,
            "condition": edge.condition,
            "unique_id": edge.unique_id,
            "location": edge.location.map(vec3_json),
        }));
    }

    let mut shops_json = Vec::with_capacity(shops_raw.len());
    let mut shop_labels = Vec::with_capacity(shops_raw.len());
    for entry in &shops_raw {
        let shop_name = db
            .shop(entry.shop_id)
            .map_err(|err| format!("查询商店 {} 名称失败：{err}", entry.shop_id))?
            .and_then(|s| s.name_zh);
        let type_zh = entry.interact_type.as_deref().map(facility_zh);
        let mut details = Vec::new();
        if let Some(t) = type_zh {
            details.push(t.to_owned());
        }
        if let Some(uid) = entry.unique_id {
            details.push(format!("NPC unique_id={uid}"));
        }
        if let Some(vsid) = entry.various_shop_id {
            details.push(format!("综合商店 #{vsid}"));
        }
        if let Some(cond) = &entry.visible_condition {
            details.push(format!("条件：{cond}"));
        }
        let shop_title = match &shop_name {
            Some(n) => format!("#{}「{n}」", entry.shop_id),
            None => format!("#{}", entry.shop_id),
        };
        let detail_part = if details.is_empty() {
            String::new()
        } else {
            format!("（{}）", details.join("，"))
        };
        shop_labels.push(format!("{shop_title}{detail_part}"));
        shops_json.push(json!({
            "shop_id": entry.shop_id,
            "shop_name": shop_name,
            "various_shop_id": entry.various_shop_id,
            "unique_id": entry.unique_id,
            "map_unit_id": entry.map_unit_id,
            "interact_type": entry.interact_type,
            "interact_type_zh": type_zh,
            "visible_condition": entry.visible_condition,
            "location": entry.location.map(vec3_json),
        }));
    }

    let mut save_points_json = Vec::with_capacity(save_point_objs.len());
    let mut save_point_labels = Vec::with_capacity(save_point_objs.len());
    for obj in &save_point_objs {
        let LevelObjectData::SavePoint(sp) = &obj.data else {
            continue;
        };
        let target_map_id = sp.target_map_id.or(sp.transition_map_id);
        let target_map_name = match target_map_id {
            Some(tid) => cached_map_name_clean(db, tid, &mut map_names)?,
            None => None,
        };
        let pos_str = obj
            .location
            .map_or_else(|| "坐标未知".to_owned(), format_vec3_zh);
        let tp_str = match target_map_id {
            Some(tid) => format!(
                " -> 传送至 {}",
                format_map_id_label(tid, target_map_name.as_deref())
            ),
            None => String::new(),
        };
        save_point_labels.push(format!("{pos_str}{tp_str}"));
        save_points_json.push(json!({
            "unique_id": obj.unique_id,
            "save_point_type": sp.save_point_type,
            "is_teleport": sp.is_teleport,
            "target_map_id": target_map_id,
            "target_map_name": target_map_name,
            "target_point_id": sp.target_point_id.or(sp.transition_point_id),
            "visible_condition": obj.visible_condition,
            "location": obj.location.map(vec3_json),
        }));
    }

    let encounter_zones_json: Vec<Value> = enc_zones
        .iter()
        .map(|ez| {
            json!({
                "id": ez.id,
                "kind": ez.kind,
                "priority": ez.priority,
                "terrain_id": ez.terrain_id,
                "battle_stage_id": ez.battle_stage_id,
                "encount_datatable": ez.encount_datatable,
                "terrain_datatables": ez.terrain_datatables,
                "visible_condition": ez.visible_condition,
                "flag_condition": ez.flag_condition,
                "location": ez.location.map(vec3_json),
            })
        })
        .collect();

    let mut summary_lines = vec![base_summary];
    if !shops_json.is_empty() {
        summary_lines.push(format!("商店：{}", shop_labels.join("；")));
    } else if !shop_facilities.is_empty() {
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

    let mut item_names: HashMap<i64, Option<String>> = HashMap::new();
    let mut treasures_json = Vec::with_capacity(treasure_boxes.len());
    if !treasure_boxes.is_empty() {
        let mut chest_parts = Vec::with_capacity(treasure_boxes.len());
        for chest in treasure_boxes {
            let content = describe_treasure_box_db(db, chest, &mut item_names)?;
            let placement = chest_objs.iter().find(|lo| {
                matches!(
                    &lo.data,
                    LevelObjectData::TreasureBox(tp) if tp.chest_id == Some(chest.id)
                )
            });
            let pos_suffix = placement
                .and_then(|p| p.location)
                .map_or_else(String::new, |p| format!(" @{}", format_vec3_zh(p)));
            chest_parts.push(format!("#{} {content}{pos_suffix}", chest.id));

            let item_name = match chest.item_id {
                Some(id) => cached_item_name_clean(db, id, &mut item_names)?,
                None => None,
            };
            treasures_json.push(json!({
                "box_id": chest.id,
                "unique_id": placement.and_then(|p| p.unique_id),
                "item_id": chest.item_id,
                "item_name": item_name,
                "item_num": chest.item_num,
                "money": chest.money,
                "is_trap": chest.is_trap,
                "battle_group_id": chest.battle_group_id,
                "battle_finish_event_id": chest.battle_finish_event_id,
                "location": placement.and_then(|p| p.location).map(vec3_json),
                "visible_condition": placement.and_then(|p| p.visible_condition.clone()),
            }));
        }
        summary_lines.push(format!("宝箱明细：{}", chest_parts.join("；")));
    }

    if !exit_labels.is_empty() {
        summary_lines.push(format!("出口：{}", exit_labels.join("、")));
    }
    if !npcs_json.is_empty() {
        summary_lines.push(format!("NPC：共 {} 名", npcs_json.len()));
    }
    if !save_point_labels.is_empty() {
        summary_lines.push(format!("存档点位置：{}", save_point_labels.join("；")));
    }
    if !encounter_zones_json.is_empty() {
        summary_lines.push(format!("遇敌区域：{} 处", encounter_zones_json.len()));
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

    Ok(tool_ok(
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
            "save_points": save_points_json,
            "teleportable": !teleport_points.is_empty(),
            "teleport_points": teleport_json,
            "exits": exits_json,
            "npcs": npcs_json,
            "shops": shops_json,
            "treasure_boxes": treasures_json,
            "encounter_zones": encounter_zones_json,
            "shop_facilities": shop_facilities,
            "facilities": facilities_json,
        }),
    ))
}

fn handle_read_enemy_db(
    cache: &DbDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("read_enemy", args, &["unit_id", "name"])?;
    let unit_id = optional_i64("read_enemy", args, "unit_id")?;
    let name = optional_non_empty_str("read_enemy", args, "name")?;

    let (db, avail) = cache.open_with_availability()?;
    match (unit_id, name) {
        (Some(id), None) => {
            let unit = db
                .unit(id)
                .map_err(|err| format!("查询单位 {id} 失败：{err}"))?
                .ok_or_else(|| format!("单位表中找不到编号为 {id} 的单位"))?;
            let ai = db
                .unit_ai(id)
                .map_err(|err| format!("查询单位 {id} AI 失败：{err}"))?;
            format_enemy_output_db(&db, &unit, ai.as_ref(), avail.enemy_chapters)
        }
        (None, Some(query_name)) => {
            let exact = db
                .enemies_by_exact_name_zh(query_name)
                .map_err(|err| format!("按名称查询敌人失败：{err}"))?;
            match exact.as_slice() {
                [] => {
                    let candidates = db
                        .units_by_name(query_name, MODEL_LANGUAGE)
                        .map_err(|err| format!("按名称模糊查询敌人失败：{err}"))?;
                    let similar: Vec<String> = candidates
                        .into_iter()
                        .filter(|u| u.category.is_enemy_category())
                        .filter_map(|u| {
                            let zh = resolve_db_zh_clean(&db, &u.name)?;
                            zh.contains(query_name)
                                .then(|| format!("{zh}（#{}）", u.id))
                        })
                        .take(5)
                        .collect();
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
                [(unit, ai)] => {
                    format_enemy_output_db(&db, unit, ai.as_ref(), avail.enemy_chapters)
                }
                many => {
                    let profiles: Vec<EnemyProfile> = many
                        .iter()
                        .map(|(u, ai)| u.to_enemy_profile(ai.as_ref()))
                        .collect();
                    Ok(format_enemy_candidates(query_name, &profiles))
                }
            }
        }
        (None, None) => Err("工具 `read_enemy` 必须提供 `unit_id` 或 `name` 之一".to_owned()),
        (Some(_), Some(_)) => {
            Err("工具 `read_enemy` 的 `unit_id` 与 `name` 不能同时传入，请二选一".to_owned())
        }
    }
}

fn format_enemy_output_db(
    db: &GameDb,
    unit: &Unit,
    ai: Option<&UnitAiProfile>,
    enemy_chapters_available: bool,
) -> Result<ToolOutput, String> {
    let profile = unit.to_enemy_profile(ai);
    let break_calc = db
        .break_calc()
        .map_err(|err| format!("查询破防参数失败：{err}"))?
        .ok_or_else(|| "数据库缺少战斗破防参数（battle_param）".to_owned())?;

    let resolved_name = db
        .resolve_ftext(&unit.name, MODEL_LANGUAGE, ResolveOptions::strict())
        .map_err(|err| format!("解析单位名称失败：{err}"))?;

    let mut high_risk_resolved: Vec<(i64, bool, Option<String>)> =
        Vec::with_capacity(profile.high_risk_skill_ids.len());
    for &skill_id in &profile.high_risk_skill_ids {
        match db
            .skill(skill_id)
            .map_err(|err| format!("查询技能 {skill_id} 失败：{err}"))?
        {
            Some(skill) => {
                let raw = resolve_db_zh_raw(db, &skill.name);
                high_risk_resolved.push((skill_id, true, raw));
            }
            None => high_risk_resolved.push((skill_id, false, None)),
        }
    }

    let mut summary = if let Some(display_name) = resolved_name.text.as_deref()
        && let Some(&weak_break_mult) = break_calc
            .break_element_multipliers
            .get(&Resistance::Weakness)
    {
        format_full_enemy_summary(
            &profile,
            display_name,
            weak_break_mult,
            break_calc.break_damage_magnification,
            &high_risk_resolved,
        )
    } else {
        format_fallback_enemy_summary_db(&break_calc, &profile, resolved_name.text.as_deref())
    };

    let chapters = if enemy_chapters_available {
        Some(
            db.chapters_of_enemy(unit.id)
                .map_err(|err| format!("查询敌人 {} 所属章节失败：{err}", unit.id))?,
        )
    } else {
        None
    };
    if let Some(ch_list) = &chapters
        && !ch_list.is_empty()
    {
        let joined: Vec<String> = ch_list.iter().map(|ch| format!("第 {ch} 章")).collect();
        summary.push_str(&format!("\n出没章节：{}", joined.join("、")));
    }

    let mut item_name_cache: HashMap<i64, Option<String>> = HashMap::new();
    let (drops_json, steals_json, drop_line, steal_line) =
        build_enemy_drops_and_steals(unit, |item_id| {
            cached_item_name_clean(db, item_id, &mut item_name_cache)
                .ok()
                .flatten()
        });
    if let Some(line) = drop_line {
        summary.push_str(&format!("\n{line}"));
    }
    if let Some(line) = steal_line {
        summary.push_str(&format!("\n{line}"));
    }

    let groups_owned = db
        .groups_containing(unit.id)
        .map_err(|err| format!("查询敌人 {} 所在战斗组失败：{err}", unit.id))?;
    let group_refs: Vec<&ffr_data::battle::BattleGroup> = groups_owned.iter().collect();
    let (battle_groups_json, groups_line) = format_enemy_battle_groups_slice(&group_refs);
    if let Some(line) = groups_line {
        summary.push_str(&format!("\n{line}"));
    }

    let high_risk_skills: Vec<Value> = high_risk_resolved
        .iter()
        .map(|(skill_id, _, raw)| {
            let skill_name = raw.as_deref().and_then(collapse_ws);
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
            "weakness_multiplier": break_calc
                .break_element_multipliers
                .get(&Resistance::Weakness)
                .copied(),
            "break_damage_magnification": break_calc.break_damage_magnification,
        },
        "high_risk_skills": high_risk_skills,
        "drops": drops_json,
        "steals": steals_json,
        "chapters": chapters,
        "battle_groups": battle_groups_json,
    });

    Ok(tool_ok(summary, payload))
}

fn handle_read_battle_group_db(
    cache: &DbDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("read_battle_group", args, &["group_id"])?;
    let group_id = required_i64("read_battle_group", args, "group_id")?;

    let db = cache.open()?;
    let Some(detail) = db
        .battle_group(group_id)
        .map_err(|err| format!("查询战斗组 {group_id} 失败：{err}"))?
    else {
        let err = DataError::InvalidSchema {
            path: BATTLE_GROUP_TABLE.to_owned(),
            field: format!("ID={group_id}"),
            detail: format!("战斗组表中找不到编号为 {group_id} 的战斗组"),
        };
        return Err(format!("读取战斗组 {group_id} 失败：{err}"));
    };

    let group = &detail.group;
    if group.unit_ids.is_empty() {
        let err = DataError::InvalidSchema {
            path: BATTLE_GROUP_TABLE.to_owned(),
            field: format!("Rows.{}.UnitIdList", group.row_key),
            detail: format!("战斗组 {group_id} 没有配置任何成员单位"),
        };
        return Err(format!("读取战斗组 {group_id} 失败：{err}"));
    }

    let mut members: Vec<(EnemyProfile, Option<String>)> = Vec::with_capacity(detail.members.len());
    let mut is_boss = false;
    for slot in &detail.members {
        let Some(unit) = &slot.unit else {
            let err = DataError::InvalidSchema {
                path: BATTLE_GROUP_TABLE.to_owned(),
                field: format!("Rows.{}.UnitIdList", group.row_key),
                detail: format!(
                    "战斗组 {group_id} 引用的单位编号 {} 在单位表中不存在",
                    slot.unit_id
                ),
            };
            return Err(format!("读取战斗组 {group_id} 失败：{err}"));
        };
        if unit.is_boss {
            is_boss = true;
        }
        let ai = db
            .unit_ai(unit.id)
            .map_err(|err| format!("查询单位 {} AI 失败：{err}", unit.id))?;
        let name = resolve_db_zh_clean(&db, &unit.name);
        members.push((unit.to_enemy_profile(ai.as_ref()), name));
    }

    let kind_label = if is_boss { "Boss 战" } else { "普通遭遇" };
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
        members.len()
    )];
    let mut members_json = Vec::with_capacity(members.len());

    for (member, name) in &members {
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

    Ok(tool_ok(
        lines.join("\n"),
        json!({
            "group_id": group.id,
            "row_key": group.row_key,
            "is_boss": is_boss,
            "can_escape": group.can_escape,
            "escape_probability": group.escape_probability,
            "is_bounty": group.is_bounty,
            "ap": group.ap,
            "unit_ids": group.unit_ids,
            "members": members_json,
        }),
    ))
}

fn handle_read_skill_db(
    cache: &DbDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("read_skill", args, &["skill_id", "name"])?;
    let skill_id = optional_i64("read_skill", args, "skill_id")?;
    let name = optional_non_empty_str("read_skill", args, "name")?;

    let db = cache.open()?;
    match (skill_id, name) {
        (Some(id), None) => {
            let skill = db
                .skill(id)
                .map_err(|err| format!("查询技能 {id} 失败：{err}"))?
                .ok_or_else(|| format!("技能表中找不到编号为 {id} 的技能"))?;
            Ok(format_skill_output_db(&db, &skill))
        }
        (None, Some(query_name)) => {
            let exact = db
                .skills_by_exact_name_zh(query_name)
                .map_err(|err| format!("按名称查询技能失败：{err}"))?;
            match exact.as_slice() {
                [] => {
                    let candidates = db
                        .skills_by_name(query_name, MODEL_LANGUAGE)
                        .map_err(|err| format!("按名称模糊查询技能失败：{err}"))?;
                    let similar: Vec<String> = candidates
                        .into_iter()
                        .filter_map(|skill| {
                            let zh = resolve_db_zh_clean(&db, &skill.name)?;
                            zh.contains(query_name)
                                .then(|| format!("{zh}（#{}）", skill.id))
                        })
                        .take(5)
                        .collect();
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
                [single] => Ok(format_skill_output_db(&db, single)),
                many => {
                    let refs: Vec<&Skill> = many.iter().collect();
                    Ok(format_skill_candidates(query_name, &refs))
                }
            }
        }
        (None, None) => Err("工具 `read_skill` 必须提供 `skill_id` 或 `name` 之一".to_owned()),
        (Some(_), Some(_)) => {
            Err("工具 `read_skill` 的 `skill_id` 与 `name` 不能同时传入，请二选一".to_owned())
        }
    }
}

fn format_skill_output_db(db: &GameDb, skill: &Skill) -> ToolOutput {
    let name = resolve_db_zh_clean(db, &skill.name);
    let description = resolve_db_zh_clean(db, &skill.description);
    let name_display = name.as_deref().unwrap_or("无中文译名");
    let element_label = element_zh(skill.element);
    let damage_label = damage_type_zh(skill.damage_type);
    let attr_label = skill_attr_zh(skill.skill_attr_type);
    let cost_label = format_skill_cost(skill.cost, skill.cost_type);
    let target_label = format_skill_target(
        skill.default_target_relation,
        skill.target_type,
        skill.default_target_state,
    );
    let loc_label = available_location_zh(skill.available_location);

    let mut summary = format!(
        "技能 #{}「{}」（{}/{}）；属性：{}；威力：{}；破防削减：{}；消耗：{}；目标：{}；场合：{}",
        skill.id,
        name_display,
        attr_label,
        damage_label,
        element_label,
        skill.power,
        skill.break_damage,
        cost_label,
        target_label,
        loc_label,
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
            "element": element_label,
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

fn handle_read_item_db(
    cache: &DbDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("read_item", args, &["item_id", "name"])?;
    let item_id = optional_i64("read_item", args, "item_id")?;
    let name = optional_non_empty_str("read_item", args, "name")?;

    let db = cache.open()?;
    match (item_id, name) {
        (Some(id), None) => {
            let item = db
                .item(id)
                .map_err(|err| format!("查询道具 {id} 失败：{err}"))?
                .ok_or_else(|| format!("道具表中找不到编号为 {id} 的道具"))?;
            format_item_output_db(&db, &item)
        }
        (None, Some(query_name)) => {
            let candidates = db
                .items_by_name(query_name, MODEL_LANGUAGE)
                .map_err(|err| format!("按名称查询道具失败：{err}"))?;
            let mut exact: Vec<&Item> = Vec::new();
            let mut contains: Vec<(&Item, String)> = Vec::new();
            for item in &candidates {
                let Some(raw) = resolve_db_zh_raw(&db, &item.name) else {
                    continue;
                };
                if raw.trim() == query_name {
                    exact.push(item);
                }
                if raw.contains(query_name) {
                    contains.push((item, raw));
                }
            }
            match exact.as_slice() {
                [] => {
                    let similar: Vec<String> = contains
                        .into_iter()
                        .take(5)
                        .filter_map(|(item, raw)| {
                            collapse_ws(&raw).map(|n| format!("{n}（#{}）", item.id))
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
                [single] => format_item_output_db(&db, single),
                many => Ok(format_item_candidates(query_name, many)),
            }
        }
        (None, None) => Err("工具 `read_item` 必须提供 `item_id` 或 `name` 之一".to_owned()),
        (Some(_), Some(_)) => {
            Err("工具 `read_item` 的 `item_id` 与 `name` 不能同时传入，请二选一".to_owned())
        }
    }
}

fn format_item_output_db(db: &GameDb, item: &Item) -> Result<ToolOutput, String> {
    let name = resolve_db_zh_clean(db, &item.name);
    let description = resolve_db_zh_clean(db, &item.description);
    let mut summary = build_item_summary_db(item, name.as_deref(), description.as_deref());
    let shops = db
        .shops_selling(item.id)
        .map_err(|err| format!("查询出售道具 {} 的商店失败：{err}", item.id))?;
    let sold_in_shop_ids: Vec<i64> = shops.iter().map(|shop| shop.id).collect();
    if !sold_in_shop_ids.is_empty() {
        let shop_ids_str: Vec<String> =
            sold_in_shop_ids.iter().map(|id| format!("#{id}")).collect();
        summary.push_str(&format!("；在售商店：{}", shop_ids_str.join("、")));
    }

    Ok(tool_ok(
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
    ))
}

fn handle_read_quest_db(
    cache: &DbDataCache,
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

    let db = cache.open()?;
    if let Some(id) = quest_id {
        let quest = db
            .quest(id)
            .map_err(|err| format!("查询支线任务 {id} 失败：{err}"))?
            .ok_or_else(|| format!("任务表中找不到编号为 {id} 的支线任务"))?;
        return format_quest_output_db(&db, &quest);
    }
    if let Some(query_name) = name {
        let candidates = db
            .quests_by_name_zh(query_name)
            .map_err(|err| format!("按名称查询支线任务失败：{err}"))?;
        let mut exact: Vec<&Quest> = Vec::new();
        let mut contains: Vec<(&Quest, String)> = Vec::new();
        for quest in &candidates {
            let Some(raw) = resolve_db_zh_raw(&db, &quest.title) else {
                continue;
            };
            if raw.trim() == query_name {
                exact.push(quest);
            }
            if raw.contains(query_name) {
                contains.push((quest, raw));
            }
        }
        return match exact.as_slice() {
            [] => {
                let similar: Vec<String> = contains
                    .into_iter()
                    .take(5)
                    .filter_map(|(quest, raw)| {
                        collapse_ws(&raw).map(|t| format!("{t}（#{}）", quest.id))
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
            [single] => format_quest_output_db(&db, single),
            many => Ok(format_quest_candidates(query_name, many)),
        };
    }

    let ch = chapter.expect("exactly one parameter was provided");
    let quests = db
        .quests_by_chapter(ch)
        .map_err(|err| format!("按章节查询支线任务失败：{err}"))?;
    if quests.is_empty() {
        return Ok(tool_ok(
            format!("第 {ch} 章没有推导出的支线任务。"),
            json!({ "chapter": ch, "quests": [] }),
        ));
    }

    let mut chapter_names: HashMap<i64, Option<String>> = HashMap::new();
    let mut map_names: HashMap<i64, Option<String>> = HashMap::new();
    let mut item_names: HashMap<i64, Option<String>> = HashMap::new();

    let mut lines = vec![format!("第 {ch} 章支线任务（共 {} 个）：", quests.len())];
    let mut items = Vec::with_capacity(quests.len());
    for quest in &quests {
        let title = resolve_db_zh_clean(&db, &quest.title);
        let summary = build_quest_summary_db(
            &db,
            quest,
            &mut chapter_names,
            &mut map_names,
            &mut item_names,
        )?;
        lines.push(format!("- {summary}"));
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

fn format_quest_output_db(db: &GameDb, quest: &Quest) -> Result<ToolOutput, String> {
    let mut chapter_names: HashMap<i64, Option<String>> = HashMap::new();
    let mut map_names: HashMap<i64, Option<String>> = HashMap::new();
    let mut item_names: HashMap<i64, Option<String>> = HashMap::new();

    let summary = build_quest_summary_db(
        db,
        quest,
        &mut chapter_names,
        &mut map_names,
        &mut item_names,
    )?;
    let title = resolve_db_zh_clean(db, &quest.title);
    let requester = resolve_db_zh_clean(db, &quest.requester);

    let mut objectives = Vec::with_capacity(quest.objectives.len());
    for stage in &quest.objectives {
        let goal_text = resolve_db_zh_clean(db, &stage.goal_text);
        let summary_text = resolve_db_zh_clean(db, &stage.summary_text);
        let mut destinations = Vec::with_capacity(stage.destinations.len());
        for dest in &stage.destinations {
            let map_name = cached_map_name_clean(db, dest.destination_id, &mut map_names)?;
            destinations.push(json!({
                "map_id": dest.destination_id,
                "map_name": map_name,
                "point_type": dest.point_type,
                "destination_type": dest.destination_type,
                "object_type": dest.object_type,
                "object_id": dest.object_id,
                "condition": dest.condition,
            }));
        }
        objectives.push(json!({
            "condition": stage.condition,
            "goal_text": goal_text,
            "summary_text": summary_text,
            "destinations": destinations,
            "quest_board_ids": stage.quest_board_ids,
        }));
    }

    let mut prizes = Vec::with_capacity(quest.prizes.len());
    for prize in &quest.prizes {
        let item_name = match prize.item_id {
            Some(id) => cached_item_name_clean(db, id, &mut item_names)?,
            None => None,
        };
        let text = resolve_db_zh_clean(db, &prize.text);
        prizes.push(json!({
            "item_id": prize.item_id,
            "item_name": item_name,
            "count": prize.count,
            "text": text,
        }));
    }

    Ok(tool_ok(
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
    ))
}

fn handle_read_shop_db(
    cache: &DbDataCache,
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

    let db = cache.open()?;
    if let Some(id) = shop_id {
        let resolved = db
            .shop(id)
            .map_err(|err| format!("查询商店 {id} 失败：{err}"))?
            .ok_or_else(|| format!("商店表中找不到编号为 {id} 的商店"))?;
        return format_shop_output_db(&db, &resolved);
    }

    if let Some(query_name) = name {
        let candidates = db
            .shops_by_name_zh(query_name)
            .map_err(|err| format!("按名称查询商店失败：{err}"))?;
        let mut exact: Vec<&Shop> = Vec::new();
        let mut similar: Vec<String> = Vec::new();
        for shop in &candidates {
            let Some(raw) = resolve_db_zh_raw(&db, &shop.name) else {
                continue;
            };
            if raw.trim() == query_name {
                exact.push(shop);
            }
            if similar.len() < 5
                && let Some(zh) = collapse_ws(&raw)
                && zh.contains(query_name)
            {
                similar.push(format!("{zh}（#{}）", shop.id));
            }
        }
        return match exact.as_slice() {
            [] => {
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
            [single] => {
                let resolved = db
                    .shop(single.id)
                    .map_err(|err| format!("查询商店 {} 失败：{err}", single.id))?
                    .ok_or_else(|| format!("商店表中找不到编号为 {} 的商店", single.id))?;
                format_shop_output_db(&db, &resolved)
            }
            many => Ok(format_shop_candidates(query_name, many)),
        };
    }

    let target_item_id = item_id.expect("exactly one parameter was provided");
    let target_item = db
        .item(target_item_id)
        .map_err(|err| format!("查询道具 {target_item_id} 失败：{err}"))?;
    let item_name = target_item
        .as_ref()
        .and_then(|item| resolve_db_zh_clean(&db, &item.name));
    let item_label = match (&item_name, &target_item) {
        (Some(n), _) => format!("道具 #{target_item_id}「{n}」"),
        (None, Some(_)) => format!("道具 #{target_item_id}（无中文译名）"),
        (None, None) => format!("编号 #{target_item_id}（道具表未收录）"),
    };

    let shops = db
        .shops_selling(target_item_id)
        .map_err(|err| format!("反查出售道具 {target_item_id} 的商店失败：{err}"))?;
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
        let shop_name = resolve_db_zh_clean(&db, &shop.name);
        let shop_display = shop_name
            .as_deref()
            .map_or_else(|| "无中文店名".to_owned(), |n| format!("「{n}」"));
        let offers: Vec<Value> = shop
            .items
            .iter()
            .filter(|entry| entry.item_id == target_item_id)
            .map(|entry| {
                let price = target_item
                    .as_ref()
                    .map(|it| entry.effective_price(it.buying_price));
                json!({
                    "price": price,
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
            .and_then(|entry| {
                target_item
                    .as_ref()
                    .map(|it| entry.effective_price(it.buying_price))
            })
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

fn format_shop_output_db(db: &GameDb, resolved: &ResolvedShop) -> Result<ToolOutput, String> {
    let shop = &resolved.shop;
    let name = resolve_db_zh_clean(db, &shop.name);

    let shop_title = match &name {
        Some(n) => format!("商店 #{}「{n}」", shop.id),
        None => format!("商店 #{}（无中文店名）", shop.id),
    };

    let mut goods = Vec::with_capacity(resolved.stock.len());
    let mut items = Vec::with_capacity(resolved.stock.len());
    for slot in &resolved.stock {
        let item_name = slot
            .item
            .as_ref()
            .and_then(|it| resolve_db_zh_clean(db, &it.name));
        let (display_name, price_str) = match (&slot.item, slot.price) {
            (Some(_), Some(price)) => (
                item_name.clone().unwrap_or_else(|| "无中文译名".to_owned()),
                format!("{price}G"),
            ),
            (Some(it), None) => (
                item_name.clone().unwrap_or_else(|| "无中文译名".to_owned()),
                format!("{}G", slot.entry.effective_price(it.buying_price)),
            ),
            (None, _) => (
                format!("未知道具 #{}", slot.entry.item_id),
                "价格未知".to_owned(),
            ),
        };
        let limit = match slot.entry.max_order_num {
            Some(max) => format!("，限购 {max}"),
            None => String::new(),
        };
        goods.push(format!(
            "{display_name}（#{}，{price_str}{limit}）",
            slot.entry.item_id
        ));
        items.push(json!({
            "item_id": slot.entry.item_id,
            "item_name": item_name,
            "price": slot.price,
            "max_order_num": slot.entry.max_order_num,
            "price_ratio": slot.entry.price_ratio,
            "condition": slot.entry.condition,
        }));
    }

    let summary = if goods.is_empty() {
        format!("{shop_title}：暂无商品")
    } else {
        format!(
            "{shop_title}（共 {} 件）：{}",
            goods.len(),
            goods.join("、")
        )
    };

    let mut sell_item_names: HashMap<i64, Option<String>> = HashMap::new();
    let mut sell_items = Vec::with_capacity(shop.sell_items.len());
    for entry in &shop.sell_items {
        let item_name = cached_item_name_clean(db, entry.item_id, &mut sell_item_names)?;
        sell_items.push(json!({
            "item_id": entry.item_id,
            "item_name": item_name,
            "price_ratio": entry.price_ratio,
        }));
    }

    Ok(tool_ok(
        summary,
        json!({
            "shop_id": shop.id,
            "row_key": shop.row_key,
            "name": name,
            "items": items,
            "sell_items": sell_items,
        }),
    ))
}

fn handle_search_data_db(
    cache: &DbDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("search_data", args, &["query", "kinds"])?;
    let query = required_non_empty_str("search_data", args, "query")?;
    let (kinds, _explicit_kinds) = parse_search_kinds(args)?;

    let db = cache.open()?;
    let mut hits = Vec::new();

    if kinds.contains(&SearchKind::Chapter) {
        let chapters = db
            .chapters()
            .map_err(|err| format!("检索章节失败：{err}"))?;
        let counts = db
            .main_scenario_counts_by_chapter()
            .map_err(|err| format!("统计主线条数失败：{err}"))?;
        for ch in &chapters {
            if let Some(name) = resolve_db_zh_clean(&db, &ch.name)
                && let Some(rank) = match_rank(&name, query)
            {
                let count = counts.get(&ch.chapter_no).copied().unwrap_or(0);
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

    if kinds.contains(&SearchKind::Map) {
        let maps = db
            .maps_by_name(query, MODEL_LANGUAGE)
            .map_err(|err| format!("检索地图失败：{err}"))?;
        for map in maps {
            if let Some(name) = resolve_db_zh_clean(&db, &map.name)
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
        let items = db
            .items_by_name(query, MODEL_LANGUAGE)
            .map_err(|err| format!("检索道具失败：{err}"))?;
        for item in items {
            if let Some(name) = resolve_db_zh_clean(&db, &item.name)
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
        let quests = db
            .quests_by_name_zh(query)
            .map_err(|err| format!("检索支线任务失败：{err}"))?;
        for quest in quests {
            if let Some(name) = resolve_db_zh_clean(&db, &quest.title)
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
        let shops = db
            .shops_by_name_zh(query)
            .map_err(|err| format!("检索商店失败：{err}"))?;
        for shop in shops {
            if let Some(name) = resolve_db_zh_clean(&db, &shop.name)
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

    if kinds.contains(&SearchKind::Enemy) {
        let units = db
            .units_by_name(query, MODEL_LANGUAGE)
            .map_err(|err| format!("检索敌人失败：{err}"))?;
        for unit in units {
            if !unit.category.is_enemy_category() {
                continue;
            }
            if let Some(name) = resolve_db_zh_clean(&db, &unit.name)
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
        let skills = db
            .skills_by_name(query, MODEL_LANGUAGE)
            .map_err(|err| format!("检索技能失败：{err}"))?;
        for skill in skills {
            if let Some(name) = resolve_db_zh_clean(&db, &skill.name)
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

    let kind_names: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
    let payload = json!({
        "query": query,
        "kinds": kind_names,
        "total_matches": total_matches,
        "truncated": truncated,
        "results": results_json,
    });

    Ok(tool_ok(lines.join("\n"), payload))
}

// ---- DB-backed summary formatters (matching `ffr-data` verbatim) ----

fn format_scenario_destinations_db(
    db: &GameDb,
    entry: &MainScenarioEntry,
    map_names: &mut HashMap<i64, Option<String>>,
) -> Result<(Vec<String>, Vec<Value>), String> {
    let mut labels = Vec::with_capacity(entry.destinations.len());
    let mut json_list = Vec::with_capacity(entry.destinations.len());
    for dest in &entry.destinations {
        let map_name = cached_map_name_clean(db, dest.destination_id, map_names)?;
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
    Ok((labels, json_list))
}

fn describe_treasure_box_db(
    db: &GameDb,
    chest: &TreasureBox,
    item_names: &mut HashMap<i64, Option<String>>,
) -> Result<String, String> {
    let mut parts = Vec::new();
    if let Some(item_id) = chest.item_id {
        let name = cached_item_name_clean(db, item_id, item_names)?
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
        Ok("空宝箱".to_owned())
    } else {
        Ok(parts.join("，"))
    }
}

fn build_map_summary_db(
    map: &MapInfo,
    name: Option<&str>,
    island_name: Option<&str>,
    contents: &MapContents,
) -> String {
    let display_name = name.unwrap_or("未命名地图");
    let map_type = map_type_label(&map.map_type);
    let mut parts = vec![format!(
        "地图 #{id}「{display_name}」（关卡 {level}，{map_type}）",
        id = map.id,
        level = map.level_name
    )];

    if let Some(dungeon_sub) = dungeon_type_label(&map.dungeon_type) {
        parts.push(format!("迷宫类型：{dungeon_sub}"));
    }

    if !map.island.is_empty() && map.island != "eMapIslandType::None" {
        let island_label = island_name
            .map(str::to_owned)
            .unwrap_or_else(|| strip_enum_prefix(&map.island).to_owned());
        parts.push(format!("区域：{island_label}"));
    }

    if map.danger_level > 0 {
        parts.push(format!("危险度：{}", map.danger_level));
    }

    if map.do_movement_encount {
        if let Some(group_id) = map.encount_group_id {
            parts.push(format!("移动遇敌：开启（遇敌组 #{group_id}）"));
        } else {
            parts.push("移动遇敌：开启".to_owned());
        }
    }

    if !map.facilities.is_empty() {
        let mut names = Vec::new();
        for fac in &map.facilities {
            let label = facility_zh(&fac.interact_type);
            if !names.contains(&label) {
                names.push(label);
            }
        }
        parts.push(format!("设施：{}", names.join("、")));
    }

    if !contents.teleport_points.is_empty() {
        parts.push("世界地图可传送".to_owned());
    }

    if let Some(sp) = &contents.save_point {
        let mut caps = Vec::new();
        if sp.can_save {
            caps.push("存档");
        }
        if sp.can_use_tent {
            caps.push("帐篷");
        }
        if sp.can_teleport {
            caps.push("迷宫内传送");
        }
        if !caps.is_empty() {
            parts.push(format!("存档点支持：{}", caps.join("/")));
        }
    }

    let box_count = contents.treasure_boxes.len();
    if box_count > 0 {
        parts.push(format!("宝箱：{box_count} 个"));
    }

    parts.join("；")
}

fn build_item_summary_db(item: &Item, name: Option<&str>, description: Option<&str>) -> String {
    let display_name = name.unwrap_or("无中文译名");
    let category = item_category_summary_label(item);
    let price = if item.is_available_sale {
        format!("买价 {} / 卖价 {}", item.buying_price, item.selling_price)
    } else {
        format!("买价 {} / 不可出售", item.buying_price)
    };
    let effect = build_item_effect_overview(item, description);
    format!(
        "道具 #{id}「{display_name}」（{category}）；{price}；效果：{effect}",
        id = item.id
    )
}

fn build_item_effect_overview(item: &Item, description: Option<&str>) -> String {
    let mut facts = Vec::new();
    match &item.detail {
        ItemDetail::Consumable {
            usable_scene,
            is_consumed_after_use,
            skill_id,
            is_throwable,
            throw_skill_id,
        } => {
            facts.push(format!("使用场合：{}", usable_scene_label(usable_scene)));
            if !*is_consumed_after_use {
                facts.push("使用后不消耗".to_owned());
            }
            if let Some(sid) = skill_id {
                facts.push(format!("技能 #{sid}"));
            }
            if *is_throwable {
                if let Some(tsid) = throw_skill_id {
                    facts.push(format!("可投掷（技能 #{tsid}）"));
                } else {
                    facts.push("可投掷".to_owned());
                }
            }
        }
        ItemDetail::Seed { harvest_recipes } => {
            facts.push(format!("培育配方 {} 种", harvest_recipes.len()));
        }
        ItemDetail::Equipment {
            params,
            passive_skill_ids,
            ..
        } => {
            if !params.is_empty() {
                let stats: Vec<String> = params
                    .iter()
                    .map(|p| {
                        let sign = if p.value >= 0 { "+" } else { "" };
                        format!("{}{sign}{}", param_type_label(&p.param_type), p.value)
                    })
                    .collect();
                facts.push(format!("属性：{}", stats.join("、")));
            }
            if !passive_skill_ids.is_empty() {
                let ids: Vec<String> = passive_skill_ids
                    .iter()
                    .map(|id| format!("#{id}"))
                    .collect();
                facts.push(format!("被动技能：{}", ids.join("、")));
            }
        }
        ItemDetail::Vision {
            finish_blow_skill_id,
            attack_type,
            roles,
            ..
        } => {
            facts.push(format!("攻击类型：{}", attack_type_label(attack_type)));
            if !roles.is_empty() {
                let role_labels: Vec<String> =
                    roles.iter().map(|r| role_label(r).to_owned()).collect();
                facts.push(format!("职责：{}", role_labels.join("/")));
            }
            if let Some(sid) = finish_blow_skill_id {
                facts.push(format!("终结技 #{sid}"));
            }
        }
        ItemDetail::Valuable => {}
        ItemDetail::Artifact {
            item_tag,
            effect_values,
            is_initial_active,
        } => {
            if !item_tag.is_empty() && item_tag != "None" {
                facts.push(format!("标签：{item_tag}"));
            }
            if !effect_values.is_empty() {
                let vals: Vec<String> = effect_values.iter().map(|v| format_number(*v)).collect();
                facts.push(format!("效果值：{}", vals.join("、")));
            }
            if *is_initial_active {
                facts.push("初始生效".to_owned());
            }
        }
    }

    if let Some(desc) = description {
        facts.push(desc.to_owned());
    }

    if facts.is_empty() {
        "无效果说明".to_owned()
    } else {
        facts.join("；")
    }
}

fn build_quest_summary_db(
    db: &GameDb,
    quest: &Quest,
    chapter_names: &mut HashMap<i64, Option<String>>,
    map_names: &mut HashMap<i64, Option<String>>,
    item_names: &mut HashMap<i64, Option<String>>,
) -> Result<String, String> {
    let title = resolve_db_zh_clean(db, &quest.title).unwrap_or_else(|| "无中文标题".to_owned());
    let chapter = match quest.chapter_no {
        Some(ch) => {
            let chapter_title = cached_chapter_name_clean(db, ch, chapter_names)?;
            match chapter_title {
                Some(name) => format!("第 {ch} 章「{name}」"),
                None => format!("第 {ch} 章"),
            }
        }
        None => "所属章节未定（前置支线条件）".to_owned(),
    };

    let mut parts = vec![format!(
        "任务 #{id}「{title}」（{chapter}，地区：{region}）",
        id = quest.id,
        region = strip_enum_prefix(&quest.category)
    )];

    if let Some(requester) = resolve_db_zh_clean(db, &quest.requester) {
        if let Some(map_id) = quest.requester_map_id {
            let map_name = cached_map_name_clean(db, map_id, map_names)?;
            match map_name {
                Some(name) => {
                    parts.push(format!("委托人：{requester}（地图 #{map_id}「{name}」）"))
                }
                None => parts.push(format!("委托人：{requester}（地图 #{map_id}）")),
            }
        } else {
            parts.push(format!("委托人：{requester}"));
        }
    }

    if !quest.accept_condition.is_empty() {
        parts.push(format!("接取条件：{}", quest.accept_condition));
    }

    let goals: Vec<String> = quest
        .objectives
        .iter()
        .enumerate()
        .filter_map(|(idx, stage)| {
            resolve_db_zh_clean(db, &stage.goal_text).map(|text| format!("{}. {text}", idx + 1))
        })
        .collect();
    if !goals.is_empty() {
        parts.push(format!("目标：{}", goals.join(" -> ")));
    }

    let mut prizes = Vec::new();
    for prize in &quest.prizes {
        if let Some(item_id) = prize.item_id {
            let item_name = cached_item_name_clean(db, item_id, item_names)?
                .unwrap_or_else(|| format!("道具 #{item_id}"));
            prizes.push(format!("{item_name}×{}", prize.count));
        } else if let Some(text) = resolve_db_zh_clean(db, &prize.text) {
            prizes.push(text);
        }
    }
    if !prizes.is_empty() {
        parts.push(format!("奖励：{}", prizes.join("、")));
    }

    Ok(parts.join("；"))
}

fn format_full_enemy_summary(
    profile: &EnemyProfile,
    display_name: &str,
    weak_break_mult: f64,
    break_damage_magnification: f64,
    high_risk_resolved: &[(i64, bool, Option<String>)],
) -> String {
    let boss_label = enemy_boss_label(profile);
    let weak_text = join_elements_zh(&profile.weaknesses);
    let resist_text = format_resistances_zh(profile);

    let mut summary = format!(
        "{display_name}（编号 {}，Lv.{}，{boss_label}）：HP {}，破防槽（盾值）{}（弱点破防倍率 {:.1}x，破防增伤 {:.0}%）；弱点：{}；抗性：{}。",
        profile.unit_id,
        profile.level,
        profile.max_hp,
        profile.max_break_point,
        weak_break_mult,
        break_damage_magnification,
        weak_text,
        resist_text,
    );

    if !high_risk_resolved.is_empty() {
        let mut skill_labels = Vec::with_capacity(high_risk_resolved.len());
        for (skill_id, exists, raw_name) in high_risk_resolved {
            if *exists {
                if let Some(text) = raw_name {
                    skill_labels.push(format!("{text}（编号 {skill_id}）"));
                } else {
                    skill_labels.push(format!("编号 {skill_id}（缺少 {MODEL_LANGUAGE} 译名）"));
                }
            } else {
                skill_labels.push(format!("编号 {skill_id}（技能表未收录）"));
            }
        }
        summary.push_str(&format!(
            "高危/斗气技能：{}（可用破防打断）。",
            skill_labels.join("、")
        ));
    }

    summary
}

fn format_fallback_enemy_summary_db(
    break_calc: &BreakCalcParams,
    profile: &EnemyProfile,
    name: Option<&str>,
) -> String {
    let name_display = name.unwrap_or("无中文译名");
    let boss_label = enemy_boss_label(profile);
    let weak_text = join_elements_zh(&profile.weaknesses);
    let resist_text = format_resistances_zh(profile);
    let mult = break_calc
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
        break_calc.break_damage_magnification,
        weak_text,
        resist_text,
    )
}

fn enemy_boss_label(profile: &EnemyProfile) -> &'static str {
    if profile.is_boss {
        "Boss"
    } else {
        match profile.category {
            UnitCategory::Boss => "非Boss（大型/稀有分类）",
            UnitCategory::Enemy => "普通敌人",
            UnitCategory::Playable => "可操控角色",
            UnitCategory::Beast => "召唤兽",
            UnitCategory::Vision => "幻兵",
        }
    }
}

fn format_resistances_zh(profile: &EnemyProfile) -> String {
    let mut parts = Vec::new();
    if !profile.halved.is_empty() {
        parts.push(format!("减半({})", join_elements_zh(&profile.halved)));
    }
    if !profile.invalid.is_empty() {
        parts.push(format!("无效({})", join_elements_zh(&profile.invalid)));
    }
    if !profile.absorb.is_empty() {
        parts.push(format!("吸收({})", join_elements_zh(&profile.absorb)));
    }
    if parts.is_empty() {
        "无".to_owned()
    } else {
        parts.join("；")
    }
}

// ---- Per-request small lookup caches ----

pub(crate) fn cached_map_name_clean(
    db: &GameDb,
    map_id: i64,
    cache: &mut HashMap<i64, Option<String>>,
) -> Result<Option<String>, String> {
    if let Some(hit) = cache.get(&map_id) {
        return Ok(hit.clone());
    }
    let name = db
        .map(map_id)
        .map_err(|err| format!("查询地图 {map_id} 失败：{err}"))?
        .and_then(|m| resolve_db_zh_clean(db, &m.name));
    cache.insert(map_id, name.clone());
    Ok(name)
}

pub(crate) fn cached_item_name_clean(
    db: &GameDb,
    item_id: i64,
    cache: &mut HashMap<i64, Option<String>>,
) -> Result<Option<String>, String> {
    if let Some(hit) = cache.get(&item_id) {
        return Ok(hit.clone());
    }
    let name = db
        .item(item_id)
        .map_err(|err| format!("查询道具 {item_id} 失败：{err}"))?
        .and_then(|it| resolve_db_zh_clean(db, &it.name));
    cache.insert(item_id, name.clone());
    Ok(name)
}

fn cached_chapter_name_clean(
    db: &GameDb,
    chapter_no: i64,
    cache: &mut HashMap<i64, Option<String>>,
) -> Result<Option<String>, String> {
    if let Some(hit) = cache.get(&chapter_no) {
        return Ok(hit.clone());
    }
    let name = db
        .chapter(chapter_no)
        .map_err(|err| format!("查询第 {chapter_no} 章失败：{err}"))?
        .and_then(|c| resolve_db_zh_clean(db, &c.name));
    cache.insert(chapter_no, name.clone());
    Ok(name)
}

// ---- Enum label helpers matching `ffr_data::world::summary` verbatim ----

fn item_category_summary_label(item: &Item) -> String {
    match (&item.category, &item.detail) {
        (ItemCategory::Consumable, _) => "消耗品".to_owned(),
        (ItemCategory::Seed, _) => "种子".to_owned(),
        (
            ItemCategory::Equipment,
            ItemDetail::Equipment {
                equipment_type,
                detail_type,
                ..
            },
        ) => {
            let main = equipment_type_label(equipment_type);
            match equipment_detail_label(detail_type) {
                Some(sub) => format!("{main}/{sub}"),
                None => main.to_owned(),
            }
        }
        (ItemCategory::Equipment, _) => "装备".to_owned(),
        (ItemCategory::Vision, _) => "幻兵".to_owned(),
        (ItemCategory::Valuable, _) => "贵重品".to_owned(),
        (ItemCategory::Artifact, _) => "圣遗物".to_owned(),
    }
}

pub(crate) fn map_type_label(raw: &str) -> &str {
    match raw {
        "eMapType::Field" => "世界地图",
        "eMapType::Town" => "城镇",
        "eMapType::Dungeon" => "迷宫",
        "eMapType::VisionSmallShrine" => "幻兵小祠",
        other => strip_enum_prefix(other),
    }
}

fn dungeon_type_label(raw: &str) -> Option<&'static str> {
    match raw {
        "eMapDungeonType::Castle" => Some("城堡"),
        "eMapDungeonType::Cave" => Some("洞窟"),
        _ => None,
    }
}

fn usable_scene_label(raw: &str) -> &str {
    match raw {
        "eItemUsableScene::Any" => "战斗/地图通用",
        "eItemUsableScene::BattleOnly" => "仅战斗",
        "eItemUsableScene::MapOnly" => "仅地图",
        other => strip_enum_prefix(other),
    }
}

fn equipment_type_label(raw: &str) -> &str {
    match raw {
        "eEquipmentType::Weapon" => "武器",
        "eEquipmentType::Armor" => "防具",
        "eEquipmentType::Accessory" => "饰品",
        other => strip_enum_prefix(other),
    }
}

fn equipment_detail_label(raw: &str) -> Option<&'static str> {
    match raw {
        "eEquipmentDetailType::GreatSowrd" | "eEquipmentDetailType::GreatSword" => Some("大剑"),
        "eEquipmentDetailType::Katana" => Some("太刀"),
        "eEquipmentDetailType::Bow" => Some("弓"),
        "eEquipmentDetailType::Hammer" => Some("锤"),
        "eEquipmentDetailType::Grimoire" => Some("魔导书"),
        "eEquipmentDetailType::Whip" => Some("鞭"),
        "eEquipmentDetailType::Gun" => Some("枪"),
        "eEquipmentDetailType::Rod" => Some("法杖"),
        "eEquipmentDetailType::LightArmor" => Some("轻甲"),
        "eEquipmentDetailType::HeavyArmor" => Some("重甲"),
        "eEquipmentDetailType::Robe" => Some("长袍"),
        _ => None,
    }
}

fn param_type_label(raw: &str) -> &str {
    match raw {
        "eUnitParameterType::PhysicalAttack" => "物攻",
        "eUnitParameterType::MagicalAttack" => "魔攻",
        "eUnitParameterType::PhysicalDefense" => "物防",
        "eUnitParameterType::MagicalDefense" => "魔防",
        "eUnitParameterType::Defence" => "防御",
        "eUnitParameterType::Divine" => "精神",
        "eUnitParameterType::Agility" => "敏捷",
        "eUnitParameterType::HitPoint" => "HP",
        "eUnitParameterType::MagicPoint" => "MP",
        other => strip_enum_prefix(other),
    }
}

fn attack_type_label(raw: &str) -> &str {
    match raw {
        "eSkillDamageType::Physic" => "物理",
        "eSkillDamageType::Magic" => "魔法",
        "eSkillDamageType::None" => "无",
        other => strip_enum_prefix(other),
    }
}

fn role_label(raw: &str) -> &str {
    match raw {
        "eUnitRole::Attacker" => "攻击手",
        "eUnitRole::Breaker" => "破防手",
        "eUnitRole::Defender" | "eUnitRole::Tank" => "防御手",
        "eUnitRole::Enhancer" => "强化手",
        "eUnitRole::Healer" => "治疗手",
        "eUnitRole::Supporter" | "eUnitRole::Buffer" => "支援手",
        "eUnitRole::Jammer" | "eUnitRole::Debuffer" => "妨碍手",
        other => strip_enum_prefix(other),
    }
}

fn strip_enum_prefix(raw: &str) -> &str {
    raw.rsplit_once("::").map_or(raw, |(_, suffix)| suffix)
}

fn format_number(value: f64) -> String {
    if (value - value.round()).abs() <= f64::EPSILON {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.2}")
    }
}
