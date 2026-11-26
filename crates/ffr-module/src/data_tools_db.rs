//! `ffr-db` SQLite backend implementations for all 9 `ToolCategory::Data` tools.
//!
//! Produces output (both Chinese summary text and structured JSON) strictly identical to the
//! legacy `GameDataCache` JSON backend in [`crate::data_tools`], while querying only indexed
//! records via [`GameDb`] without loading whole tables into memory.

use std::collections::{BTreeSet, HashMap, HashSet};

use agp_game::ToolOutput;
use agp_platform::FileChannel;
use ffr_data::assets::level::{LevelObjectData, LevelObjectKind, MapEdgeKind};
use ffr_data::battle::{BATTLE_GROUP_TABLE, EnemyProfile, Skill, Unit, UnitAiProfile};
use ffr_data::world::{Item, Quest, Shop};
use ffr_data::{
    DataError, FTextValue, MODEL_LANGUAGE, MainScenarioEntry, QUEST_MENU_STRING_TABLE,
    ResolveOptions, main_quest_id_for_chapter,
};
use ffr_db::GameDb;
use ffr_db::query::world::{MapContents, ResolvedShop};
use serde_json::{Map, Value, json};

use crate::data_cache::DbDataCache;
use crate::data_tools::{
    SearchHit, SearchKind, build_enemy_drops_and_steals, build_map_summary_shared,
    build_quest_summary_shared, collapse_ws, describe_treasure_box_parts, element_zh,
    elements_code_vec, elements_zh_vec, facility_zh, format_enemy_battle_groups_slice,
    format_enemy_candidates, format_enemy_summary_shared, format_item_candidates,
    format_item_output_parts, format_quest_candidates, format_search_output,
    format_shop_candidates, format_skill_candidates, format_skill_output_parts, item_category_zh,
    join_elements_zh, match_rank, match_rank_zh_text, optional_i64, optional_non_empty_str,
    parse_search_kinds, reject_unknown_keys, required_i64, required_non_empty_str, tool_ok,
    unit_category_code,
};
use crate::data_tools_ffr3::{
    collect_map_npcs_for_read_map, format_vec3_zh, handle_find_route_db, handle_query_game_db_db,
    handle_read_npc_db, handle_read_talk_db, handle_whats_near_db, handle_where_to_get_db,
    map_direction_enum_zh, map_edge_kind_zh, vec3_json,
};
use crate::text_format::{
    ZhText, format_map_ref_zh, resolve_db_zh_text_clean, resolve_db_zh_text_raw, with_zh_fields,
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
            let name = resolve_db_zh_text_raw(&db, &ch.name);
            let entry_count = counts.get(&ch.chapter_no).copied().unwrap_or(0);
            lines.push(format!(
                "- 第 {} 章{}（主线 {} 条）",
                ch.chapter_no,
                name.bracketed_or("无中文译名"),
                entry_count
            ));
            let main_quest_id = main_quest_id_for_chapter(ch.chapter_no);
            let main_quest_title = main_quest_id.and_then(|_| {
                name.zh()
                    .map(|n| format_db_main_quest_title(&db, ch.chapter_no, n))
            });
            items.push(with_zh_fields(
                json!({
                    "chapter_no": ch.chapter_no,
                    "main_quest_id": main_quest_id,
                    "main_quest_title": main_quest_title,
                    "row_key": ch.row_key,
                    "main_scenario_count": entry_count,
                }),
                &[("name", &name)],
            ));
        }
        return Ok(tool_ok(lines.join("\n"), json!({ "chapters": items })));
    };

    let Some(ch) = db
        .chapter(chapter_no)
        .map_err(|err| format!("查询第 {chapter_no} 章失败：{err}"))?
    else {
        return Err(format!("章节表中找不到第 {chapter_no} 章"));
    };
    let name = resolve_db_zh_text_raw(&db, &ch.name);
    let entries = db
        .main_scenario_by_chapter(chapter_no)
        .map_err(|err| format!("查询第 {chapter_no} 章主线剧情失败：{err}"))?;

    let mut lines = vec![format!(
        "第 {} 章{}（共 {} 条主线剧情）：",
        chapter_no,
        name.bracketed_or("无中文译名"),
        entries.len()
    )];
    let mut json_entries = Vec::with_capacity(entries.len());
    let mut map_names: HashMap<i64, ZhText> = HashMap::new();

    for entry in &entries {
        let goal_text = resolve_db_zh_text_clean(&db, &entry.text);
        let summary_text = resolve_db_zh_text_clean(&db, &entry.summary_text);
        let (dest_labels, dest_json) = format_scenario_destinations_db(&db, entry, &mut map_names)?;

        let progress_str = entry
            .min_progress
            .map_or_else(|| "无进度阈值".to_owned(), |p| format!("进度>={p}"));
        let goal_display = goal_text.inline_or("无中文目标");
        let mut line = format!("- [{progress_str}] {goal_display}");
        if !dest_labels.is_empty() {
            line.push_str(&format!("（目的地：{}）", dest_labels.join("、")));
        }
        if let Some(summary) = summary_text.inline_opt() {
            line.push_str(&format!(" — {summary}"));
        }
        lines.push(line);

        json_entries.push(with_zh_fields(
            json!({
                "row_key": entry.row_key,
                "condition": entry.condition,
                "min_progress": entry.min_progress,
                "destinations": dest_json,
            }),
            &[("text", &goal_text), ("summary", &summary_text)],
        ));
    }

    let main_quest_id = main_quest_id_for_chapter(ch.chapter_no);
    let main_quest_title = main_quest_id.and_then(|_| {
        name.zh()
            .map(|n| format_db_main_quest_title(&db, ch.chapter_no, n))
    });
    Ok(tool_ok(
        lines.join("\n"),
        with_zh_fields(
            json!({
                "chapter_no": ch.chapter_no,
                "main_quest_id": main_quest_id,
                "main_quest_title": main_quest_title,
                "row_key": ch.row_key,
                "main_scenario": json_entries,
            }),
            &[("name", &name)],
        ),
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
    let name = resolve_db_zh_text_clean(db, &map.name);
    let island_name = contents
        .island
        .as_ref()
        .map(|island| resolve_db_zh_text_clean(db, &island.name))
        .unwrap_or_default();

    let save_point = contents.save_point.as_ref();
    let teleport_points = &contents.teleport_points;
    let treasure_boxes = &contents.treasure_boxes;
    let base_summary = build_map_summary_shared(
        map,
        &name,
        &island_name,
        !teleport_points.is_empty(),
        save_point,
        treasure_boxes.len(),
    );

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

    let mut map_names: HashMap<i64, ZhText> = HashMap::new();
    map_names.insert(map.id, name.clone());

    let mut exits_json = Vec::with_capacity(exits_raw.len());
    let mut exit_labels = Vec::new();
    for edge in &exits_raw {
        let target_name = match edge.target_map_id {
            Some(tid) => cached_map_name_zh(db, tid, &mut map_names)?,
            None => ZhText::Missing,
        };
        let label = match (edge.kind, edge.target_map_id) {
            (MapEdgeKind::SavePointTeleport, Some(tid)) => {
                format!("{}（存档点传送）", format_map_ref_zh(tid, &target_name))
            }
            (MapEdgeKind::SameMap, _) => "同图传送".to_owned(),
            (_, Some(tid)) => format_map_ref_zh(tid, &target_name),
            (_, None) => "未知目标".to_owned(),
        };
        if !exit_labels.contains(&label) {
            exit_labels.push(label);
        }
        exits_json.push(with_zh_fields(
            json!({
                "kind": edge.kind.as_str(),
                "kind_zh": map_edge_kind_zh(edge.kind),
                "target_map_id": edge.target_map_id,
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
            }),
            &[("target_map_name", &target_name)],
        ));
    }

    let mut shops_json = Vec::with_capacity(shops_raw.len());
    let mut shop_labels = Vec::with_capacity(shops_raw.len());
    for entry in &shops_raw {
        let shop_name = db
            .shop(entry.shop_id)
            .map_err(|err| format!("查询商店 {} 名称失败：{err}", entry.shop_id))?
            .map(|s| resolve_db_zh_text_clean(db, &s.shop.name))
            .unwrap_or_default();
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
        let shop_title = format!("#{}{}", entry.shop_id, shop_name.bracketed_or_empty());
        let detail_part = if details.is_empty() {
            String::new()
        } else {
            format!("（{}）", details.join("，"))
        };
        shop_labels.push(format!("{shop_title}{detail_part}"));
        shops_json.push(with_zh_fields(
            json!({
                "shop_id": entry.shop_id,
                "various_shop_id": entry.various_shop_id,
                "unique_id": entry.unique_id,
                "map_unit_id": entry.map_unit_id,
                "interact_type": entry.interact_type,
                "interact_type_zh": type_zh,
                "visible_condition": entry.visible_condition,
                "location": entry.location.map(vec3_json),
            }),
            &[("shop_name", &shop_name)],
        ));
    }

    let mut save_points_json = Vec::with_capacity(save_point_objs.len());
    let mut save_point_labels = Vec::with_capacity(save_point_objs.len());
    for obj in &save_point_objs {
        let LevelObjectData::SavePoint(sp) = &obj.data else {
            continue;
        };
        let target_map_id = sp.target_map_id.or(sp.transition_map_id);
        let target_map_name = match target_map_id {
            Some(tid) => cached_map_name_zh(db, tid, &mut map_names)?,
            None => ZhText::Missing,
        };
        let pos_str = obj
            .location
            .map_or_else(|| "坐标未知".to_owned(), format_vec3_zh);
        let tp_str = match target_map_id {
            Some(tid) => format!(" -> 传送至 {}", format_map_ref_zh(tid, &target_map_name)),
            None => String::new(),
        };
        save_point_labels.push(format!("{pos_str}{tp_str}"));
        save_points_json.push(with_zh_fields(
            json!({
                "unique_id": obj.unique_id,
                "save_point_type": sp.save_point_type,
                "is_teleport": sp.is_teleport,
                "target_map_id": target_map_id,
                "target_point_id": sp.target_point_id.or(sp.transition_point_id),
                "visible_condition": obj.visible_condition,
                "location": obj.location.map(vec3_json),
            }),
            &[("target_map_name", &target_map_name)],
        ));
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

    let mut item_names: HashMap<i64, ZhText> = HashMap::new();
    let mut treasures_json = Vec::with_capacity(treasure_boxes.len());
    if !treasure_boxes.is_empty() {
        let mut chest_parts = Vec::with_capacity(treasure_boxes.len());
        for chest in treasure_boxes {
            let item_name = match chest.item_id {
                Some(id) => cached_item_name_zh(db, id, &mut item_names)?,
                None => ZhText::Missing,
            };
            let content = describe_treasure_box_parts(chest, &item_name);
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

            treasures_json.push(with_zh_fields(
                json!({
                    "box_id": chest.id,
                    "unique_id": placement.and_then(|p| p.unique_id),
                    "item_id": chest.item_id,
                    "item_num": chest.item_num,
                    "money": chest.money,
                    "is_trap": chest.is_trap,
                    "battle_group_id": chest.battle_group_id,
                    "battle_finish_event_id": chest.battle_finish_event_id,
                    "location": placement.and_then(|p| p.location).map(vec3_json),
                    "visible_condition": placement.and_then(|p| p.visible_condition.clone()),
                }),
                &[("item_name", &item_name)],
            ));
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
        with_zh_fields(
            json!({
                "map_id": map.id,
                "row_key": map.row_key,
                "level_name": map.level_name,
                "level_asset_path": map.level_asset_path,
                "map_type": map.map_type,
                "dungeon_type": map.dungeon_type,
                "island": map.island,
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
            &[("name", &name), ("island_name", &island_name)],
        ),
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

    let resolved_name = resolve_db_zh_text_raw(db, &unit.name);

    let mut high_risk_resolved: Vec<(i64, bool, ZhText)> =
        Vec::with_capacity(profile.high_risk_skill_ids.len());
    for &skill_id in &profile.high_risk_skill_ids {
        match db
            .skill(skill_id)
            .map_err(|err| format!("查询技能 {skill_id} 失败：{err}"))?
        {
            Some(skill) => {
                let raw = resolve_db_zh_text_raw(db, &skill.name);
                high_risk_resolved.push((skill_id, true, raw));
            }
            None => high_risk_resolved.push((skill_id, false, ZhText::Missing)),
        }
    }

    let mut summary =
        format_enemy_summary_shared(&break_calc, &profile, &resolved_name, &high_risk_resolved);

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

    let mut item_name_cache: HashMap<i64, ZhText> = HashMap::new();
    let (drops_json, steals_json, drop_line, steal_line) =
        build_enemy_drops_and_steals(unit, |item_id| {
            cached_item_name_zh(db, item_id, &mut item_name_cache).unwrap_or_default()
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
            let skill_name = raw.cleaned();
            with_zh_fields(json!({ "skill_id": skill_id }), &[("name", &skill_name)])
        })
        .collect();

    let payload = with_zh_fields(
        json!({
            "unit_id": profile.unit_id,
            "row_key": profile.row_key,
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
                    .get(&ffr_data::battle::Resistance::Weakness)
                    .copied(),
                "break_damage_magnification": break_calc.break_damage_magnification,
            },
            "high_risk_skills": high_risk_skills,
            "drops": drops_json,
            "steals": steals_json,
            "chapters": chapters,
            "battle_groups": battle_groups_json,
        }),
        &[("name", &resolved_name)],
    );

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

    let mut members: Vec<(EnemyProfile, ZhText)> = Vec::with_capacity(detail.members.len());
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
        let name = resolve_db_zh_text_clean(&db, &unit.name);
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
        let role = if member.is_boss { "Boss" } else { "普通" };
        let weak = join_elements_zh(&member.weaknesses);
        lines.push(format!(
            "- unit_id={}{}（Lv.{}，{}，HP {}，破防槽 {}，弱点：{}）",
            member.unit_id,
            name.bracketed_or("无中文译名"),
            member.level,
            role,
            member.max_hp,
            member.max_break_point,
            weak
        ));
        members_json.push(with_zh_fields(
            json!({
                "unit_id": member.unit_id,
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
            }),
            &[("name", name)],
        ));
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
    let name = resolve_db_zh_text_clean(db, &skill.name);
    let description = resolve_db_zh_text_clean(db, &skill.description);
    format_skill_output_parts(skill, &name, &description)
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
    let name = resolve_db_zh_text_clean(db, &item.name);
    let description = resolve_db_zh_text_clean(db, &item.description);
    let shops = db
        .shops_selling(item.id)
        .map_err(|err| format!("查询出售道具 {} 的商店失败：{err}", item.id))?;
    let sold_in_shop_ids: Vec<i64> = shops.iter().map(|shop| shop.id).collect();
    Ok(format_item_output_parts(
        item,
        &name,
        &description,
        sold_in_shop_ids,
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

    let mut chapter_names: HashMap<i64, ZhText> = HashMap::new();
    let mut map_names: HashMap<i64, ZhText> = HashMap::new();
    let mut item_names: HashMap<i64, ZhText> = HashMap::new();

    let mut lines = vec![format!("第 {ch} 章支线任务（共 {} 个）：", quests.len())];
    let mut items = Vec::with_capacity(quests.len());
    for quest in &quests {
        let title = resolve_db_zh_text_clean(&db, &quest.title);
        let summary = build_quest_summary_db(
            &db,
            quest,
            &mut chapter_names,
            &mut map_names,
            &mut item_names,
        )?;
        lines.push(format!("- {summary}"));
        items.push(with_zh_fields(
            json!({
                "quest_id": quest.id,
                "row_key": quest.row_key,
                "chapter_no": quest.chapter_no,
                "requester_map_id": quest.requester_map_id,
                "report_map_id": quest.report_map_id,
                "accept_condition": quest.accept_condition,
            }),
            &[("title", &title)],
        ));
    }
    Ok(tool_ok(
        lines.join("\n"),
        json!({ "chapter": ch, "quests": items }),
    ))
}

fn format_quest_output_db(db: &GameDb, quest: &Quest) -> Result<ToolOutput, String> {
    let mut chapter_names: HashMap<i64, ZhText> = HashMap::new();
    let mut map_names: HashMap<i64, ZhText> = HashMap::new();
    let mut item_names: HashMap<i64, ZhText> = HashMap::new();

    let summary = build_quest_summary_db(
        db,
        quest,
        &mut chapter_names,
        &mut map_names,
        &mut item_names,
    )?;
    let title = resolve_db_zh_text_clean(db, &quest.title);
    let requester = resolve_db_zh_text_clean(db, &quest.requester);

    let mut objectives = Vec::with_capacity(quest.objectives.len());
    for stage in &quest.objectives {
        let goal_text = resolve_db_zh_text_clean(db, &stage.goal_text);
        let summary_text = resolve_db_zh_text_clean(db, &stage.summary_text);
        let mut destinations = Vec::with_capacity(stage.destinations.len());
        for dest in &stage.destinations {
            let map_name = cached_map_name_zh(db, dest.destination_id, &mut map_names)?;
            destinations.push(with_zh_fields(
                json!({
                    "map_id": dest.destination_id,
                    "point_type": dest.point_type,
                    "destination_type": dest.destination_type,
                    "object_type": dest.object_type,
                    "object_id": dest.object_id,
                    "condition": dest.condition,
                }),
                &[("map_name", &map_name)],
            ));
        }
        objectives.push(with_zh_fields(
            json!({
                "condition": stage.condition,
                "destinations": destinations,
                "quest_board_ids": stage.quest_board_ids,
            }),
            &[("goal_text", &goal_text), ("summary_text", &summary_text)],
        ));
    }

    let mut prizes = Vec::with_capacity(quest.prizes.len());
    for prize in &quest.prizes {
        let item_name = match prize.item_id {
            Some(id) => cached_item_name_zh(db, id, &mut item_names)?,
            None => ZhText::Missing,
        };
        let text = resolve_db_zh_text_clean(db, &prize.text);
        prizes.push(with_zh_fields(
            json!({
                "item_id": prize.item_id,
                "count": prize.count,
            }),
            &[("item_name", &item_name), ("text", &text)],
        ));
    }

    Ok(tool_ok(
        summary,
        with_zh_fields(
            json!({
                "quest_id": quest.id,
                "row_key": quest.row_key,
                "chapter_no": quest.chapter_no,
                "min_main_progress": quest.min_main_progress,
                "category": quest.category,
                "requester_map_id": quest.requester_map_id,
                "report_map_id": quest.report_map_id,
                "accept_condition": quest.accept_condition,
                "occurrence_condition": quest.occurrence_condition,
                "achievement_condition": quest.achievement_condition,
                "completion_condition": quest.completion_condition,
                "objectives": objectives,
                "prizes": prizes,
            }),
            &[("title", &title), ("requester", &requester)],
        ),
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
        .map(|item| resolve_db_zh_text_clean(&db, &item.name))
        .unwrap_or_default();
    let item_label = match (&item_name, target_item.is_some()) {
        (ZhText::Translated(n), _) => format!("道具 #{target_item_id}「{n}」"),
        (ZhText::Untranslated { ja }, _) => {
            format!("道具 #{target_item_id}{}", ZhText::format_untranslated(ja))
        }
        (ZhText::Missing, true) => format!("道具 #{target_item_id}（无中文译名）"),
        (ZhText::Missing, false) => format!("编号 #{target_item_id}（道具表未收录）"),
    };

    let shops = db
        .shops_selling(target_item_id)
        .map_err(|err| format!("反查出售道具 {target_item_id} 的商店失败：{err}"))?;
    if shops.is_empty() {
        return Ok(tool_ok(
            format!("没有商店出售{item_label}。"),
            with_zh_fields(
                json!({
                    "item_id": target_item_id,
                    "shops": [],
                }),
                &[("item_name", &item_name)],
            ),
        ));
    }

    let mut lines = vec![format!("出售{item_label}的商店（共 {} 家）：", shops.len())];
    let mut shops_json = Vec::with_capacity(shops.len());
    for shop in &shops {
        let shop_name = resolve_db_zh_text_clean(&db, &shop.name);
        let shop_display = match &shop_name {
            ZhText::Translated(n) => format!("「{n}」"),
            ZhText::Untranslated { ja } => ZhText::format_untranslated(ja),
            ZhText::Missing => "无中文店名".to_owned(),
        };
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
        shops_json.push(with_zh_fields(
            json!({
                "shop_id": shop.id,
                "row_key": shop.row_key,
                "offers": offers,
            }),
            &[("name", &shop_name)],
        ));
    }

    Ok(tool_ok(
        lines.join("\n"),
        with_zh_fields(
            json!({
                "item_id": target_item_id,
                "shops": shops_json,
            }),
            &[("item_name", &item_name)],
        ),
    ))
}

fn format_shop_output_db(db: &GameDb, resolved: &ResolvedShop) -> Result<ToolOutput, String> {
    let shop = &resolved.shop;
    let name = resolve_db_zh_text_clean(db, &shop.name);
    let shop_title = format!("商店 #{}{}", shop.id, name.bracketed_or_paren("无中文店名"));

    let mut goods = Vec::with_capacity(resolved.stock.len());
    let mut items = Vec::with_capacity(resolved.stock.len());
    for slot in &resolved.stock {
        let item_name = slot
            .item
            .as_ref()
            .map(|it| resolve_db_zh_text_clean(db, &it.name))
            .unwrap_or_default();
        let (display_name, price_str) = match (&slot.item, slot.price) {
            (Some(_), Some(price)) => (item_name.inline_or("无中文译名"), format!("{price}G")),
            (Some(it), None) => (
                item_name.inline_or("无中文译名"),
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
        items.push(with_zh_fields(
            json!({
                "item_id": slot.entry.item_id,
                "price": slot.price,
                "max_order_num": slot.entry.max_order_num,
                "price_ratio": slot.entry.price_ratio,
                "condition": slot.entry.condition,
            }),
            &[("item_name", &item_name)],
        ));
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

    let mut sell_item_names: HashMap<i64, ZhText> = HashMap::new();
    let mut sell_items = Vec::with_capacity(shop.sell_items.len());
    for entry in &shop.sell_items {
        let item_name = cached_item_name_zh(db, entry.item_id, &mut sell_item_names)?;
        sell_items.push(with_zh_fields(
            json!({
                "item_id": entry.item_id,
                "price_ratio": entry.price_ratio,
            }),
            &[("item_name", &item_name)],
        ));
    }

    Ok(tool_ok(
        summary,
        with_zh_fields(
            json!({
                "shop_id": shop.id,
                "row_key": shop.row_key,
                "items": items,
                "sell_items": sell_items,
            }),
            &[("name", &name)],
        ),
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
            let name = resolve_db_zh_text_clean(&db, &ch.name);
            if let Some(rank) = match_rank_zh_text(&name, query) {
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
        let mut seen = HashSet::new();
        let maps = db
            .maps_by_name(query, MODEL_LANGUAGE)
            .map_err(|err| format!("检索地图失败：{err}"))?;
        for map in maps {
            let name = resolve_db_zh_text_clean(&db, &map.name);
            if let Some(rank) = match_rank_zh_text(&name, query)
                && seen.insert(map.id)
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
        let untranslated = db
            .untranslated_maps(MODEL_LANGUAGE)
            .map_err(|err| format!("检索未翻译地图失败：{err}"))?;
        for entry in untranslated {
            if entry.field != "name" || seen.contains(&entry.key) {
                continue;
            }
            if let Some(clean_ja) = collapse_ws(&entry.value)
                && match_rank(&clean_ja, query).is_some()
                && let Some(map) = db
                    .map(entry.key)
                    .map_err(|err| format!("查询地图 {} 失败：{err}", entry.key))?
            {
                let name = resolve_db_zh_text_clean(&db, &map.name);
                if let Some(rank) = match_rank_zh_text(&name, query)
                    && seen.insert(map.id)
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
    }

    if kinds.contains(&SearchKind::Item) {
        let mut seen = HashSet::new();
        let items = db
            .items_by_name(query, MODEL_LANGUAGE)
            .map_err(|err| format!("检索道具失败：{err}"))?;
        for item in items {
            let name = resolve_db_zh_text_clean(&db, &item.name);
            if let Some(rank) = match_rank_zh_text(&name, query)
                && seen.insert(item.id)
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
        let untranslated = db
            .untranslated_items(MODEL_LANGUAGE)
            .map_err(|err| format!("检索未翻译道具失败：{err}"))?;
        for entry in untranslated {
            if entry.field != "name" || seen.contains(&entry.key) {
                continue;
            }
            if let Some(clean_ja) = collapse_ws(&entry.value)
                && match_rank(&clean_ja, query).is_some()
                && let Some(item) = db
                    .item(entry.key)
                    .map_err(|err| format!("查询道具 {} 失败：{err}", entry.key))?
            {
                let name = resolve_db_zh_text_clean(&db, &item.name);
                if let Some(rank) = match_rank_zh_text(&name, query)
                    && seen.insert(item.id)
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
    }

    if kinds.contains(&SearchKind::Quest) {
        let mut seen = HashSet::new();
        let quests = db
            .quests_by_name_zh(query)
            .map_err(|err| format!("检索支线任务失败：{err}"))?;
        for quest in quests {
            let name = resolve_db_zh_text_clean(&db, &quest.title);
            if let Some(rank) = match_rank_zh_text(&name, query)
                && seen.insert(quest.id)
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
        let untranslated = db
            .untranslated_quests(MODEL_LANGUAGE)
            .map_err(|err| format!("检索未翻译支线任务失败：{err}"))?;
        for entry in untranslated {
            if entry.field != "title" || seen.contains(&entry.key) {
                continue;
            }
            if let Some(clean_ja) = collapse_ws(&entry.value)
                && match_rank(&clean_ja, query).is_some()
                && let Some(quest) = db
                    .quest(entry.key)
                    .map_err(|err| format!("查询支线任务 {} 失败：{err}", entry.key))?
            {
                let name = resolve_db_zh_text_clean(&db, &quest.title);
                if let Some(rank) = match_rank_zh_text(&name, query)
                    && seen.insert(quest.id)
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
    }

    if kinds.contains(&SearchKind::Shop) {
        let mut seen = HashSet::new();
        let shops = db
            .shops_by_name_zh(query)
            .map_err(|err| format!("检索商店失败：{err}"))?;
        for shop in shops {
            let name = resolve_db_zh_text_clean(&db, &shop.name);
            if let Some(rank) = match_rank_zh_text(&name, query)
                && seen.insert(shop.id)
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
        let untranslated = db
            .untranslated_shops(MODEL_LANGUAGE)
            .map_err(|err| format!("检索未翻译商店失败：{err}"))?;
        for entry in untranslated {
            if entry.field != "name" || seen.contains(&entry.key) {
                continue;
            }
            if let Some(clean_ja) = collapse_ws(&entry.value)
                && match_rank(&clean_ja, query).is_some()
                && let Some(resolved) = db
                    .shop(entry.key)
                    .map_err(|err| format!("查询商店 {} 失败：{err}", entry.key))?
            {
                let shop = &resolved.shop;
                let name = resolve_db_zh_text_clean(&db, &shop.name);
                if let Some(rank) = match_rank_zh_text(&name, query)
                    && seen.insert(shop.id)
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

    if kinds.contains(&SearchKind::Enemy) {
        let mut seen = HashSet::new();
        let units = db
            .units_by_name(query, MODEL_LANGUAGE)
            .map_err(|err| format!("检索敌人失败：{err}"))?;
        for unit in units {
            if !unit.category.is_enemy_category() {
                continue;
            }
            let name = resolve_db_zh_text_clean(&db, &unit.name);
            if let Some(rank) = match_rank_zh_text(&name, query)
                && seen.insert(unit.id)
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
        let untranslated = db
            .untranslated_units(MODEL_LANGUAGE)
            .map_err(|err| format!("检索未翻译敌人失败：{err}"))?;
        for entry in untranslated {
            if entry.field != "name" || seen.contains(&entry.key) {
                continue;
            }
            if let Some(clean_ja) = collapse_ws(&entry.value)
                && match_rank(&clean_ja, query).is_some()
                && let Some(unit) = db
                    .unit(entry.key)
                    .map_err(|err| format!("查询单位 {} 失败：{err}", entry.key))?
                && unit.category.is_enemy_category()
            {
                let name = resolve_db_zh_text_clean(&db, &unit.name);
                if let Some(rank) = match_rank_zh_text(&name, query)
                    && seen.insert(unit.id)
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
    }

    if kinds.contains(&SearchKind::Skill) {
        let mut seen = HashSet::new();
        let skills = db
            .skills_by_name(query, MODEL_LANGUAGE)
            .map_err(|err| format!("检索技能失败：{err}"))?;
        for skill in skills {
            let name = resolve_db_zh_text_clean(&db, &skill.name);
            if let Some(rank) = match_rank_zh_text(&name, query)
                && seen.insert(skill.id)
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
        let untranslated = db
            .untranslated_skills(MODEL_LANGUAGE)
            .map_err(|err| format!("检索未翻译技能失败：{err}"))?;
        for entry in untranslated {
            if entry.field != "name" || seen.contains(&entry.key) {
                continue;
            }
            if let Some(clean_ja) = collapse_ws(&entry.value)
                && match_rank(&clean_ja, query).is_some()
                && let Some(skill) = db
                    .skill(entry.key)
                    .map_err(|err| format!("查询技能 {} 失败：{err}", entry.key))?
            {
                let name = resolve_db_zh_text_clean(&db, &skill.name);
                if let Some(rank) = match_rank_zh_text(&name, query)
                    && seen.insert(skill.id)
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

    Ok(format_search_output(query, &kinds, hits, &[]))
}

// ---- DB-backed helpers ----

fn format_scenario_destinations_db(
    db: &GameDb,
    entry: &MainScenarioEntry,
    map_names: &mut HashMap<i64, ZhText>,
) -> Result<(Vec<String>, Vec<Value>), String> {
    let mut labels = Vec::with_capacity(entry.destinations.len());
    let mut json_list = Vec::with_capacity(entry.destinations.len());
    for dest in &entry.destinations {
        let map_name = cached_map_name_zh(db, dest.destination_id, map_names)?;
        labels.push(format_map_ref_zh(dest.destination_id, &map_name));
        json_list.push(with_zh_fields(
            json!({
                "map_id": dest.destination_id,
                "point_type": dest.point_type,
                "destination_type": dest.destination_type,
                "is_point_to_object": dest.is_point_to_object,
                "object_type": dest.object_type,
                "object_id": dest.object_id,
                "condition": dest.condition,
            }),
            &[("map_name", &map_name)],
        ));
    }
    Ok((labels, json_list))
}

fn build_quest_summary_db(
    db: &GameDb,
    quest: &Quest,
    chapter_names: &mut HashMap<i64, ZhText>,
    map_names: &mut HashMap<i64, ZhText>,
    item_names: &mut HashMap<i64, ZhText>,
) -> Result<String, String> {
    let title = resolve_db_zh_text_clean(db, &quest.title);
    let chapter_name = match quest.chapter_no {
        Some(ch) => Some(cached_chapter_name_zh(db, ch, chapter_names)?),
        None => None,
    };
    let requester = resolve_db_zh_text_clean(db, &quest.requester);
    let requester_map_name = match quest.requester_map_id {
        Some(map_id) => Some(cached_map_name_zh(db, map_id, map_names)?),
        None => None,
    };
    let goals: Vec<ZhText> = quest
        .objectives
        .iter()
        .map(|stage| resolve_db_zh_text_clean(db, &stage.goal_text))
        .collect();
    let mut prizes = Vec::with_capacity(quest.prizes.len());
    for prize in &quest.prizes {
        let item_name = match prize.item_id {
            Some(id) => cached_item_name_zh(db, id, item_names)?,
            None => ZhText::Missing,
        };
        let text = resolve_db_zh_text_clean(db, &prize.text);
        prizes.push((prize.item_id, item_name, prize.count, text));
    }
    Ok(build_quest_summary_shared(
        quest,
        &title,
        chapter_name.as_ref(),
        &requester,
        requester_map_name.as_ref(),
        &goals,
        &prizes,
    ))
}

// ---- Per-request small lookup caches ----

pub(crate) fn cached_map_name_zh(
    db: &GameDb,
    map_id: i64,
    cache: &mut HashMap<i64, ZhText>,
) -> Result<ZhText, String> {
    if let Some(hit) = cache.get(&map_id) {
        return Ok(hit.clone());
    }
    let name = db
        .map(map_id)
        .map_err(|err| format!("查询地图 {map_id} 失败：{err}"))?
        .map(|m| resolve_db_zh_text_clean(db, &m.name))
        .unwrap_or_default();
    cache.insert(map_id, name.clone());
    Ok(name)
}

pub(crate) fn cached_item_name_zh(
    db: &GameDb,
    item_id: i64,
    cache: &mut HashMap<i64, ZhText>,
) -> Result<ZhText, String> {
    if let Some(hit) = cache.get(&item_id) {
        return Ok(hit.clone());
    }
    let name = db
        .item(item_id)
        .map_err(|err| format!("查询道具 {item_id} 失败：{err}"))?
        .map(|it| resolve_db_zh_text_clean(db, &it.name))
        .unwrap_or_default();
    cache.insert(item_id, name.clone());
    Ok(name)
}

fn cached_chapter_name_zh(
    db: &GameDb,
    chapter_no: i64,
    cache: &mut HashMap<i64, ZhText>,
) -> Result<ZhText, String> {
    if let Some(hit) = cache.get(&chapter_no) {
        return Ok(hit.clone());
    }
    let name = db
        .chapter(chapter_no)
        .map_err(|err| format!("查询第 {chapter_no} 章失败：{err}"))?
        .map(|c| resolve_db_zh_text_clean(db, &c.name))
        .unwrap_or_default();
    cache.insert(chapter_no, name.clone());
    Ok(name)
}
