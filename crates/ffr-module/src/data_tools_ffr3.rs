//! Extended `ffr-3` static game-data tools backed by `ffr-db`.
//!
//! Implements:
//! - `find_route`: shortest map-to-map route including save-point teleports, conditions, and landing coordinates.
//! - `where_to_get`: complete item acquisition sources (chests, shop NPCs, enemy drops/steals, quest rewards).
//! - `read_npc`: lookup by `(map_id, unique_id)`, `map_unit_id`, or `name`.
//! - `read_talk`: dialogue lines, continuation/choice branches, linked cutscene sequence, and battle group.
//! - `whats_near`: spatial proximity query defaulting to live bridge position or explicit map + coordinates.
//! - `query_game_db`: bounded read-only SQL (<=200 rows, 2s progress-handler timeout, ~20 KB truncation).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use agp_game::ToolOutput;
use agp_platform::FileChannel;
use ffr_data::MODEL_LANGUAGE;
use ffr_data::assets::level::{LevelObjectData, LevelObjectKind, MapEdgeKind, MapExit, Vec3};
use ffr_data::world::{Item, MapInfo};
use ffr_db::db::is_query_timeout_error;
use ffr_db::query::links::{MapUnit, NpcInfo};
use ffr_db::query::world::{ItemDropKind, ItemSource};
use ffr_db::{DbError, GameDb};
use serde_json::{Map, Value, json};

use crate::data_cache::DbDataCache;
use crate::data_tools::{
    collapse_ws, facility_zh, format_item_candidates, map_type_label_shared as map_type_label,
    optional_i64, optional_non_empty_str, reject_unknown_keys, required_non_empty_str, tool_ok,
};
use crate::data_tools_db::{
    cached_item_name_zh, cached_map_name_zh, resolve_db_zh_clean, resolve_db_zh_raw,
};
use crate::state_tools::{interact_type_zh, obj_f64, obj_i64, obj_object, obj_str};
use crate::text_format::{
    ZhText, format_map_ref_zh, resolve_db_map_unit_name_zh, resolve_db_text_key_clean,
    resolve_db_zh_text_clean, with_zh_fields,
};

/// Default search radius (in UE units) for `whats_near`.
pub(crate) const DEFAULT_NEAR_RADIUS: f64 = 2000.0;
/// Distance threshold (in UE units) below which `whats_near` reports "就在这里" with `direction: null`.
pub(crate) const NEAR_HERE_DISTANCE: f64 = 50.0;
/// World-space azimuth in degrees (measured from `+X` toward `+Y`) treated as compass North (0°).
/// Currently assumed to be `0.0` (`+X` = North, `+Y` = East) and not yet verified on live hardware;
/// update this single constant after checking `VM_MiniMap_C.PlayerDirectionDegree` against player movement.
pub(crate) const COMPASS_NORTH_AZIMUTH_DEG: f64 = 0.0;
/// Maximum rows returned by `query_game_db`.
pub(crate) const QUERY_DB_MAX_ROWS: usize = 200;
/// Wall-clock timeout enforced via SQLite progress handler in `query_game_db`.
pub(crate) const QUERY_DB_TIMEOUT: Duration = Duration::from_secs(2);
/// Approximate serialized JSON output cap (~20 KB) for `query_game_db`.
pub(crate) const QUERY_DB_MAX_OUTPUT_BYTES: usize = 20 * 1024;
/// Reserved bytes for metadata keys around `rows` in `query_game_db` JSON payload.
const QUERY_DB_WRAPPER_RESERVE_BYTES: usize = 1024;
/// Maximum preview rows included in `query_game_db` Chinese summary text.
const QUERY_DB_SUMMARY_PREVIEW_ROWS: usize = 8;

// ---- 1. find_route ----

pub(crate) fn handle_find_route_db(
    cache: &DbDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("find_route", args, &["from", "to"])?;
    let from_raw = args
        .get("from")
        .filter(|v| !v.is_null())
        .ok_or_else(|| "工具 `find_route` 缺少必需参数 `from`（地图编号或中文名）".to_owned())?;
    let to_raw = args
        .get("to")
        .filter(|v| !v.is_null())
        .ok_or_else(|| "工具 `find_route` 缺少必需参数 `to`（地图编号或中文名）".to_owned())?;

    let db = cache.open()?;
    let from_res = resolve_map_selector(&db, "from", from_raw)?;
    let to_res = resolve_map_selector(&db, "to", to_raw)?;

    match (from_res, to_res) {
        (MapSelectorResult::Ambiguous(a), MapSelectorResult::Ambiguous(b)) => {
            let mut lines = vec![
                "参数 `from` 与 `to` 均存在重名地图，请改用地图编号指定：".to_owned(),
                format!("起点 `from`（「{}」）候选：", a.query),
            ];
            lines.extend(format_map_candidate_lines(&db, &a.maps));
            lines.push(format!("终点 `to`（「{}」）候选：", b.query));
            lines.extend(format_map_candidate_lines(&db, &b.maps));
            let from_cands = build_map_candidates_json(&db, &a.maps);
            let to_cands = build_map_candidates_json(&db, &b.maps);
            Ok(tool_ok(
                lines.join("\n"),
                json!({
                    "ambiguous": true,
                    "parameters": ["from", "to"],
                    "from_query": a.query,
                    "to_query": b.query,
                    "from_candidates": from_cands,
                    "to_candidates": to_cands,
                }),
            ))
        }
        (MapSelectorResult::Ambiguous(amb), _) | (_, MapSelectorResult::Ambiguous(amb)) => {
            let mut lines = vec![format!(
                "参数 `{}` 的地图名称「{}」匹配到 {} 张候选地图，请改用地图编号精确指定：",
                amb.param,
                amb.query,
                amb.maps.len()
            )];
            lines.extend(format_map_candidate_lines(&db, &amb.maps));
            let candidates = build_map_candidates_json(&db, &amb.maps);
            Ok(tool_ok(
                lines.join("\n"),
                json!({
                    "ambiguous": true,
                    "parameter": amb.param,
                    "query": amb.query,
                    "candidates": candidates,
                }),
            ))
        }
        (MapSelectorResult::One(from_map), MapSelectorResult::One(to_map)) => {
            format_route_output_db(&db, &from_map, &to_map)
        }
    }
}

struct AmbiguousMaps {
    param: &'static str,
    query: String,
    maps: Vec<MapInfo>,
}

enum MapSelectorResult {
    One(Box<MapInfo>),
    Ambiguous(AmbiguousMaps),
}

fn resolve_map_selector(
    db: &GameDb,
    param: &'static str,
    raw: &Value,
) -> Result<MapSelectorResult, String> {
    if let Some(id) = raw.as_i64() {
        let map = db
            .map(id)
            .map_err(|err| format!("查询参数 `{param}` 地图 #{id} 失败：{err}"))?
            .ok_or_else(|| format!("参数 `{param}` 找不到编号为 {id} 的地图"))?;
        return Ok(MapSelectorResult::One(Box::new(map)));
    }
    let Some(text) = raw.as_str() else {
        return Err(format!(
            "工具 `find_route` 的参数 `{param}` 必须是地图编号（整数）或地图简体中文名（字符串）"
        ));
    };
    let query = text.trim();
    if query.is_empty() {
        return Err(format!("工具 `find_route` 的参数 `{param}` 不能为空字符串"));
    }

    let candidates = db
        .maps_by_name(query, MODEL_LANGUAGE)
        .map_err(|err| format!("按名称查询参数 `{param}` 地图失败：{err}"))?;
    let mut exact = Vec::new();
    let mut similar = Vec::new();
    for map in candidates {
        let Some(name) = resolve_db_zh_clean(db, &map.name) else {
            continue;
        };
        if name == query {
            exact.push(map);
        } else if similar.len() < 5 && name.contains(query) {
            similar.push(format!("{name}（#{}）", map.id));
        }
    }

    match exact.len() {
        0 => {
            if similar.is_empty() {
                Err(format!(
                    "参数 `{param}` 找不到简体中文名为「{query}」的地图，请改用地图编号或用 search_data 查找"
                ))
            } else {
                Err(format!(
                    "参数 `{param}` 找不到完全名为「{query}」的地图；相近候选：{}（请改用地图编号查询）",
                    similar.join("、")
                ))
            }
        }
        1 => Ok(MapSelectorResult::One(Box::new(exact.remove(0)))),
        _ => Ok(MapSelectorResult::Ambiguous(AmbiguousMaps {
            param,
            query: query.to_owned(),
            maps: exact,
        })),
    }
}

fn format_map_candidate_lines(db: &GameDb, maps: &[MapInfo]) -> Vec<String> {
    maps.iter()
        .map(|m| {
            let name = resolve_db_zh_text_clean(db, &m.name);
            format!(
                "- map_id={}{}（关卡 {}，{}）",
                m.id,
                name.bracketed_or("未命名地图"),
                m.level_name,
                map_type_label(&m.map_type)
            )
        })
        .collect()
}

fn build_map_candidates_json(db: &GameDb, maps: &[MapInfo]) -> Vec<Value> {
    maps.iter()
        .map(|m| {
            let name = resolve_db_zh_text_clean(db, &m.name);
            with_zh_fields(
                json!({
                    "map_id": m.id,
                    "level_name": m.level_name,
                    "map_type": m.map_type,
                    "island": m.island,
                }),
                &[("name", &name)],
            )
        })
        .collect()
}

fn format_route_output_db(
    db: &GameDb,
    from_map: &MapInfo,
    to_map: &MapInfo,
) -> Result<ToolOutput, String> {
    let from_name = resolve_db_zh_text_clean(db, &from_map.name);
    let to_name = resolve_db_zh_text_clean(db, &to_map.name);
    let from_label = format_map_ref_zh(from_map.id, &from_name);
    let to_label = format_map_ref_zh(to_map.id, &to_name);
    let from_json = with_zh_fields(
        json!({ "map_id": from_map.id, "level_name": from_map.level_name }),
        &[("name", &from_name)],
    );
    let to_json = with_zh_fields(
        json!({ "map_id": to_map.id, "level_name": to_map.level_name }),
        &[("name", &to_name)],
    );

    if from_map.id == to_map.id {
        return Ok(tool_ok(
            format!("起点与终点相同（{from_label}），无需换图（0 步）。"),
            json!({
                "from": from_json,
                "to": to_json,
                "reachable": true,
                "hops": 0,
                "uses_savepoint_teleport": false,
                "has_conditional_steps": false,
                "has_event_only_steps": false,
                "steps": [],
            }),
        ));
    }

    let route = db
        .map_route(from_map.id, to_map.id)
        .map_err(|err| format!("计算从 {} 到 {} 的路线失败：{err}", from_map.id, to_map.id))?;

    let Some(edges) = route else {
        let to_teleportable = db
            .map_contents(to_map.id)
            .ok()
            .flatten()
            .is_some_and(|c| !c.teleport_points.is_empty());
        let mut msg = format!(
            "静态地图连接图（含存档点传送）中未找到从 {from_label} 到 {to_label} 的可达路线。"
        );
        if to_teleportable {
            msg.push_str("（目标地图在大地图上存在传送地标，可通过世界地图传送抵达。）");
        }
        return Ok(tool_ok(
            msg,
            json!({
                "from": from_json,
                "to": to_json,
                "reachable": false,
                "hops": null,
                "to_world_teleportable": to_teleportable,
                "steps": [],
            }),
        ));
    };

    let mut map_names: HashMap<i64, ZhText> = HashMap::new();
    map_names.insert(from_map.id, from_name.clone());
    map_names.insert(to_map.id, to_name.clone());

    let uses_savepoint_teleport = edges
        .iter()
        .any(|e| e.kind == MapEdgeKind::SavePointTeleport);
    let has_conditional_steps = edges.iter().any(|e| e.condition.is_some());
    let has_event_only_steps = edges.iter().any(|e| e.is_event_only);

    let mut lines = vec![format!(
        "从 {from_label} 到 {to_label} 的最短路线（共 {} 步）：",
        edges.len()
    )];
    let mut steps_json = Vec::with_capacity(edges.len());

    for (idx, edge) in edges.iter().enumerate() {
        let step_no = idx + 1;
        let step_from_name = cached_map_name_zh(db, edge.from_map_id, &mut map_names)?;
        let step_to_id = edge.target_map_id;
        let step_to_name = match step_to_id {
            Some(tid) => cached_map_name_zh(db, tid, &mut map_names)?,
            None => ZhText::Missing,
        };
        let step_from_label = format_map_ref_zh(edge.from_map_id, &step_from_name);
        let step_to_label = match step_to_id {
            Some(tid) => format_map_ref_zh(tid, &step_to_name),
            None => "未知地图".to_owned(),
        };

        let kind_zh = map_edge_kind_zh(edge.kind);
        let exit_pos_str = edge
            .location
            .map_or_else(|| "坐标未知".to_owned(), format_vec3_zh);
        let target_pos_str = edge
            .target_location
            .map_or_else(|| "落点坐标未知".to_owned(), format_vec3_zh);
        let mut tags = Vec::new();
        if edge.kind == MapEdgeKind::SavePointTeleport {
            tags.push("存档点传送".to_owned());
        } else if edge.is_auto_transition {
            tags.push("自动触发".to_owned());
        } else {
            tags.push("需交互触发".to_owned());
        }
        if edge.is_event_only {
            tags.push("仅事件触发".to_owned());
        }
        if let Some(cond) = &edge.condition {
            tags.push(format!("条件：{cond}"));
        }
        let target_point_str = edge
            .target_point_id
            .map_or_else(String::new, |p| format!(" #{p}"));
        lines.push(format!(
            "{step_no}. {step_from_label} -> {step_to_label}（{kind_zh}，出口 {exit_pos_str} -> 落点{target_point_str} {target_pos_str}；{}）",
            tags.join("，")
        ));

        steps_json.push(format_map_exit_json(
            edge,
            step_no,
            &step_from_name,
            &step_to_name,
        ));
    }

    Ok(tool_ok(
        lines.join("\n"),
        json!({
            "from": from_json,
            "to": to_json,
            "reachable": true,
            "hops": edges.len(),
            "uses_savepoint_teleport": uses_savepoint_teleport,
            "has_conditional_steps": has_conditional_steps,
            "has_event_only_steps": has_event_only_steps,
            "steps": steps_json,
        }),
    ))
}

fn format_map_exit_json(
    edge: &MapExit,
    step_no: usize,
    from_name: &ZhText,
    to_name: &ZhText,
) -> Value {
    let dir_zh = edge
        .target_direction
        .as_deref()
        .and_then(map_direction_enum_zh);
    with_zh_fields(
        json!({
            "step": step_no,
            "from_map_id": edge.from_map_id,
            "to_map_id": edge.target_map_id,
            "kind": edge.kind.as_str(),
            "kind_zh": map_edge_kind_zh(edge.kind),
            "is_savepoint_teleport": edge.kind == MapEdgeKind::SavePointTeleport,
            "source_level": edge.source_level,
            "actor_name": edge.actor_name,
            "unique_id": edge.unique_id,
            "exit_location": edge.location.map(vec3_json),
            "target_point_id": edge.target_point_id,
            "target_location": edge.target_location.map(vec3_json),
            "target_direction": edge.target_direction,
            "target_direction_zh": dir_zh,
            "is_auto_transition": edge.is_auto_transition,
            "is_event_only": edge.is_event_only,
            "condition": edge.condition,
        }),
        &[("from_map_name", from_name), ("to_map_name", to_name)],
    )
}

// ---- 2. where_to_get ----

pub(crate) fn handle_where_to_get_db(
    cache: &DbDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("where_to_get", args, &["item_id", "name"])?;
    let item_id = optional_i64("where_to_get", args, "item_id")?;
    let name = optional_non_empty_str("where_to_get", args, "name")?;

    let db = cache.open()?;
    let item = match (item_id, name) {
        (Some(id), None) => db
            .item(id)
            .map_err(|err| format!("查询道具 {id} 失败：{err}"))?
            .ok_or_else(|| format!("参数 `item_id` 在道具表中找不到编号为 {id} 的道具"))?,
        (None, Some(query_name)) => {
            let candidates = db
                .items_by_name(query_name, MODEL_LANGUAGE)
                .map_err(|err| format!("按名称查询道具失败：{err}"))?;
            let mut exact: Vec<&Item> = Vec::new();
            let mut contains: Vec<(&Item, String)> = Vec::new();
            for it in &candidates {
                let Some(raw) = resolve_db_zh_raw(&db, &it.name) else {
                    continue;
                };
                if raw.trim() == query_name {
                    exact.push(it);
                }
                if raw.contains(query_name) {
                    contains.push((it, raw));
                }
            }
            match exact.as_slice() {
                [] => {
                    let similar: Vec<String> = contains
                        .into_iter()
                        .take(5)
                        .filter_map(|(it, raw)| {
                            collapse_ws(&raw).map(|n| format!("{n}（#{}）", it.id))
                        })
                        .collect();
                    if similar.is_empty() {
                        return Err(format!(
                            "参数 `name` 找不到简体中文名为「{query_name}」的道具，请用 search_data 模糊查找编号"
                        ));
                    }
                    return Err(format!(
                        "参数 `name` 找不到完全名为「{query_name}」的道具；相近候选：{}（请改用 item_id 或 search_data 查询）",
                        similar.join("、")
                    ));
                }
                [single] => (*single).clone(),
                many => return Ok(format_item_candidates(query_name, many)),
            }
        }
        (None, None) => {
            return Err("工具 `where_to_get` 必须提供 `item_id` 或 `name` 之一".to_owned());
        }
        (Some(_), Some(_)) => {
            return Err(
                "工具 `where_to_get` 的 `item_id` 与 `name` 不能同时传入，请二选一".to_owned(),
            );
        }
    };

    format_where_to_get_output_db(&db, &item)
}

fn format_where_to_get_output_db(db: &GameDb, item: &Item) -> Result<ToolOutput, String> {
    let item_name = resolve_db_zh_text_clean(db, &item.name);
    let item_display = format!(
        "道具 #{}{}",
        item.id,
        item_name.bracketed_or_paren("无中文译名")
    );

    let sources = db
        .item_sources(item.id)
        .map_err(|err| format!("查询道具 {} 来源失败：{err}", item.id))?;
    let chest_placements = db
        .treasure_box_objects_for_item(item.id)
        .map_err(|err| format!("查询道具 {} 的宝箱坐标失败：{err}", item.id))?;

    let mut chests_json = Vec::new();
    let mut shops_json = Vec::new();
    let mut enemy_drops_json = Vec::new();
    let mut quest_rewards_json = Vec::new();

    let mut chest_lines = Vec::new();
    let mut shop_lines = Vec::new();
    let mut drop_lines = Vec::new();
    let mut quest_lines = Vec::new();

    let mut enemy_chapters_cache: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut map_names_cache: HashMap<i64, ZhText> = HashMap::new();
    let mut shop_names_cache: HashMap<i64, ZhText> = HashMap::new();
    let mut used_placement_indices: HashSet<usize> = HashSet::new();

    for source in sources {
        match source {
            ItemSource::TreasureBox {
                map_id,
                map_name_zh,
                box_id,
                count,
                is_trap,
            } => {
                // Match level placement by (map_id, chest_id), falling back to an unmatched placement
                // on the same map for this item.
                let matched_idx = chest_placements
                    .iter()
                    .enumerate()
                    .find(|(idx, lo)| {
                        !used_placement_indices.contains(idx)
                            && lo.map_id == Some(map_id)
                            && matches!(
                                &lo.data,
                                LevelObjectData::TreasureBox(tp) if tp.chest_id == Some(box_id)
                            )
                    })
                    .or_else(|| {
                        chest_placements.iter().enumerate().find(|(idx, lo)| {
                            !used_placement_indices.contains(idx) && lo.map_id == Some(map_id)
                        })
                    })
                    .map(|(idx, _)| idx);

                let placement = matched_idx.map(|idx| {
                    used_placement_indices.insert(idx);
                    &chest_placements[idx]
                });
                let location = placement.and_then(|p| p.location);
                let visible_condition = placement.and_then(|p| p.visible_condition.clone());
                let unique_id = placement.and_then(|p| p.unique_id);
                let battle_group_id = if is_trap {
                    db.treasure_box(map_id, box_id)
                        .ok()
                        .flatten()
                        .and_then(|tb| tb.battle_group_id)
                } else {
                    None
                };

                let map_name = resolve_opt_map_name_zh(
                    db,
                    map_id,
                    map_name_zh.as_deref(),
                    &mut map_names_cache,
                )?;
                let map_label = format_map_ref_zh(map_id, &map_name);
                let pos_part =
                    location.map_or_else(String::new, |p| format!("，坐标 {}", format_vec3_zh(p)));
                let cond_part = visible_condition
                    .as_deref()
                    .map_or_else(String::new, |c| format!("，条件：{c}"));
                let trap_part = if is_trap { "，陷阱箱" } else { "" };
                chest_lines.push(format!(
                    "  - {map_label} 宝箱 #{box_id}（×{count}{trap_part}{pos_part}{cond_part}）"
                ));

                chests_json.push(with_zh_fields(
                    json!({
                        "map_id": map_id,
                        "box_id": box_id,
                        "unique_id": unique_id,
                        "count": count,
                        "is_trap": is_trap,
                        "battle_group_id": battle_group_id,
                        "location": location.map(vec3_json),
                        "visible_condition": visible_condition,
                    }),
                    &[("map_name", &map_name)],
                ));
            }
            ItemSource::Shop {
                shop_id,
                shop_name_zh,
                price,
                price_ratio,
                max_order_num,
                condition,
            } => {
                let locations = db
                    .shop_locations(shop_id)
                    .map_err(|err| format!("查询商店 {shop_id} 的地图位置失败：{err}"))?;
                let price_label = price.map_or_else(|| "价格未知".to_owned(), |p| format!("{p}G"));
                let item_cond_non_empty = (!condition.is_empty()).then_some(condition.clone());
                let mut loc_json = Vec::with_capacity(locations.len());
                let mut loc_descs = Vec::with_capacity(locations.len());
                for loc in &locations {
                    let loc_map_name = resolve_opt_map_name_zh(
                        db,
                        loc.map_id,
                        loc.map_name_zh.as_deref(),
                        &mut map_names_cache,
                    )?;
                    let loc_npc_name_zh = resolve_db_map_unit_name_zh(
                        db,
                        loc.npc_name_zh.as_deref(),
                        None,
                        loc.map_unit_id,
                    );
                    let map_label = format_map_ref_zh(loc.map_id, &loc_map_name);
                    let npc_label = format_npc_short_label(
                        loc.unique_id,
                        &loc_npc_name_zh,
                        loc.npc_name.as_deref(),
                        loc.interact_type.as_deref(),
                    );
                    let mut extra = Vec::new();
                    if let Some(vc) = &loc.visible_condition {
                        extra.push(format!("NPC条件：{vc}"));
                    }
                    if let Some(pos) = loc.location {
                        extra.push(format!("坐标 {}", format_vec3_zh(pos)));
                    }
                    let extra_str = if extra.is_empty() {
                        String::new()
                    } else {
                        format!("（{}）", extra.join("，"))
                    };
                    loc_descs.push(format!("{map_label} {npc_label}{extra_str}"));

                    loc_json.push(with_zh_fields(
                        json!({
                            "map_id": loc.map_id,
                            "unique_id": loc.unique_id,
                            "map_unit_id": loc.map_unit_id,
                            "npc_name": loc.npc_name,
                            "various_shop_id": loc.various_shop_id,
                            "interact_type": loc.interact_type,
                            "interact_type_zh": loc.interact_type.as_deref().map(facility_zh),
                            "visible_condition": loc.visible_condition,
                            "location": loc.location.map(vec3_json),
                        }),
                        &[
                            ("map_name", &loc_map_name),
                            ("npc_name_zh", &loc_npc_name_zh),
                        ],
                    ));
                }

                let shop_name = resolve_opt_shop_name_zh(
                    db,
                    shop_id,
                    shop_name_zh.as_deref(),
                    &mut shop_names_cache,
                )?;
                let shop_title = format!("商店 #{shop_id}{}", shop_name.bracketed_or_empty());
                let cond_part = item_cond_non_empty
                    .as_deref()
                    .map_or_else(String::new, |c| format!("，上架条件：{c}"));
                let limit_part = max_order_num.map_or_else(String::new, |m| format!("，限购 {m}"));
                let where_part = if loc_descs.is_empty() {
                    "未在关卡中找到对应 NPC".to_owned()
                } else {
                    loc_descs.join("；")
                };
                shop_lines.push(format!(
                    "  - {shop_title}（{price_label}{limit_part}{cond_part}）— {where_part}"
                ));

                shops_json.push(with_zh_fields(
                    json!({
                        "shop_id": shop_id,
                        "price": price,
                        "price_ratio": price_ratio,
                        "max_order_num": max_order_num,
                        "condition": item_cond_non_empty,
                        "locations": loc_json,
                    }),
                    &[("shop_name", &shop_name)],
                ));
            }
            ItemSource::EnemyDrop {
                unit_id,
                unit_name_zh,
                drop_kind,
                rate,
            } => {
                let chapters = if let Some(cached) = enemy_chapters_cache.get(&unit_id) {
                    cached.clone()
                } else {
                    let chs = db
                        .chapters_of_enemy(unit_id)
                        .map_err(|err| format!("查询敌人 {unit_id} 出没章节失败：{err}"))?;
                    enemy_chapters_cache.insert(unit_id, chs.clone());
                    chs
                };

                let unit_name = resolve_opt_unit_name_zh(db, unit_id, unit_name_zh.as_deref());
                let kind_code = drop_kind_code(drop_kind);
                let kind_zh = drop_kind_zh(drop_kind);
                let enemy_label = match &unit_name {
                    ZhText::Translated(n) => format!("{n}（#{unit_id}）"),
                    ZhText::Untranslated { ja } => {
                        format!("单位 #{unit_id}{}", ZhText::format_untranslated(ja))
                    }
                    ZhText::Missing => format!("单位 #{unit_id}"),
                };
                let ch_part = if chapters.is_empty() {
                    "章节未定".to_owned()
                } else {
                    chapters
                        .iter()
                        .map(|c| format!("第{c}章"))
                        .collect::<Vec<_>>()
                        .join("、")
                };
                drop_lines.push(format!("  - {enemy_label}：{kind_zh} {rate}%（{ch_part}）"));

                enemy_drops_json.push(with_zh_fields(
                    json!({
                        "unit_id": unit_id,
                        "kind": kind_code,
                        "kind_zh": kind_zh,
                        "rate": rate,
                        "chapters": chapters,
                    }),
                    &[("unit_name", &unit_name)],
                ));
            }
            ItemSource::QuestPrize {
                quest_id,
                quest_name_zh,
                count,
            } => {
                let quest_name = resolve_opt_quest_name_zh(db, quest_id, quest_name_zh.as_deref());
                let q_label = format!("支线任务 #{quest_id}{}", quest_name.bracketed_or_empty());
                quest_lines.push(format!("  - {q_label}（×{count}）"));
                quest_rewards_json.push(with_zh_fields(
                    json!({
                        "source": "quest",
                        "quest_id": quest_id,
                        "count": count,
                    }),
                    &[("quest_name", &quest_name)],
                ));
            }
            ItemSource::QuestBoardReward {
                quest_id,
                quest_name_zh,
                count,
            } => {
                let quest_name =
                    resolve_opt_quest_board_name_zh(db, quest_id, quest_name_zh.as_deref());
                let q_label = format!("委托板任务 #{quest_id}{}", quest_name.bracketed_or_empty());
                let cnt_part = count.map_or_else(String::new, |c| format!("（×{c}）"));
                quest_lines.push(format!("  - {q_label}{cnt_part}"));
                quest_rewards_json.push(with_zh_fields(
                    json!({
                        "source": "quest_board",
                        "quest_id": quest_id,
                        "count": count,
                    }),
                    &[("quest_name", &quest_name)],
                ));
            }
        }
    }

    let total_sources =
        chests_json.len() + shops_json.len() + enemy_drops_json.len() + quest_rewards_json.len();
    let mut lines = vec![format!(
        "{item_display}获取途径（宝箱 {}、商店 {}、敌人掉落/偷窃 {}、任务奖励 {}）：",
        chests_json.len(),
        shops_json.len(),
        enemy_drops_json.len(),
        quest_rewards_json.len()
    )];
    if total_sources == 0 {
        lines.push("未在静态数据表中找到该道具的宝箱、商店、掉落/偷窃或任务奖励来源。".to_owned());
    } else {
        append_capped_section(&mut lines, "【宝箱】", &chest_lines, 15);
        append_capped_section(&mut lines, "【商店】", &shop_lines, 15);
        append_capped_section(&mut lines, "【敌人掉落与偷窃】", &drop_lines, 15);
        append_capped_section(&mut lines, "【任务奖励】", &quest_lines, 15);
    }

    Ok(tool_ok(
        lines.join("\n"),
        with_zh_fields(
            json!({
                "item_id": item.id,
                "buying_price": item.buying_price,
                "selling_price": item.selling_price,
                "counts": {
                    "treasure_boxes": chests_json.len(),
                    "shops": shops_json.len(),
                    "enemy_drops": enemy_drops_json.len(),
                    "quest_rewards": quest_rewards_json.len(),
                },
                "treasure_boxes": chests_json,
                "shops": shops_json,
                "enemy_drops": enemy_drops_json,
                "quest_rewards": quest_rewards_json,
            }),
            &[("item_name", &item_name)],
        ),
    ))
}

fn append_capped_section(out: &mut Vec<String>, header: &str, items: &[String], max_items: usize) {
    if items.is_empty() {
        return;
    }
    out.push(header.to_owned());
    for line in items.iter().take(max_items) {
        out.push(line.clone());
    }
    if items.len() > max_items {
        out.push(format!(
            "  …其余 {} 条省略，完整列表见 JSON。",
            items.len() - max_items
        ));
    }
}

// ---- 3. read_npc ----

pub(crate) fn handle_read_npc_db(
    cache: &DbDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys(
        "read_npc",
        args,
        &["map_id", "unique_id", "map_unit_id", "name"],
    )?;
    let map_id = optional_i64("read_npc", args, "map_id")?;
    let unique_id = optional_i64("read_npc", args, "unique_id")?;
    let map_unit_id = optional_i64("read_npc", args, "map_unit_id")?;
    let name = optional_non_empty_str("read_npc", args, "name")?;

    let mode_count = usize::from(unique_id.is_some())
        + usize::from(map_unit_id.is_some())
        + usize::from(name.is_some());
    if mode_count == 0 {
        if map_id.is_some() {
            return Err(
                "工具 `read_npc` 传入 `map_id` 时还需搭配 `unique_id`、`map_unit_id` 或 `name` 之一；若要列出某地图全部 NPC 请使用 `read_map`"
                    .to_owned(),
            );
        }
        return Err(
            "工具 `read_npc` 必须提供以下查询方式之一：(1) `map_id` + `unique_id`；(2) `map_unit_id`；(3) `name`"
                .to_owned(),
        );
    }
    if mode_count > 1 {
        return Err(
            "工具 `read_npc` 的 `unique_id`、`map_unit_id`、`name` 不能同时传入，请三选一"
                .to_owned(),
        );
    }

    let db = cache.open()?;

    // Mode A: (map_id, unique_id)
    if let Some(uid) = unique_id {
        let Some(mid) = map_id else {
            return Err("工具 `read_npc` 按 `unique_id` 查询时必须同时提供 `map_id`".to_owned());
        };
        let Some(npc) = db
            .npc(mid, uid)
            .map_err(|err| format!("查询地图 {mid} 的 NPC unique_id={uid} 失败：{err}"))?
        else {
            if db
                .map(mid)
                .map_err(|err| format!("校验地图 {mid} 失败：{err}"))?
                .is_none()
            {
                return Err(format!("参数 `map_id` 找不到编号为 {mid} 的地图"));
            }
            return Err(format!(
                "参数 `unique_id` 在地图 #{mid} 中找不到 unique_id={uid} 的 NPC"
            ));
        };
        return format_single_npc_output_db(&db, &npc);
    }

    // Mode B: map_unit_id (optional map_id filter)
    if let Some(mu_id) = map_unit_id {
        if let Some(mid) = map_id
            && db
                .map(mid)
                .map_err(|err| format!("校验地图 {mid} 失败：{err}"))?
                .is_none()
        {
            return Err(format!("参数 `map_id` 找不到编号为 {mid} 的地图"));
        }
        let map_unit = db
            .map_unit(mu_id)
            .map_err(|err| format!("查询地图单位 {mu_id} 失败：{err}"))?;
        let occurrences = db
            .where_is_map_unit(mu_id)
            .map_err(|err| format!("查询地图单位 {mu_id} 出现位置失败：{err}"))?;
        if map_unit.is_none() && occurrences.is_empty() {
            return Err(format!(
                "参数 `map_unit_id` 找不到编号为 {mu_id} 的地图单位"
            ));
        }
        return format_map_unit_npcs_output_db(&db, mu_id, map_unit.as_ref(), occurrences, map_id);
    }

    // Mode C: name (optional map_id filter)
    let query = name.expect("mode_count == 1 guaranteed name");
    if let Some(mid) = map_id
        && db
            .map(mid)
            .map_err(|err| format!("校验地图 {mid} 失败：{err}"))?
            .is_none()
    {
        return Err(format!("参数 `map_id` 找不到编号为 {mid} 的地图"));
    }

    let mut matches = db
        .search_map_units_by_name(query)
        .map_err(|err| format!("按名称搜索 NPC 失败：{err}"))?;
    let mut matched_ids: HashSet<i64> = matches.iter().map(|mu| mu.id).collect();
    let untranslated_units = db
        .untranslated_map_units(MODEL_LANGUAGE)
        .map_err(|err| format!("搜索未翻译 NPC 失败：{err}"))?;
    for entry in untranslated_units {
        if entry.field != "name" || matched_ids.contains(&entry.key) {
            continue;
        }
        if entry.value.contains(query)
            && let Some(mu) = db
                .map_unit(entry.key)
                .map_err(|err| format!("查询地图单位 {} 失败：{err}", entry.key))?
        {
            matched_ids.insert(mu.id);
            matches.push(mu);
        }
    }

    if let Some(mid) = map_id {
        let map_objs = db
            .map_objects(mid, Some(LevelObjectKind::MapUnit))
            .map_err(|err| format!("查询地图 {mid} 的 NPC 列表失败：{err}"))?;
        let present_ids: HashSet<i64> = map_objs
            .iter()
            .filter_map(|o| match &o.data {
                LevelObjectData::MapUnit(u) => u.map_unit_id,
                _ => None,
            })
            .collect();
        matches.retain(|mu| present_ids.contains(&mu.id));
    }

    if matches.is_empty() {
        return Err(match map_id {
            Some(mid) => format!("参数 `name` 在地图 #{mid} 中找不到名字包含「{query}」的 NPC"),
            None => format!("参数 `name` 找不到名字包含「{query}」的 NPC"),
        });
    }

    let exact: Vec<MapUnit> = matches
        .iter()
        .filter(|mu| {
            let resolved_zh =
                resolve_db_map_unit_name_zh(&db, mu.name_zh.as_deref(), Some(mu), Some(mu.id));
            resolved_zh.zh() == Some(query)
                || resolved_zh.untranslated_ja() == Some(query)
                || mu.name == query
                || mu.row_key == query
        })
        .cloned()
        .collect();

    let resolved_list = if !exact.is_empty() { exact } else { matches };
    if resolved_list.len() > 1 {
        let mut lines = vec![format!(
            "参数 `name`「{query}」匹配到 {} 个 NPC 单位候选，请改用 `map_unit_id` 或 `map_id` + `unique_id` 精确查询：",
            resolved_list.len()
        )];
        let mut cands = Vec::with_capacity(resolved_list.len());
        for mu in &resolved_list {
            let mu_name_zh =
                resolve_db_map_unit_name_zh(&db, mu.name_zh.as_deref(), Some(mu), Some(mu.id));
            let display = match &mu_name_zh {
                ZhText::Translated(zh) => format!("「{zh}」（内部名：{}）", mu.name),
                ZhText::Untranslated { ja } => {
                    format!("{}（内部名：{}）", ZhText::format_untranslated(ja), mu.name)
                }
                ZhText::Missing => format!("「{}」（无中文译名）", mu.name),
            };
            lines.push(format!("- map_unit_id={} {display}", mu.id));
            cands.push(with_zh_fields(
                json!({
                    "map_unit_id": mu.id,
                    "name": mu.name,
                    "row_key": mu.row_key,
                }),
                &[("name_zh", &mu_name_zh)],
            ));
        }
        return Ok(tool_ok(
            lines.join("\n"),
            json!({
                "ambiguous": true,
                "parameter": "name",
                "query": query,
                "map_id": map_id,
                "candidates": cands,
            }),
        ));
    }

    let single = &resolved_list[0];
    let occurrences = db
        .where_is_map_unit(single.id)
        .map_err(|err| format!("查询地图单位 {} 出现位置失败：{err}", single.id))?;
    format_map_unit_npcs_output_db(&db, single.id, Some(single), occurrences, map_id)
}

fn format_single_npc_output_db(db: &GameDb, npc: &NpcInfo) -> Result<ToolOutput, String> {
    let mut item_names: HashMap<i64, ZhText> = HashMap::new();
    let mut map_names: HashMap<i64, ZhText> = HashMap::new();
    let (summary, payload) = render_npc_detail_db(db, npc, &mut item_names, &mut map_names)?;
    Ok(tool_ok(summary, payload))
}

fn render_npc_detail_db(
    db: &GameDb,
    npc: &NpcInfo,
    item_names: &mut HashMap<i64, ZhText>,
    map_names: &mut HashMap<i64, ZhText>,
) -> Result<(String, Value), String> {
    let npc_name_zh =
        resolve_db_map_unit_name_zh(db, npc.name_zh.as_deref(), None, npc.map_unit_id);
    let name_display = match (&npc_name_zh, &npc.name) {
        (ZhText::Translated(zh), Some(raw)) => format!("「{zh}」（内部名：{raw}）"),
        (ZhText::Translated(zh), None) => format!("「{zh}」"),
        (ZhText::Untranslated { ja }, Some(raw)) => {
            format!("{}（内部名：{raw}）", ZhText::format_untranslated(ja))
        }
        (ZhText::Untranslated { ja }, None) => ZhText::format_untranslated(ja),
        (ZhText::Missing, Some(raw)) => format!("「{raw}」（无中文译名）"),
        (ZhText::Missing, None) => "未命名 NPC".to_owned(),
    };
    let map_name = resolve_opt_map_name_zh(db, npc.map_id, npc.map_name_zh.as_deref(), map_names)?;
    let map_label = format_map_ref_zh(npc.map_id, &map_name);
    let pos_str = npc
        .location
        .map_or_else(|| "坐标未知".to_owned(), format_vec3_zh);
    let interact_zh = npc.interact_type.as_deref().map(interact_type_zh);
    let is_inn = npc.inn_id.is_some() || npc.inn.is_some();

    let mut lines = vec![format!(
        "NPC {name_display}（{map_label}，unique_id={}，map_unit_id={}，坐标 {pos_str}）",
        npc.unique_id,
        npc.map_unit_id
            .map_or_else(|| "无".to_owned(), |id| id.to_string())
    )];
    if let Some(itype) = interact_zh {
        lines.push(format!("- 交互类型：{itype}"));
    }
    if let Some(cond) = &npc.visible_condition {
        lines.push(format!("- 出现条件：{cond}"));
    }
    if is_inn {
        let price_str = npc
            .inn
            .as_ref()
            .map_or_else(|| "价格未知".to_owned(), |i| format!("{}G", i.price));
        lines.push(format!(
            "- 旅店：是（inn_id={}，住宿 {price_str}）",
            npc.inn_id
                .map_or_else(|| "未知".to_owned(), |id| id.to_string())
        ));
    } else {
        lines.push("- 旅店：否".to_owned());
    }

    let talk_events_json: Vec<Value> = npc
        .talk_events
        .iter()
        .map(|ev| {
            json!({
                "event_id": ev.id,
                "event_type": ev.event_type,
                "description": (!ev.description.is_empty()).then_some(&ev.description),
                "encount_group_id": ev.encount_group_id,
            })
        })
        .collect();
    if !npc.talk_events.is_empty() {
        let ids: Vec<&str> = npc.talk_events.iter().map(|e| e.id.as_str()).collect();
        lines.push(format!("- 对话事件：{}", ids.join("、")));
    }

    let mut shops_json = Vec::with_capacity(npc.shops.len());
    for resolved_shop in &npc.shops {
        let mut stock_json = Vec::with_capacity(resolved_shop.stock.len());
        let mut goods_labels = Vec::with_capacity(resolved_shop.stock.len());
        for slot in &resolved_shop.stock {
            let it_name = match &slot.item {
                Some(it) => resolve_db_zh_text_clean(db, &it.name),
                None => cached_item_name_zh(db, slot.entry.item_id, item_names)?,
            };
            let price = slot.price.or_else(|| {
                slot.item
                    .as_ref()
                    .map(|it| slot.entry.effective_price(it.buying_price))
            });
            let display = match &it_name {
                ZhText::Translated(n) => n.clone(),
                ZhText::Untranslated { ja } => {
                    format!(
                        "道具 #{}{}",
                        slot.entry.item_id,
                        ZhText::format_untranslated(ja)
                    )
                }
                ZhText::Missing => format!("道具 #{}", slot.entry.item_id),
            };
            let price_str = price.map_or_else(|| "价格未知".to_owned(), |p| format!("{p}G"));
            goods_labels.push(format!("{display}（#{}，{price_str}）", slot.entry.item_id));
            stock_json.push(with_zh_fields(
                json!({
                    "item_id": slot.entry.item_id,
                    "price": price,
                    "max_order_num": slot.entry.max_order_num,
                    "price_ratio": slot.entry.price_ratio,
                    "condition": (!slot.entry.condition.is_empty()).then_some(&slot.entry.condition),
                }),
                &[("item_name", &it_name)],
            ));
        }
        let shop_name = resolve_db_zh_text_clean(db, &resolved_shop.shop.name);
        let shop_label = format!(
            "商店 #{}{}",
            resolved_shop.shop.id,
            shop_name.bracketed_or_empty()
        );
        if goods_labels.is_empty() {
            lines.push(format!("- {shop_label}：无商品"));
        } else {
            lines.push(format!(
                "- {shop_label}（{} 件）：{}",
                goods_labels.len(),
                goods_labels.join("、")
            ));
        }
        shops_json.push(with_zh_fields(
            json!({
                "shop_id": resolved_shop.shop.id,
                "items": stock_json,
            }),
            &[("shop_name", &shop_name)],
        ));
    }

    let payload = with_zh_fields(
        json!({
            "map_id": npc.map_id,
            "unique_id": npc.unique_id,
            "level_name": npc.level_name,
            "actor_name": npc.actor_name,
            "location": npc.location.map(vec3_json),
            "direction": npc.direction,
            "interact_type": npc.interact_type,
            "interact_type_zh": interact_zh,
            "visible_condition": npc.visible_condition,
            "map_unit_id": npc.map_unit_id,
            "name": npc.name,
            "talk_events": talk_events_json,
            "various_shop_id": npc.various_shop_id,
            "shops": shops_json,
            "is_inn": is_inn,
            "inn_id": npc.inn_id,
            "inn_price": npc.inn.as_ref().map(|i| i.price),
        }),
        &[("map_name", &map_name), ("name_zh", &npc_name_zh)],
    );
    Ok((lines.join("\n"), payload))
}

fn format_map_unit_npcs_output_db(
    db: &GameDb,
    map_unit_id: i64,
    map_unit: Option<&MapUnit>,
    mut occurrences: Vec<ffr_db::query::links::MapUnitOccurrence>,
    map_id_filter: Option<i64>,
) -> Result<ToolOutput, String> {
    if let Some(mid) = map_id_filter {
        occurrences.retain(|o| o.map_id == Some(mid));
    }

    let name_zh = resolve_db_map_unit_name_zh(
        db,
        map_unit.and_then(|u| u.name_zh.as_deref()),
        map_unit,
        Some(map_unit_id),
    );
    let name_raw = map_unit.map(|u| u.name.clone());
    let title = match (&name_zh, &name_raw) {
        (ZhText::Translated(zh), Some(raw)) => format!("「{zh}」（内部名：{raw}）"),
        (ZhText::Translated(zh), None) => format!("「{zh}」"),
        (ZhText::Untranslated { ja }, Some(raw)) => {
            format!("{}（内部名：{raw}）", ZhText::format_untranslated(ja))
        }
        (ZhText::Untranslated { ja }, None) => ZhText::format_untranslated(ja),
        (ZhText::Missing, Some(raw)) => format!("「{raw}」（无中文译名）"),
        (ZhText::Missing, None) => format!("地图单位 #{map_unit_id}"),
    };

    // If there is exactly 1 placement with valid (map_id, unique_id), return the full rich NPC view.
    if occurrences.len() == 1
        && let (Some(mid), Some(uid)) = (occurrences[0].map_id, occurrences[0].unique_id)
        && let Some(npc) = db
            .npc(mid, uid)
            .map_err(|err| format!("查询 NPC ({mid}, {uid}) 失败：{err}"))?
    {
        return format_single_npc_output_db(db, &npc);
    }

    let mut item_names: HashMap<i64, ZhText> = HashMap::new();
    let mut map_names: HashMap<i64, ZhText> = HashMap::new();
    let mut lines = vec![format!(
        "地图单位 #{map_unit_id} {title}：共 {} 处出现位置{}",
        occurrences.len(),
        map_id_filter.map_or_else(String::new, |mid| format!("（已限定地图 #{mid}）"))
    )];
    let mut placements_json = Vec::with_capacity(occurrences.len());

    for (idx, occ) in occurrences.iter().enumerate() {
        let detailed = match (occ.map_id, occ.unique_id) {
            (Some(mid), Some(uid)) => db
                .npc(mid, uid)
                .map_err(|err| format!("查询 NPC ({mid}, {uid}) 详情失败：{err}"))?,
            _ => None,
        };

        let occ_map_name = match occ.map_id {
            Some(mid) => {
                resolve_opt_map_name_zh(db, mid, occ.map_name_zh.as_deref(), &mut map_names)?
            }
            None => ZhText::from_parts_clean(occ.map_name_zh.as_deref(), None),
        };
        let map_label = match occ.map_id {
            Some(mid) => format_map_ref_zh(mid, &occ_map_name),
            None => format!("关卡 {}", occ.level_name),
        };
        let pos_str = occ
            .location
            .map_or_else(|| "坐标未知".to_owned(), format_vec3_zh);
        let uid_str = occ
            .unique_id
            .map_or_else(|| "?".to_owned(), |id| id.to_string());

        let (talk_events_json, shops_json, is_inn, inn_id, inn_price, various_shop_id) =
            if let Some(ref npc) = detailed {
                let (_, val) = render_npc_detail_db(db, npc, &mut item_names, &mut map_names)?;
                (
                    val["talk_events"].clone(),
                    val["shops"].clone(),
                    val["is_inn"].as_bool().unwrap_or(false),
                    val["inn_id"].clone(),
                    val["inn_price"].clone(),
                    val["various_shop_id"].clone(),
                )
            } else {
                (
                    json!([]),
                    json!([]),
                    occ.inn_id.is_some(),
                    json!(occ.inn_id),
                    Value::Null,
                    Value::Null,
                )
            };

        if idx < 20 {
            let mut tags = Vec::new();
            if let Some(itype) = occ.interact_type.as_deref() {
                tags.push(interact_type_zh(itype).to_owned());
            }
            if is_inn {
                tags.push(match inn_price.as_i64() {
                    Some(p) => format!("旅店({p}G)"),
                    None => "旅店".to_owned(),
                });
            }
            if let Some(shop_arr) = shops_json.as_array()
                && !shop_arr.is_empty()
            {
                let sids: Vec<String> = shop_arr
                    .iter()
                    .filter_map(|s| s["shop_id"].as_i64().map(|id| format!("#{id}")))
                    .collect();
                tags.push(format!("商店 {}", sids.join("/")));
            }
            if let Some(ev_arr) = talk_events_json.as_array()
                && !ev_arr.is_empty()
            {
                tags.push(format!("对话 {} 条", ev_arr.len()));
            }
            if let Some(cond) = &occ.visible_condition {
                tags.push(format!("条件：{cond}"));
            }
            let tag_str = if tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", tags.join("，"))
            };
            lines.push(format!(
                "- {map_label} unique_id={uid_str} 坐标 {pos_str}{tag_str}"
            ));
        }

        placements_json.push(with_zh_fields(
            json!({
                "map_id": occ.map_id,
                "unique_id": occ.unique_id,
                "level_name": occ.level_name,
                "actor_name": occ.actor_name,
                "unit_kind": occ.unit_kind,
                "interact_type": occ.interact_type,
                "interact_type_zh": occ.interact_type.as_deref().map(interact_type_zh),
                "visible_condition": occ.visible_condition,
                "location": occ.location.map(vec3_json),
                "talk_events": talk_events_json,
                "various_shop_id": various_shop_id,
                "shops": shops_json,
                "is_inn": is_inn,
                "inn_id": inn_id,
                "inn_price": inn_price,
            }),
            &[("map_name", &occ_map_name)],
        ));
    }
    if occurrences.len() > 20 {
        lines.push(format!(
            "…其余 {} 处省略，完整列表见 JSON。",
            occurrences.len() - 20
        ));
    }

    Ok(tool_ok(
        lines.join("\n"),
        with_zh_fields(
            json!({
                "map_unit_id": map_unit_id,
                "name": name_raw,
                "map_id_filter": map_id_filter,
                "placement_count": placements_json.len(),
                "placements": placements_json,
            }),
            &[("name_zh", &name_zh)],
        ),
    ))
}

// ---- 4. read_talk ----

pub(crate) fn handle_read_talk_db(
    cache: &DbDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("read_talk", args, &["event_id"])?;
    let event_id = required_non_empty_str("read_talk", args, "event_id")?;

    let db = cache.open()?;
    let Some(ev) = db
        .talk_event(event_id)
        .map_err(|err| format!("查询对话事件「{event_id}」失败：{err}"))?
    else {
        return Err(format!(
            "参数 `event_id` 在对话事件表中找不到编号为「{event_id}」的对话事件"
        ));
    };

    let talk_lines = db
        .talk_lines(event_id, MODEL_LANGUAGE)
        .map_err(|err| format!("查询对话事件「{event_id}」台词失败：{err}"))?;
    let next_entries = db
        .talk_event_next(event_id)
        .map_err(|err| format!("查询对话事件「{event_id}」后续事件失败：{err}"))?;
    let sequence = db
        .sequence_of_talk_event(event_id)
        .map_err(|err| format!("查询对话事件「{event_id}」关联过场失败：{err}"))?;

    let battle_group_json = match ev.encount_group_id {
        Some(gid) => {
            let detail = db
                .battle_group(gid)
                .map_err(|err| format!("查询对话触发的战斗组 #{gid} 失败：{err}"))?;
            match detail {
                Some(d) => {
                    let members: Vec<Value> = d
                        .members
                        .iter()
                        .map(|m| {
                            let uname = m
                                .unit
                                .as_ref()
                                .map(|u| resolve_db_zh_text_clean(&db, &u.name))
                                .unwrap_or_default();
                            with_zh_fields(
                                json!({
                                    "unit_id": m.unit_id,
                                    "level": m.unit.as_ref().map(|u| u.level),
                                    "is_boss": m.unit.as_ref().map(|u| u.is_boss),
                                }),
                                &[("name", &uname)],
                            )
                        })
                        .collect();
                    Some(json!({
                        "group_id": d.group.id,
                        "is_boss": d.group.is_boss.unwrap_or_else(|| d.members.iter().any(|m| m.unit.as_ref().is_some_and(|u| u.is_boss))),
                        "can_escape": d.group.can_escape,
                        "is_bounty": d.group.is_bounty,
                        "ap": d.group.ap,
                        "members": members,
                    }))
                }
                None => Some(json!({ "group_id": gid, "unresolved": true })),
            }
        }
        None => None,
    };

    let sequence_json = sequence.as_ref().map(|seq| {
        let can_skip = !seq.has_skip_ban_param;
        json!({
            "path": seq.path,
            "name": seq.name,
            "duration_secs": seq.duration_secs,
            "can_skip": can_skip,
            "is_skip_confirm": ev.is_skip_confirm,
            "has_skip_markers": seq.has_skip_markers,
            "has_skip_ban_param": seq.has_skip_ban_param,
            "binding_count": seq.binding_count,
            "event_count": seq.event_count,
        })
    });

    let mut summary = vec![format!(
        "对话事件「{}」（类型：{}，台词 {} 句）",
        ev.id,
        ev.event_type,
        talk_lines.len()
    )];
    if !ev.description.is_empty() {
        summary.push(format!("- 说明：{}", ev.description));
    }

    if let Some(seq) = &sequence {
        let skip_desc = if seq.has_skip_ban_param {
            "不可跳过（导演蓝图含禁跳参数）"
        } else if ev.is_skip_confirm {
            "可跳过（弹出跳过确认）"
        } else if seq.has_skip_markers {
            "可跳过（含跳过标记）"
        } else {
            "可跳过（无禁跳标记）"
        };
        summary.push(format!(
            "- 关联过场：{}（时长 {:.1} 秒，{skip_desc}）",
            seq.name, seq.duration_secs
        ));
    } else if let Some(seq_path) = &ev.event_sequence_path {
        let skip_desc = if ev.is_skip_confirm {
            "弹跳过确认"
        } else {
            "跳过设置见事件配置"
        };
        summary.push(format!(
            "- 关联过场路径：{seq_path}（过场表未收录，{skip_desc}）"
        ));
    }

    if let Some(bg) = &battle_group_json {
        let gid = bg["group_id"].as_i64().unwrap_or(-1);
        if let Some(members) = bg["members"].as_array() {
            let member_labels: Vec<String> = members
                .iter()
                .map(|m| {
                    let uid = m["unit_id"].as_i64().unwrap_or(-1);
                    if let Some(n) = m["name"].as_str() {
                        format!("{n}(#{uid})")
                    } else if m["name_untranslated"] == Value::Bool(true)
                        && let Some(ja) = m["name_ja"].as_str()
                    {
                        format!("#{uid}{}", ZhText::format_untranslated(ja))
                    } else {
                        format!("#{uid}")
                    }
                })
                .collect();
            let boss_tag = if bg["is_boss"] == Value::Bool(true) {
                "Boss 战"
            } else {
                "普通战"
            };
            summary.push(format!(
                "- 触发战斗组：#{gid}（{boss_tag}，成员：{}）",
                member_labels.join("、")
            ));
        } else {
            summary.push(format!("- 触发战斗组：#{gid}（战斗组表未收录）"));
        }
    }

    let continuous_json: Vec<Value> = next_entries
        .iter()
        .filter(|n| n.kind == "continuous")
        .map(|n| {
            json!({
                "event_id": n.next_event,
                "condition": (!n.condition.is_empty()).then_some(&n.condition),
                "ord": n.ord,
            })
        })
        .collect();
    let choice_json: Vec<Value> = next_entries
        .iter()
        .filter(|n| n.kind == "choice")
        .map(|n| {
            json!({
                "choice_index": n.choice_index,
                "event_id": n.next_event,
                "condition": (!n.condition.is_empty()).then_some(&n.condition),
                "ord": n.ord,
            })
        })
        .collect();

    if !continuous_json.is_empty() {
        let parts: Vec<String> = next_entries
            .iter()
            .filter(|n| n.kind == "continuous")
            .map(|n| {
                if n.condition.is_empty() {
                    n.next_event.clone()
                } else {
                    format!("{} [条件: {}]", n.next_event, n.condition)
                }
            })
            .collect();
        summary.push(format!("- 后续事件：{}", parts.join(" -> ")));
    }
    if !choice_json.is_empty() {
        let parts: Vec<String> = next_entries
            .iter()
            .filter(|n| n.kind == "choice")
            .map(|n| {
                let idx_str = n
                    .choice_index
                    .map_or_else(|| "?".to_owned(), |i| i.to_string());
                if n.condition.is_empty() {
                    format!("选项 {idx_str} -> {}", n.next_event)
                } else {
                    format!("选项 {idx_str} -> {} [条件: {}]", n.next_event, n.condition)
                }
            })
            .collect();
        summary.push(format!("- 选项分支：{}", parts.join("；")));
    }

    let mut lines_json = Vec::with_capacity(talk_lines.len());
    if !talk_lines.is_empty() {
        summary.push("- 台词：".to_owned());
    }
    for (idx, line) in talk_lines.iter().enumerate() {
        let line_text = resolve_db_text_key_clean(
            &db,
            line.text.as_deref(),
            line.text_ns.as_deref(),
            line.text_key.as_deref(),
        );
        let text_display = line_text.inline_or("（无中文译文）");
        let speaker_display =
            if line.speaker.is_empty() || line.speaker.eq_ignore_ascii_case("None") {
                "旁白"
            } else {
                line.speaker.as_str()
            };
        summary.push(format!("  {}. {speaker_display}：{text_display}", idx + 1));
        lines_json.push(with_zh_fields(
            json!({
                "ord": line.ord,
                "row_key": line.row_key,
                "talk_id": line.talk_id,
                "speaker": line.speaker,
                "emotion": line.emotion,
                "voice": (!line.voice.is_empty()).then_some(&line.voice),
            }),
            &[("text", &line_text)],
        ));
    }

    Ok(tool_ok(
        summary.join("\n"),
        json!({
            "event_id": ev.id,
            "event_type": ev.event_type,
            "category": ev.category,
            "description": (!ev.description.is_empty()).then_some(&ev.description),
            "talk_table_path": ev.talk_table_path,
            "talk_id_arg": ev.talk_id_arg(),
            "event_sequence_path": ev.event_sequence_path,
            "is_skip_confirm": ev.is_skip_confirm,
            "line_count": lines_json.len(),
            "lines": lines_json,
            "continuous_events": continuous_json,
            "choice_branches": choice_json,
            "sequence": sequence_json,
            "encount_group_id": ev.encount_group_id,
            "battle_group": battle_group_json,
        }),
    ))
}

// ---- 5. whats_near ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum NearKind {
    Npc,
    Chest,
    Exit,
    SavePoint,
    Event,
    EncounterZone,
}

impl NearKind {
    const ALL: [Self; 6] = [
        Self::Npc,
        Self::Chest,
        Self::Exit,
        Self::SavePoint,
        Self::Event,
        Self::EncounterZone,
    ];

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "npc" => Some(Self::Npc),
            "chest" => Some(Self::Chest),
            "exit" => Some(Self::Exit),
            "save_point" => Some(Self::SavePoint),
            "event" => Some(Self::Event),
            "encounter_zone" => Some(Self::EncounterZone),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Npc => "npc",
            Self::Chest => "chest",
            Self::Exit => "exit",
            Self::SavePoint => "save_point",
            Self::Event => "event",
            Self::EncounterZone => "encounter_zone",
        }
    }

    fn label_zh(self) -> &'static str {
        match self {
            Self::Npc => "NPC",
            Self::Chest => "宝箱",
            Self::Exit => "出口",
            Self::SavePoint => "存档点",
            Self::Event => "事件触发",
            Self::EncounterZone => "遇敌区域",
        }
    }
}

pub(crate) async fn handle_whats_near_db(
    channel: &FileChannel,
    cache: &DbDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys(
        "whats_near",
        args,
        &["map_id", "x", "y", "z", "radius", "kinds"],
    )?;
    let explicit_map_id = optional_i64("whats_near", args, "map_id")?;
    let explicit_x = optional_finite_f64("whats_near", args, "x")?;
    let explicit_y = optional_finite_f64("whats_near", args, "y")?;
    let explicit_z = optional_finite_f64("whats_near", args, "z")?;
    let radius = match optional_finite_f64("whats_near", args, "radius")? {
        Some(r) if r <= 0.0 => {
            return Err("工具 `whats_near` 的参数 `radius` 必须大于 0".to_owned());
        }
        Some(r) => r,
        None => DEFAULT_NEAR_RADIUS,
    };
    let kinds = parse_near_kinds(args)?;

    let explicit_xy = match (explicit_x, explicit_y, explicit_z) {
        (Some(x), Some(y), z) => Some((x, y, z)),
        (None, None, None) => None,
        _ => {
            return Err(
                "工具 `whats_near` 显式指定坐标时必须同时提供 `x` 和 `y`（`z` 可选）".to_owned(),
            );
        }
    };

    let db = cache.open()?;

    let (map_id, origin_x, origin_y, origin_z, position_source) = if let (
        Some(mid),
        Some((x, y, z)),
    ) =
        (explicit_map_id, explicit_xy)
    {
        (mid, x, y, z, "explicit")
    } else {
        let snapshot = channel.read_fresh().await.map_err(|err| {
                format!(
                    "无法从文件桥读取当前位置（{err}）；可改用显式参数 `map_id` 与 `x`、`y`（可选 `z`）离线查询"
                )
            })?;
        let Some(root) = snapshot.data.as_object() else {
            return Err(
                    "文件桥返回的 state.json 中 data 不是对象，无法读取当前位置；请改用显式参数 `map_id`、`x`、`y` 查询"
                        .to_owned(),
                );
        };
        let field = obj_object(root, "field");
        let bridge_map_id = field
            .and_then(|f| obj_i64(f, "map_id"))
            .filter(|&id| id >= 0)
            .or_else(|| {
                obj_str(root, "map")
                    .and_then(|lvl| db.map_by_level(lvl).ok().flatten())
                    .map(|m| m.id)
            });
        let bridge_loc_obj = field
            .and_then(|f| obj_object(f, "player"))
            .and_then(|p| obj_object(p, "location"))
            .or_else(|| obj_object(root, "pawn_location"));
        let bridge_xyz = bridge_loc_obj.and_then(|loc| {
            let x = obj_f64(loc, "x")?;
            let y = obj_f64(loc, "y")?;
            let z = obj_f64(loc, "z")?;
            (x.is_finite() && y.is_finite() && z.is_finite()).then_some((x, y, Some(z)))
        });

        let resolved_map_id = explicit_map_id.or(bridge_map_id).ok_or_else(|| {
                "文件桥当前状态未提供有效地图编号（可能位于标题或载入画面）；请改用显式参数 `map_id` 与 `x`、`y`（可选 `z`）查询".to_owned()
            })?;
        let (rx, ry, rz) = explicit_xy.or(bridge_xyz).ok_or_else(|| {
                "文件桥当前状态未提供有效角色坐标（player.location / pawn_location 为空）；请改用显式参数 `map_id` 与 `x`、`y`（可选 `z`）查询".to_owned()
            })?;
        let src = if explicit_map_id.is_some() || explicit_xy.is_some() {
            "mixed"
        } else {
            "bridge"
        };
        (resolved_map_id, rx, ry, rz, src)
    };

    let map = db
        .map(map_id)
        .map_err(|err| format!("查询地图 {map_id} 失败：{err}"))?
        .ok_or_else(|| format!("参数 `map_id` 找不到编号为 {map_id} 的地图"))?;
    let map_name = resolve_db_zh_text_clean(&db, &map.name);

    let mut db_kinds: Vec<LevelObjectKind> = Vec::new();
    for &k in &kinds {
        match k {
            NearKind::Npc => {
                db_kinds.push(LevelObjectKind::MapUnit);
                db_kinds.push(LevelObjectKind::Shop);
                db_kinds.push(LevelObjectKind::Inn);
            }
            NearKind::Chest => db_kinds.push(LevelObjectKind::TreasureBox),
            NearKind::Exit => db_kinds.push(LevelObjectKind::Transition),
            NearKind::SavePoint => db_kinds.push(LevelObjectKind::SavePoint),
            NearKind::Event => db_kinds.push(LevelObjectKind::EventTrigger),
            NearKind::EncounterZone => db_kinds.push(LevelObjectKind::EncounterZone),
        }
    }

    let nearby = db
        .level_objects_near(
            map_id,
            origin_x,
            origin_y,
            origin_z,
            radius,
            Some(&db_kinds),
        )
        .map_err(|err| format!("空间查询地图 {map_id} 附近对象失败：{err}"))?;

    // Group Shop and Inn records by actor_id so they merge cleanly onto their NPC (`MapUnit`) entry.
    let mut shops_by_actor: HashMap<i64, Vec<ffr_data::assets::level::ShopPlacement>> =
        HashMap::new();
    let mut inns_by_actor: HashMap<i64, Vec<ffr_data::assets::level::InnPlacement>> =
        HashMap::new();
    let mut map_unit_actor_ids: HashSet<i64> = HashSet::new();
    for item in &nearby {
        match &item.object.data {
            LevelObjectData::MapUnit(_) => {
                map_unit_actor_ids.insert(item.actor_id);
            }
            LevelObjectData::Shop(s) => {
                shops_by_actor
                    .entry(item.actor_id)
                    .or_default()
                    .push(s.clone());
            }
            LevelObjectData::Inn(i) => {
                inns_by_actor
                    .entry(item.actor_id)
                    .or_default()
                    .push(i.clone());
            }
            _ => {}
        }
    }

    let mut map_names_cache: HashMap<i64, ZhText> = HashMap::new();
    let mut item_names_cache: HashMap<i64, ZhText> = HashMap::new();
    let mut map_units_cache: HashMap<i64, Option<MapUnit>> = HashMap::new();
    let mut shop_names_cache: HashMap<i64, ZhText> = HashMap::new();
    let mut inn_prices_cache: HashMap<i64, Option<i64>> = HashMap::new();

    struct NearEntry {
        distance: f64,
        kind: NearKind,
        unique_id: Option<i64>,
        summary_line: String,
        json: Value,
    }

    let mut entries: Vec<NearEntry> = Vec::new();
    let mut handled_standalone_actor: HashSet<i64> = HashSet::new();

    for hit in &nearby {
        let obj = &hit.object;
        let loc = obj.location;
        let is_here = hit.distance < NEAR_HERE_DISTANCE;
        let (dir_zh, dir_code, bearing_deg) = if is_here {
            (None, None, None)
        } else {
            match loc {
                Some(p) => compass_direction(origin_x, origin_y, p.x, p.y),
                None => (None, None, None),
            }
        };
        let pos_str = loc.map_or_else(|| "坐标未知".to_owned(), format_vec3_zh);
        let proximity_label = if is_here {
            "就在这里".to_owned()
        } else {
            let dist_display = format!("{:.0}", hit.distance);
            match dir_zh {
                Some(d) => format!("距离 {dist_display}（{d}）"),
                None => format!("距离 {dist_display}"),
            }
        };

        match &obj.data {
            LevelObjectData::MapUnit(mu) => {
                let mu_rec = match mu.map_unit_id {
                    Some(id) => cached_map_unit(&db, id, &mut map_units_cache)?,
                    None => None,
                };
                let actor_shops = shops_by_actor.get(&hit.actor_id);
                let actor_inns = inns_by_actor.get(&hit.actor_id);

                let shop_id = actor_shops
                    .and_then(|list| list.first())
                    .map(|s| s.shop_id)
                    .or(mu.shop_id);
                let is_various_shop = actor_shops
                    .and_then(|list| list.first())
                    .is_some_and(|s| s.is_various_shop);
                let shop_name = match shop_id {
                    Some(sid) => cached_shop_name_zh(&db, sid, &mut shop_names_cache)?,
                    None => ZhText::Missing,
                };
                let inn_id = actor_inns
                    .and_then(|list| list.first())
                    .map(|i| i.inn_id)
                    .or(mu.inn_id);
                let inn_price = match inn_id {
                    Some(iid) => cached_inn_price(&db, iid, &mut inn_prices_cache)?,
                    None => None,
                };

                let npc_name_zh = resolve_db_map_unit_name_zh(
                    &db,
                    mu_rec.as_ref().and_then(|r| r.name_zh.as_deref()),
                    mu_rec.as_ref(),
                    mu.map_unit_id,
                );
                let npc_name_raw = mu_rec.as_ref().map(|r| r.name.clone());
                let interact_type = mu
                    .interact_type
                    .clone()
                    .or_else(|| actor_shops.and_then(|l| l.first()?.interact_type.clone()))
                    .or_else(|| actor_inns.and_then(|l| l.first()?.interact_type.clone()));
                let interact_zh = interact_type.as_deref().map(interact_type_zh);

                let name_label = format_npc_short_label(
                    obj.unique_id,
                    &npc_name_zh,
                    npc_name_raw.as_deref(),
                    interact_type.as_deref(),
                );
                let mut extras = Vec::new();
                if let Some(sid) = shop_id {
                    if is_various_shop {
                        extras.push(format!("综合商店 #{sid}"));
                    } else {
                        extras.push(format!("商店 #{sid}"));
                    }
                }
                if let Some(iid) = inn_id {
                    match inn_price {
                        Some(p) => extras.push(format!("旅店 #{iid}({p}G)")),
                        None => extras.push(format!("旅店 #{iid}")),
                    }
                }
                if let Some(cond) = &obj.visible_condition {
                    extras.push(format!("条件：{cond}"));
                }
                let extra_str = if extras.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", extras.join("，"))
                };

                entries.push(NearEntry {
                    distance: hit.distance,
                    kind: NearKind::Npc,
                    unique_id: obj.unique_id,
                    summary_line: format!(
                        "- [NPC] {proximity_label}：{name_label}{extra_str}，坐标 {pos_str}"
                    ),
                    json: with_zh_fields(
                        json!({
                            "kind": NearKind::Npc.as_str(),
                            "kind_zh": NearKind::Npc.label_zh(),
                            "distance": hit.distance,
                            "direction_zh": dir_zh,
                            "direction": dir_code,
                            "bearing_deg": bearing_deg,
                            "unique_id": obj.unique_id,
                            "map_unit_id": mu.map_unit_id,
                            "name": npc_name_raw,
                            "unit_kind": mu.kind,
                            "interact_type": interact_type,
                            "interact_type_zh": interact_zh,
                            "shop_id": shop_id,
                            "is_various_shop": is_various_shop,
                            "inn_id": inn_id,
                            "inn_price": inn_price,
                            "visible_condition": obj.visible_condition,
                            "location": loc.map(vec3_json),
                        }),
                        &[("name_zh", &npc_name_zh), ("shop_name", &shop_name)],
                    ),
                });
            }
            LevelObjectData::Shop(sh) => {
                if map_unit_actor_ids.contains(&hit.actor_id)
                    || !handled_standalone_actor.insert(hit.actor_id)
                {
                    continue;
                }
                let shop_name = cached_shop_name_zh(&db, sh.shop_id, &mut shop_names_cache)?;
                let shop_label = format!("商店 #{}{}", sh.shop_id, shop_name.bracketed_or_empty());
                entries.push(NearEntry {
                    distance: hit.distance,
                    kind: NearKind::Npc,
                    unique_id: obj.unique_id,
                    summary_line: format!(
                        "- [NPC] {proximity_label}：{shop_label}（unique_id={}），坐标 {pos_str}",
                        obj.unique_id
                            .map_or_else(|| "?".to_owned(), |id| id.to_string())
                    ),
                    json: with_zh_fields(
                        json!({
                            "kind": NearKind::Npc.as_str(),
                            "kind_zh": NearKind::Npc.label_zh(),
                            "distance": hit.distance,
                            "direction_zh": dir_zh,
                            "direction": dir_code,
                            "bearing_deg": bearing_deg,
                            "unique_id": obj.unique_id,
                            "map_unit_id": sh.map_unit_id,
                            "shop_id": sh.shop_id,
                            "is_various_shop": sh.is_various_shop,
                            "interact_type": sh.interact_type,
                            "interact_type_zh": sh.interact_type.as_deref().map(interact_type_zh),
                            "visible_condition": obj.visible_condition,
                            "location": loc.map(vec3_json),
                        }),
                        &[("shop_name", &shop_name)],
                    ),
                });
            }
            LevelObjectData::Inn(inn) => {
                if map_unit_actor_ids.contains(&hit.actor_id)
                    || !handled_standalone_actor.insert(hit.actor_id)
                {
                    continue;
                }
                let inn_price = cached_inn_price(&db, inn.inn_id, &mut inn_prices_cache)?;
                entries.push(NearEntry {
                    distance: hit.distance,
                    kind: NearKind::Npc,
                    unique_id: obj.unique_id,
                    summary_line: format!(
                        "- [NPC] {proximity_label}：旅店 #{}（unique_id={}），坐标 {pos_str}",
                        inn.inn_id,
                        obj.unique_id
                            .map_or_else(|| "?".to_owned(), |id| id.to_string())
                    ),
                    json: json!({
                        "kind": NearKind::Npc.as_str(),
                        "kind_zh": NearKind::Npc.label_zh(),
                        "distance": hit.distance,
                        "direction_zh": dir_zh,
                        "direction": dir_code,
                        "bearing_deg": bearing_deg,
                        "unique_id": obj.unique_id,
                        "map_unit_id": inn.map_unit_id,
                        "inn_id": inn.inn_id,
                        "inn_price": inn_price,
                        "interact_type": inn.interact_type,
                        "interact_type_zh": inn.interact_type.as_deref().map(interact_type_zh),
                        "visible_condition": obj.visible_condition,
                        "location": loc.map(vec3_json),
                    }),
                });
            }
            LevelObjectData::TreasureBox(tb) => {
                let static_tb = tb
                    .chest_id
                    .and_then(|cid| db.treasure_box(map_id, cid).ok().flatten());
                let item_id = tb
                    .item_id
                    .or_else(|| static_tb.as_ref().and_then(|s| s.item_id));
                let item_num = tb
                    .item_num
                    .or_else(|| static_tb.as_ref().map(|s| s.item_num));
                let money = tb
                    .money
                    .or_else(|| static_tb.as_ref().and_then(|s| s.money));
                let is_trap = tb
                    .is_trap
                    .or_else(|| static_tb.as_ref().map(|s| s.is_trap))
                    .unwrap_or(false);
                let battle_group_id = tb
                    .battle_group_id
                    .or_else(|| static_tb.as_ref().and_then(|s| s.battle_group_id));
                let item_name = match item_id {
                    Some(iid) => cached_item_name_zh(&db, iid, &mut item_names_cache)?,
                    None => ZhText::Missing,
                };

                let mut content_parts = Vec::new();
                if let Some(iid) = item_id {
                    let base = match &item_name {
                        ZhText::Translated(n) => format!("{n}(#{iid})"),
                        ZhText::Untranslated { ja } => {
                            format!("道具 #{iid}{}", ZhText::format_untranslated(ja))
                        }
                        ZhText::Missing => format!("道具 #{iid}(#{iid})"),
                    };
                    match item_num {
                        Some(cnt) => content_parts.push(format!("{base}×{cnt}")),
                        None => content_parts.push(base),
                    }
                }
                if let Some(g) = money {
                    content_parts.push(format!("{g}G"));
                }
                if is_trap {
                    content_parts.push("陷阱箱".to_owned());
                }
                let content_str = if content_parts.is_empty() {
                    "内容未知".to_owned()
                } else {
                    content_parts.join("、")
                };
                let cid_tag = tb
                    .chest_id
                    .or(obj.unique_id)
                    .map_or_else(|| "?".to_owned(), |id| format!("#{id}"));

                entries.push(NearEntry {
                    distance: hit.distance,
                    kind: NearKind::Chest,
                    unique_id: obj.unique_id,
                    summary_line: format!(
                        "- [宝箱] {proximity_label}：宝箱 {cid_tag}（{content_str}），坐标 {pos_str}"
                    ),
                    json: with_zh_fields(
                        json!({
                            "kind": NearKind::Chest.as_str(),
                            "kind_zh": NearKind::Chest.label_zh(),
                            "distance": hit.distance,
                            "direction_zh": dir_zh,
                            "direction": dir_code,
                            "bearing_deg": bearing_deg,
                            "unique_id": obj.unique_id,
                            "chest_id": tb.chest_id,
                            "item_id": item_id,
                            "item_num": item_num,
                            "money": money,
                            "is_trap": is_trap,
                            "battle_group_id": battle_group_id,
                            "visible_condition": obj.visible_condition,
                            "location": loc.map(vec3_json),
                        }),
                        &[("item_name", &item_name)],
                    ),
                });
            }
            LevelObjectData::Transition(tr) => {
                let mut target_labels = Vec::new();
                let mut targets_json = Vec::with_capacity(tr.targets.len());
                for t in &tr.targets {
                    let tname = match t.target_map_id {
                        Some(tid) => cached_map_name_zh(&db, tid, &mut map_names_cache)?,
                        None => ZhText::Missing,
                    };
                    if let Some(tid) = t.target_map_id {
                        let lbl = format_map_ref_zh(tid, &tname);
                        if !target_labels.contains(&lbl) {
                            target_labels.push(lbl);
                        }
                    }
                    targets_json.push(with_zh_fields(
                        json!({
                            "target_map_id": t.target_map_id,
                            "target_point_id": t.target_point_id,
                            "is_event_only": t.is_event_only,
                            "condition": t.condition,
                        }),
                        &[("target_map_name", &tname)],
                    ));
                }
                let dest_desc = if tr.is_same_map {
                    "同图传送".to_owned()
                } else if target_labels.is_empty() {
                    "未指定目标地图".to_owned()
                } else {
                    format!("通往 {}", target_labels.join(" / "))
                };
                entries.push(NearEntry {
                    distance: hit.distance,
                    kind: NearKind::Exit,
                    unique_id: obj.unique_id,
                    summary_line: format!(
                        "- [出口] {proximity_label}：{dest_desc}，坐标 {pos_str}"
                    ),
                    json: json!({
                        "kind": NearKind::Exit.as_str(),
                        "kind_zh": NearKind::Exit.label_zh(),
                        "distance": hit.distance,
                        "direction_zh": dir_zh,
                        "direction": dir_code,
                        "bearing_deg": bearing_deg,
                        "unique_id": obj.unique_id,
                        "is_same_map": tr.is_same_map,
                        "is_auto_transition": tr.is_auto_transition,
                        "is_event_only": tr.is_event_only,
                        "condition": tr.condition,
                        "targets": targets_json,
                        "location": loc.map(vec3_json),
                    }),
                });
            }
            LevelObjectData::SavePoint(sp) => {
                let target_map_id = sp.target_map_id.or(sp.transition_map_id);
                let target_map_name = match target_map_id {
                    Some(tid) => cached_map_name_zh(&db, tid, &mut map_names_cache)?,
                    None => ZhText::Missing,
                };
                let tp_desc = match target_map_id {
                    Some(tid) => {
                        format!("（可传送至 {}）", format_map_ref_zh(tid, &target_map_name))
                    }
                    None if sp.is_teleport => "（可传送）".to_owned(),
                    None => String::new(),
                };
                entries.push(NearEntry {
                    distance: hit.distance,
                    kind: NearKind::SavePoint,
                    unique_id: obj.unique_id,
                    summary_line: format!(
                        "- [存档点] {proximity_label}：存档点{tp_desc}，坐标 {pos_str}"
                    ),
                    json: with_zh_fields(
                        json!({
                            "kind": NearKind::SavePoint.as_str(),
                            "kind_zh": NearKind::SavePoint.label_zh(),
                            "distance": hit.distance,
                            "direction_zh": dir_zh,
                            "direction": dir_code,
                            "bearing_deg": bearing_deg,
                            "unique_id": obj.unique_id,
                            "save_point_type": sp.save_point_type,
                            "is_teleport": sp.is_teleport,
                            "target_map_id": target_map_id,
                            "target_point_id": sp.target_point_id.or(sp.transition_point_id),
                            "visible_condition": obj.visible_condition,
                            "location": loc.map(vec3_json),
                        }),
                        &[("target_map_name", &target_map_name)],
                    ),
                });
            }
            LevelObjectData::EventTrigger(et) => {
                let mut ev_ids: Vec<String> = Vec::new();
                if let Some(id) = &et.interact_event_id {
                    ev_ids.push(id.clone());
                }
                for e in &et.events {
                    if !ev_ids.contains(&e.event_id) {
                        ev_ids.push(e.event_id.clone());
                    }
                }
                let ev_desc = if ev_ids.is_empty() {
                    et.actor_name.clone()
                } else {
                    ev_ids.join("、")
                };
                entries.push(NearEntry {
                    distance: hit.distance,
                    kind: NearKind::Event,
                    unique_id: obj.unique_id,
                    summary_line: format!(
                        "- [事件触发] {proximity_label}：{ev_desc}，坐标 {pos_str}"
                    ),
                    json: json!({
                        "kind": NearKind::Event.as_str(),
                        "kind_zh": NearKind::Event.label_zh(),
                        "distance": hit.distance,
                        "direction_zh": dir_zh,
                        "direction": dir_code,
                        "bearing_deg": bearing_deg,
                        "unique_id": obj.unique_id,
                        "actor_name": et.actor_name,
                        "interact_type": et.interact_type,
                        "interact_event_id": et.interact_event_id,
                        "events": et.events,
                        "visible_condition": obj.visible_condition,
                        "flag_condition": et.flag_condition,
                        "location": loc.map(vec3_json),
                    }),
                });
            }
            LevelObjectData::EncounterZone(ez) => {
                let table_desc = ez
                    .encount_datatable
                    .as_deref()
                    .or_else(|| {
                        ez.terrain_datatables
                            .first()
                            .and_then(|t| t.datatable.as_deref())
                    })
                    .unwrap_or("未指定遇敌表");
                entries.push(NearEntry {
                    distance: hit.distance,
                    kind: NearKind::EncounterZone,
                    unique_id: ez.id,
                    summary_line: format!(
                        "- [遇敌区域] {proximity_label}：{}（{table_desc}），坐标 {pos_str}",
                        ez.kind
                    ),
                    json: json!({
                        "kind": NearKind::EncounterZone.as_str(),
                        "kind_zh": NearKind::EncounterZone.label_zh(),
                        "distance": hit.distance,
                        "direction_zh": dir_zh,
                        "direction": dir_code,
                        "bearing_deg": bearing_deg,
                        "id": ez.id,
                        "zone_kind": ez.kind,
                        "priority": ez.priority,
                        "encount_datatable": ez.encount_datatable,
                        "terrain_datatables": ez.terrain_datatables,
                        "visible_condition": obj.visible_condition,
                        "flag_condition": ez.flag_condition,
                        "location": loc.map(vec3_json),
                    }),
                });
            }
            _ => {}
        }
    }

    entries.sort_by(|a, b| {
        a.distance
            .total_cmp(&b.distance)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.unique_id.cmp(&b.unique_id))
    });

    let origin_str = match origin_z {
        Some(zv) => format!("({origin_x:.1}, {origin_y:.1}, {zv:.1})"),
        None => format!("({origin_x:.1}, {origin_y:.1}) [2D]"),
    };
    let source_zh = match position_source {
        "bridge" => "来自文件桥实时位置",
        "mixed" => "部分来自文件桥",
        _ => "来自显式参数",
    };
    let map_label = format_map_ref_zh(map_id, &map_name);

    let mut summary_lines = vec![format!(
        "{map_label} 坐标 {origin_str}（{source_zh}）半径 {radius:.0} 内共找到 {} 个对象（按距离排序；方向按 +X 为北推算，未经真机核对）：",
        entries.len()
    )];
    if entries.is_empty() {
        summary_lines
            .push("该半径内没有匹配的 NPC、宝箱、出口、存档点、事件触发或遇敌区域。".to_owned());
    } else {
        for entry in entries.iter().take(30) {
            summary_lines.push(entry.summary_line.clone());
        }
        if entries.len() > 30 {
            summary_lines.push(format!(
                "…其余 {} 条省略，完整列表见 JSON。",
                entries.len() - 30
            ));
        }
    }

    let items_json: Vec<Value> = entries.into_iter().map(|e| e.json).collect();
    let kind_names: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
    let origin_json = match origin_z {
        Some(zv) => json!({ "x": origin_x, "y": origin_y, "z": zv }),
        None => json!({ "x": origin_x, "y": origin_y, "z": null }),
    };

    Ok(tool_ok(
        summary_lines.join("\n"),
        with_zh_fields(
            json!({
                "map_id": map_id,
                "position_source": position_source,
                "origin": origin_json,
                "radius": radius,
                "kinds": kind_names,
                "count": items_json.len(),
                "items": items_json,
            }),
            &[("map_name", &map_name)],
        ),
    ))
}

fn parse_near_kinds(args: &Map<String, Value>) -> Result<Vec<NearKind>, String> {
    let Some(raw) = args.get("kinds") else {
        return Ok(NearKind::ALL.to_vec());
    };
    if raw.is_null() {
        return Ok(NearKind::ALL.to_vec());
    }
    let Some(arr) = raw.as_array() else {
        return Err("工具 `whats_near` 的参数 `kinds` 必须是字符串数组".to_owned());
    };
    if arr.is_empty() {
        return Err("工具 `whats_near` 的参数 `kinds` 不能为空数组".to_owned());
    }
    let mut parsed = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(text) = item.as_str() else {
            return Err("工具 `whats_near` 的参数 `kinds` 只能包含字符串".to_owned());
        };
        let Some(kind) = NearKind::parse(text) else {
            return Err(format!(
                "工具 `whats_near` 不支持类别 `{text}`，可选值为：npc、chest、exit、save_point、event、encounter_zone"
            ));
        };
        if !parsed.contains(&kind) {
            parsed.push(kind);
        }
    }
    Ok(parsed)
}

// ---- 6. query_game_db ----

pub(crate) fn handle_query_game_db_db(
    cache: &DbDataCache,
    args: &Map<String, Value>,
) -> Result<ToolOutput, String> {
    reject_unknown_keys("query_game_db", args, &["sql", "limit"])?;
    let sql = required_non_empty_str("query_game_db", args, "sql")?;
    let limit = match optional_i64("query_game_db", args, "limit")? {
        Some(n) if !(1..=QUERY_DB_MAX_ROWS as i64).contains(&n) => {
            return Err(format!(
                "工具 `query_game_db` 的参数 `limit` 必须在 1..={QUERY_DB_MAX_ROWS} 范围内"
            ));
        }
        Some(n) => usize::try_from(n).unwrap_or(QUERY_DB_MAX_ROWS),
        None => QUERY_DB_MAX_ROWS,
    };

    let db = cache.open()?;
    let res = db
        .query_readonly_limited(sql, limit, QUERY_DB_TIMEOUT)
        .map_err(|err| match err {
            DbError::WriteQueryRejected { detail } => format!("拒绝执行 SQL：{detail}"),
            other if is_query_timeout_error(&other) => format!(
                "SQL 查询超时（超过 {} 秒，已被 SQLite progress handler 中断），请缩小查询范围或使用索引列；可读取资源 `ffr://db/schema` 查看表与列",
                QUERY_DB_TIMEOUT.as_secs()
            ),
            other => format!("执行只读 SQL 失败：{other}"),
        })?;

    let fetched_rows = res.rows.len();
    let row_budget = QUERY_DB_MAX_OUTPUT_BYTES.saturating_sub(QUERY_DB_WRAPPER_RESERVE_BYTES);

    let mut kept_rows: Vec<Value> = Vec::new();
    let mut used_bytes: usize = 2; // "[]"
    let mut truncated_by_size = false;
    let mut omitted_rows: usize = 0;
    let mut omitted_bytes: usize = 0;

    for row_map in res.rows {
        let row_val = Value::Object(row_map);
        let serialized = serde_json::to_string(&row_val).unwrap_or_default();
        let entry_bytes = serialized.len() + usize::from(!kept_rows.is_empty());

        if !truncated_by_size && used_bytes + entry_bytes <= row_budget {
            used_bytes += entry_bytes;
            kept_rows.push(row_val);
            continue;
        }

        if kept_rows.is_empty() {
            // Even the first row exceeds the ~20 KB budget: truncate huge string fields inside it.
            let (shrunk, dropped) = shrink_single_row(row_val, row_budget);
            truncated_by_size = true;
            omitted_bytes = omitted_bytes.saturating_add(dropped);
            let shrunk_len = serde_json::to_string(&shrunk).map_or(0, |s| s.len());
            used_bytes = used_bytes.saturating_add(shrunk_len);
            kept_rows.push(shrunk);
        } else {
            truncated_by_size = true;
            omitted_rows += 1;
            omitted_bytes = omitted_bytes.saturating_add(serialized.len());
        }
    }

    let returned_rows = kept_rows.len();
    let mut lines = vec![format!(
        "只读 SQL 执行完成（耗时 {} ms，列：{}，返回 {} 行{}）：",
        res.elapsed_ms,
        if res.columns.is_empty() {
            "无".to_owned()
        } else {
            res.columns.join("、")
        },
        returned_rows,
        if res.truncated_rows {
            format!("，结果超过行数上限 {limit} 行")
        } else {
            String::new()
        }
    )];

    if res.truncated_rows {
        lines.push(format!(
            "注意：查询命中超过 {limit} 行，已按上限截断，仅读取前 {fetched_rows} 行。"
        ));
    }
    if truncated_by_size {
        if omitted_rows > 0 {
            lines.push(format!(
                "注意：输出超过约 20 KB 上限，已截断：保留前 {returned_rows} 行，略去其余 {omitted_rows} 行（约 {omitted_bytes} 字节）。"
            ));
        } else {
            lines.push(format!(
                "注意：单行输出超过约 20 KB 上限，已截断超长字段（约略去 {omitted_bytes} 字节）。"
            ));
        }
    }

    for (idx, row) in kept_rows
        .iter()
        .take(QUERY_DB_SUMMARY_PREVIEW_ROWS)
        .enumerate()
    {
        let preview = serde_json::to_string(row).unwrap_or_default();
        let short = truncate_utf8_with_ellipsis(&preview, 240);
        lines.push(format!("  {}. {short}", idx + 1));
    }
    if returned_rows > QUERY_DB_SUMMARY_PREVIEW_ROWS {
        lines.push(format!(
            "  …其余 {} 行见 JSON。",
            returned_rows - QUERY_DB_SUMMARY_PREVIEW_ROWS
        ));
    }

    Ok(tool_ok(
        lines.join("\n"),
        json!({
            "columns": res.columns,
            "row_limit": limit,
            "fetched_rows": fetched_rows,
            "returned_rows": returned_rows,
            "truncated_by_row_limit": res.truncated_rows,
            "truncated_by_size": truncated_by_size,
            "omitted_rows": omitted_rows,
            "omitted_bytes": omitted_bytes,
            "elapsed_ms": res.elapsed_ms,
            "rows": kept_rows,
        }),
    ))
}

fn shrink_single_row(mut row: Value, budget: usize) -> (Value, usize) {
    let orig_len = serde_json::to_string(&row).map_or(0, |s| s.len());
    if orig_len <= budget {
        return (row, 0);
    }
    if let Some(obj) = row.as_object_mut() {
        let field_count = obj.len().max(1);
        let per_field_cap = (budget / field_count).clamp(64, 4096);
        for val in obj.values_mut() {
            if let Value::String(s) = val
                && s.len() > per_field_cap
            {
                let cut = floor_char_boundary(s, per_field_cap);
                let dropped = s.len() - cut;
                let prefix = &s[..cut];
                *s = format!("{prefix}…[已截断 {dropped} 字节]");
            }
        }
    }
    let new_len = serde_json::to_string(&row).map_or(0, |s| s.len());
    (row, orig_len.saturating_sub(new_len))
}

fn truncate_utf8_with_ellipsis(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let cut = floor_char_boundary(text, max_bytes);
    format!("{}…", &text[..cut])
}

fn floor_char_boundary(text: &str, max_bytes: usize) -> usize {
    let mut end = max_bytes.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

// ---- Shared formatting & lookup helpers ----

fn optional_finite_f64(
    tool: &str,
    args: &Map<String, Value>,
    key: &str,
) -> Result<Option<f64>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(num) = value.as_f64() else {
        return Err(format!("工具 `{tool}` 的参数 `{key}` 必须是数字"));
    };
    if !num.is_finite() {
        return Err(format!("工具 `{tool}` 的参数 `{key}` 必须是有限数字"));
    }
    Ok(Some(num))
}

/// Convert a 2D displacement `(target - origin)` into an 8-way Chinese compass direction,
/// English code, and bearing degree using [`COMPASS_NORTH_AZIMUTH_DEG`].
///
/// Returns `(None, None, None)` when the horizontal displacement is smaller than
/// [`NEAR_HERE_DISTANCE`].
pub(crate) fn compass_direction(
    origin_x: f64,
    origin_y: f64,
    target_x: f64,
    target_y: f64,
) -> (Option<&'static str>, Option<&'static str>, Option<f64>) {
    let dx = target_x - origin_x;
    let dy = target_y - origin_y;
    if dx.hypot(dy) < NEAR_HERE_DISTANCE {
        return (None, None, None);
    }
    let raw_deg = dy.atan2(dx).to_degrees();
    let deg = (raw_deg - COMPASS_NORTH_AZIMUTH_DEG + 180.0).rem_euclid(360.0) - 180.0;
    let rounded = (deg * 10.0).round() / 10.0;
    let (zh, code) = if (-22.5..22.5).contains(&deg) {
        ("北", "North")
    } else if (22.5..67.5).contains(&deg) {
        ("东北", "NorthEast")
    } else if (67.5..112.5).contains(&deg) {
        ("东", "East")
    } else if (112.5..157.5).contains(&deg) {
        ("东南", "SouthEast")
    } else if !(-157.5..157.5).contains(&deg) {
        ("南", "South")
    } else if (-157.5..-112.5).contains(&deg) {
        ("西南", "SouthWest")
    } else if (-112.5..-67.5).contains(&deg) {
        ("西", "West")
    } else {
        ("西北", "NorthWest")
    };
    (Some(zh), Some(code), Some(rounded))
}

pub(crate) fn map_edge_kind_zh(kind: MapEdgeKind) -> &'static str {
    match kind {
        MapEdgeKind::CrossMap => "出口换图",
        MapEdgeKind::SameMap => "同图传送",
        MapEdgeKind::SavePointTeleport => "存档点传送",
    }
}

pub(crate) fn map_direction_enum_zh(raw: &str) -> Option<&'static str> {
    match raw.rsplit_once("::").map_or(raw, |(_, s)| s) {
        "North" => Some("北"),
        "South" => Some("南"),
        "East" => Some("东"),
        "West" => Some("西"),
        "NorthEast" => Some("东北"),
        "SouthEast" => Some("东南"),
        "SouthWest" => Some("西南"),
        "NorthWest" => Some("西北"),
        _ => None,
    }
}

pub(crate) fn format_vec3_zh(v: Vec3) -> String {
    format!("({:.1}, {:.1}, {:.1})", v.x, v.y, v.z)
}

pub(crate) fn vec3_json(v: Vec3) -> Value {
    json!({ "x": v.x, "y": v.y, "z": v.z })
}

fn format_npc_short_label(
    unique_id: Option<i64>,
    name_zh: &ZhText,
    name_raw: Option<&str>,
    interact_type: Option<&str>,
) -> String {
    let uid_part = unique_id.map_or_else(String::new, |id| format!("unique_id={id}"));
    let role_part = interact_type.map(interact_type_zh);
    let base = match (name_zh, name_raw) {
        (ZhText::Translated(zh), _) => format!("NPC「{zh}」"),
        (ZhText::Untranslated { ja }, _) => format!("NPC{}", ZhText::format_untranslated(ja)),
        (ZhText::Missing, Some(raw)) => format!("NPC「{raw}」"),
        (ZhText::Missing, None) => "NPC".to_owned(),
    };
    let mut details = Vec::new();
    if !uid_part.is_empty() {
        details.push(uid_part);
    }
    if let Some(role) = role_part {
        details.push(role.to_owned());
    }
    if details.is_empty() {
        base
    } else {
        format!("{base}（{}）", details.join("，"))
    }
}

fn drop_kind_code(kind: ItemDropKind) -> &'static str {
    match kind {
        ItemDropKind::DropNormal => "drop_normal",
        ItemDropKind::DropRare => "drop_rare",
        ItemDropKind::StealNormal => "steal_normal",
        ItemDropKind::StealRare => "steal_rare",
    }
}

fn drop_kind_zh(kind: ItemDropKind) -> &'static str {
    match kind {
        ItemDropKind::DropNormal => "普通掉落",
        ItemDropKind::DropRare => "稀有掉落",
        ItemDropKind::StealNormal => "普通偷窃",
        ItemDropKind::StealRare => "稀有偷窃",
    }
}

fn cached_map_unit(
    db: &GameDb,
    map_unit_id: i64,
    cache: &mut HashMap<i64, Option<MapUnit>>,
) -> Result<Option<MapUnit>, String> {
    if let Some(hit) = cache.get(&map_unit_id) {
        return Ok(hit.clone());
    }
    let val = db
        .map_unit(map_unit_id)
        .map_err(|err| format!("查询地图单位 {map_unit_id} 失败：{err}"))?;
    cache.insert(map_unit_id, val.clone());
    Ok(val)
}

fn cached_shop_name_zh(
    db: &GameDb,
    shop_id: i64,
    cache: &mut HashMap<i64, ZhText>,
) -> Result<ZhText, String> {
    if let Some(hit) = cache.get(&shop_id) {
        return Ok(hit.clone());
    }
    let val = db
        .shop(shop_id)
        .map_err(|err| format!("查询商店 {shop_id} 失败：{err}"))?
        .map(|s| resolve_db_zh_text_clean(db, &s.shop.name))
        .unwrap_or_default();
    cache.insert(shop_id, val.clone());
    Ok(val)
}

fn resolve_opt_map_name_zh(
    db: &GameDb,
    map_id: i64,
    prejoined_zh: Option<&str>,
    cache: &mut HashMap<i64, ZhText>,
) -> Result<ZhText, String> {
    if let Some(clean) = prejoined_zh.and_then(collapse_ws) {
        return Ok(ZhText::Translated(clean));
    }
    cached_map_name_zh(db, map_id, cache)
}

fn resolve_opt_shop_name_zh(
    db: &GameDb,
    shop_id: i64,
    prejoined_zh: Option<&str>,
    cache: &mut HashMap<i64, ZhText>,
) -> Result<ZhText, String> {
    if let Some(clean) = prejoined_zh.and_then(collapse_ws) {
        return Ok(ZhText::Translated(clean));
    }
    cached_shop_name_zh(db, shop_id, cache)
}

fn resolve_opt_unit_name_zh(db: &GameDb, unit_id: i64, prejoined_zh: Option<&str>) -> ZhText {
    if let Some(clean) = prejoined_zh.and_then(collapse_ws) {
        return ZhText::Translated(clean);
    }
    if let Ok(fields) = db.unit_untranslated_fields(unit_id)
        && let Some(hit) = fields
            .into_iter()
            .find(|f| f.field == "name" && f.lang == MODEL_LANGUAGE)
        && let Some(clean_ja) = collapse_ws(&hit.value)
    {
        return ZhText::Untranslated { ja: clean_ja };
    }
    ZhText::Missing
}

fn resolve_opt_quest_name_zh(db: &GameDb, quest_id: i64, prejoined_zh: Option<&str>) -> ZhText {
    if let Some(clean) = prejoined_zh.and_then(collapse_ws) {
        return ZhText::Translated(clean);
    }
    if let Ok(fields) = db.quest_untranslated_fields(quest_id)
        && let Some(hit) = fields
            .into_iter()
            .find(|f| f.field == "title" && f.lang == MODEL_LANGUAGE)
        && let Some(clean_ja) = collapse_ws(&hit.value)
    {
        return ZhText::Untranslated { ja: clean_ja };
    }
    ZhText::Missing
}

fn resolve_opt_quest_board_name_zh(
    db: &GameDb,
    quest_id: i64,
    prejoined_zh: Option<&str>,
) -> ZhText {
    if let Some(clean) = prejoined_zh.and_then(collapse_ws) {
        return ZhText::Translated(clean);
    }
    if let Ok(Some(qb)) = db.quest_board(quest_id) {
        return resolve_db_zh_text_clean(db, &qb.title);
    }
    ZhText::Missing
}

fn cached_inn_price(
    db: &GameDb,
    inn_id: i64,
    cache: &mut HashMap<i64, Option<i64>>,
) -> Result<Option<i64>, String> {
    if let Some(hit) = cache.get(&inn_id) {
        return Ok(*hit);
    }
    let price = match db
        .inn(inn_id)
        .map_err(|err| format!("查询旅店 {inn_id} 失败：{err}"))?
    {
        Some(inn) => Some(inn.price),
        None => db
            .inn(-1)
            .map_err(|err| format!("查询默认旅店失败：{err}"))?
            .map(|inn| inn.price),
    };
    cache.insert(inn_id, price);
    Ok(price)
}

/// Group NPC placements on a map by `unique_id` for `read_map` enrichment.
pub(crate) fn collect_map_npcs_for_read_map(
    db: &GameDb,
    map_id: i64,
) -> Result<Vec<Value>, String> {
    let map_units = db
        .map_objects(map_id, Some(LevelObjectKind::MapUnit))
        .map_err(|err| format!("查询地图 {map_id} 的 NPC 失败：{err}"))?;
    let shops = db
        .map_objects(map_id, Some(LevelObjectKind::Shop))
        .map_err(|err| format!("查询地图 {map_id} 的商店对象失败：{err}"))?;
    let inns = db
        .map_objects(map_id, Some(LevelObjectKind::Inn))
        .map_err(|err| format!("查询地图 {map_id} 的旅店对象失败：{err}"))?;

    let mut shop_by_uid: BTreeMap<i64, (i64, bool)> = BTreeMap::new();
    for s in &shops {
        if let (Some(uid), LevelObjectData::Shop(sp)) = (s.unique_id, &s.data) {
            shop_by_uid
                .entry(uid)
                .or_insert((sp.shop_id, sp.is_various_shop));
        }
    }
    let mut inn_by_uid: BTreeMap<i64, i64> = BTreeMap::new();
    for i in &inns {
        if let (Some(uid), LevelObjectData::Inn(ip)) = (i.unique_id, &i.data) {
            inn_by_uid.entry(uid).or_insert(ip.inn_id);
        }
    }

    let mut mu_cache: HashMap<i64, Option<MapUnit>> = HashMap::new();
    let mut out = Vec::with_capacity(map_units.len());
    for obj in &map_units {
        let LevelObjectData::MapUnit(mu) = &obj.data else {
            continue;
        };
        let mu_rec = match mu.map_unit_id {
            Some(id) => cached_map_unit(db, id, &mut mu_cache)?,
            None => None,
        };
        let npc_name_zh = resolve_db_map_unit_name_zh(
            db,
            mu_rec.as_ref().and_then(|r| r.name_zh.as_deref()),
            mu_rec.as_ref(),
            mu.map_unit_id,
        );
        let (shop_id, is_various_shop) = obj
            .unique_id
            .and_then(|uid| shop_by_uid.get(&uid).copied())
            .map_or((mu.shop_id, false), |(sid, v)| (Some(sid), v));
        let inn_id = obj
            .unique_id
            .and_then(|uid| inn_by_uid.get(&uid).copied())
            .or(mu.inn_id);
        out.push(with_zh_fields(
            json!({
                "unique_id": obj.unique_id,
                "map_unit_id": mu.map_unit_id,
                "name": mu_rec.as_ref().map(|r| &r.name),
                "unit_kind": mu.kind,
                "interact_type": mu.interact_type,
                "interact_type_zh": mu.interact_type.as_deref().map(interact_type_zh),
                "shop_id": shop_id,
                "is_various_shop": is_various_shop,
                "inn_id": inn_id,
                "visible_condition": obj.visible_condition,
                "location": obj.location.map(vec3_json),
            }),
            &[("name_zh", &npc_name_zh)],
        ));
    }
    Ok(out)
}
