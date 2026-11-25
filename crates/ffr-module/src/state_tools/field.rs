//! Field topic formatter for `read_game_state`.

use serde_json::{Map, Value, json};

use crate::state_tools::{
    LookupCtx, SectionOutput, extract_i64_list, format_item_label, format_location,
    format_map_label, interact_type_zh, obj_array, obj_bool, obj_f64, obj_i64, obj_object, obj_str,
};

/// Render the `field` section (Chinese summary + structured JSON) from `data.field` and v0 fields.
pub(crate) fn render_field(
    root: &Map<String, Value>,
    field: Option<&Map<String, Value>>,
    lookup: &LookupCtx,
) -> SectionOutput {
    let Some(field) = field else {
        return render_v0_fallback(root, lookup);
    };

    let top_map = obj_str(root, "map");
    let raw_map_id = obj_i64(field, "map_id");
    let (map_id, map_name) = resolve_field_map(raw_map_id, top_map, lookup);
    let area_id = obj_i64(field, "area_id");

    let is_loading = obj_bool(field, "is_loading");
    let in_battle = obj_bool(field, "in_battle");
    let in_event = obj_bool(field, "in_event");
    let in_dialog = obj_bool(field, "in_dialog");
    let in_menu = obj_bool(field, "in_menu");
    let can_control = obj_bool(field, "can_control");
    let camera_deg = obj_f64(field, "camera_deg");
    let camera_internal_rotation = obj_bool(field, "camera_internal_rotation");

    let player_obj = obj_object(field, "player");
    let player_loc = player_obj
        .and_then(|p| obj_object(p, "location"))
        .or_else(|| obj_object(root, "pawn_location"));
    let loc_pair = format_location(player_loc);
    let player_visible = player_obj.and_then(|p| obj_bool(p, "visible"));
    let player_unit_id = player_obj.and_then(|p| obj_i64(p, "unit_id"));
    let player_character =
        player_unit_id.and_then(|uid| lookup.playable_character_by_map_unit_id(uid));
    let player_character_unit_id = player_character.as_ref().map(|(unit_id, _)| *unit_id);
    let player_character_name = player_character.and_then(|(_, name)| name);
    let player_direction = player_obj.and_then(|p| obj_str(p, "direction"));
    let player_direction_zh = player_direction.map(direction_zh);
    let player_direction_deg = player_obj.and_then(|p| obj_f64(p, "direction_deg"));
    let player_movement = player_obj.and_then(|p| obj_str(p, "movement_method"));
    let player_movement_zh = player_movement.map(movement_method_zh);
    let player_speed = player_obj.and_then(|p| obj_f64(p, "speed_2d"));

    let interact_obj = obj_object(field, "interact_target");
    let interact_formatted = interact_obj.map(|target| format_interact_target(target, lookup));

    let chests_raw = obj_array(field, "chests").unwrap_or(&[]);
    let mut chests_json = Vec::with_capacity(chests_raw.len());
    let mut chest_labels = Vec::new();
    let mut opened_count = 0_usize;
    let mut unopened_count = 0_usize;
    for raw in chests_raw {
        let Some(chest) = raw.as_object() else {
            continue;
        };
        let unique_id = obj_i64(chest, "unique_id");
        let chest_id = obj_i64(chest, "chest_id");
        let is_open = obj_bool(chest, "is_open");
        match is_open {
            Some(true) => opened_count += 1,
            Some(false) => unopened_count += 1,
            None => {}
        }
        let item_id = obj_i64(chest, "item_id").filter(|&id| id >= 0);
        let item_num = obj_i64(chest, "item_num").filter(|&n| n > 0);
        let money = obj_i64(chest, "money").filter(|&m| m > 0);
        let item_name = item_id.and_then(|id| lookup.item_name(id));
        let static_box = map_id
            .zip(chest_id)
            .and_then(|(mid, cid)| lookup.treasure_box(mid, cid));
        let is_trap = static_box.as_ref().map(|tb| tb.is_trap);
        let battle_group_id = static_box.as_ref().and_then(|tb| tb.battle_group_id);
        let chest_loc = format_location(obj_object(chest, "location"));

        let status = match is_open {
            Some(true) => "已开",
            Some(false) => "未开",
            None => "状态未知",
        };
        let mut content_parts = Vec::new();
        if let Some(id) = item_id {
            let label = format_item_label(id, item_name.as_deref());
            match item_num {
                Some(n) => content_parts.push(format!("{label}×{n}")),
                None => content_parts.push(label),
            }
        }
        if let Some(g) = money {
            content_parts.push(format!("{g}G"));
        }
        if is_trap == Some(true) {
            content_parts.push("陷阱箱".to_owned());
        }
        let id_tag = chest_id
            .or(unique_id)
            .map_or_else(|| "?".to_owned(), |id| format!("#{id}"));
        let detail = if content_parts.is_empty() {
            format!("宝箱 {id_tag}（{status}）")
        } else {
            format!("宝箱 {id_tag}（{status}，{}）", content_parts.join("、"))
        };
        chest_labels.push(detail);

        chests_json.push(json!({
            "unique_id": unique_id,
            "chest_id": chest_id,
            "is_open": is_open,
            "item_id": item_id,
            "item_name": item_name,
            "item_num": item_num,
            "money": money,
            "is_trap": is_trap,
            "battle_group_id": battle_group_id,
            "location": chest_loc.map(|(_, v)| v),
        }));
    }

    let save_points_raw = obj_array(field, "save_points").unwrap_or(&[]);
    let mut save_points_json = Vec::with_capacity(save_points_raw.len());
    for raw in save_points_raw {
        let Some(sp) = raw.as_object() else {
            continue;
        };
        let target_map_id = obj_i64(sp, "target_map_id").filter(|&id| id >= 0);
        let target_map_name = target_map_id.and_then(|id| lookup.map_name(id));
        save_points_json.push(json!({
            "unique_id": obj_i64(sp, "unique_id"),
            "type": obj_str(sp, "type"),
            "is_teleport": obj_bool(sp, "is_teleport"),
            "can_teleport": obj_bool(sp, "can_teleport"),
            "target_map_id": target_map_id,
            "target_map_name": target_map_name,
            "target_point_id": obj_i64(sp, "target_point_id"),
            "location": format_location(obj_object(sp, "location")).map(|(_, v)| v),
        }));
    }

    let trans_and_npcs = obj_object(field, "transitions_and_npcs");
    let triggers_raw = trans_and_npcs
        .and_then(|t| obj_array(t, "triggers"))
        .unwrap_or(&[]);
    let mut triggers_json = Vec::with_capacity(triggers_raw.len());
    let mut exit_labels = Vec::new();
    for raw in triggers_raw {
        let Some(trig) = raw.as_object() else {
            continue;
        };
        let target_map_id = obj_i64(trig, "map_id").filter(|&id| id >= 0);
        let target_map_name = target_map_id.and_then(|id| lookup.map_name(id));
        if let Some(id) = target_map_id {
            let label = format_map_label(id, target_map_name.as_deref());
            if !exit_labels.contains(&label) {
                exit_labels.push(label);
            }
        }
        triggers_json.push(json!({
            "unique_id": obj_i64(trig, "unique_id"),
            "map_id": target_map_id,
            "map_name": target_map_name,
            "point_id": obj_i64(trig, "point_id"),
            "is_same_map": obj_bool(trig, "is_same_map"),
            "is_auto": obj_bool(trig, "is_auto"),
            "location": format_location(obj_object(trig, "location")).map(|(_, v)| v),
        }));
    }

    let npcs_raw = trans_and_npcs
        .and_then(|t| obj_array(t, "npcs"))
        .unwrap_or(&[]);
    let mut npcs_json = Vec::with_capacity(npcs_raw.len());
    let mut notable_npcs = Vec::new();
    for raw in npcs_raw {
        let Some(npc) = raw.as_object() else {
            continue;
        };
        let interact_type = obj_str(npc, "interact_type");
        let interact_zh = interact_type.map(interact_type_zh);
        let shop_id = obj_i64(npc, "shop_id").filter(|&id| id >= 0);
        let shop_name = shop_id.and_then(|id| lookup.shop_name(id));
        let inn_id = obj_i64(npc, "inn_id").filter(|&id| id >= 0);
        if let Some(sid) = shop_id {
            let label = match &shop_name {
                Some(name) => format!("商店 #{sid}「{name}」"),
                None => format!("商店 #{sid}"),
            };
            notable_npcs.push(label);
        } else if let Some(iid) = inn_id {
            notable_npcs.push(format!("旅店 #{iid}"));
        }
        npcs_json.push(json!({
            "unique_id": obj_i64(npc, "unique_id"),
            "unit_id": obj_i64(npc, "unit_id"),
            "interact_type": interact_type,
            "interact_type_zh": interact_zh,
            "shop_id": shop_id,
            "shop_name": shop_name,
            "inn_id": inn_id,
            "location": format_location(obj_object(npc, "location")).map(|(_, v)| v),
        }));
    }

    let quest_targets_obj = obj_object(field, "quest_targets");
    let qt_main = quest_targets_obj
        .and_then(|q| obj_i64(q, "main_quest_id"))
        .filter(|&id| id > 0);
    let qt_main_name = qt_main.and_then(|id| lookup.main_quest_title(id));
    let qt_pref = quest_targets_obj
        .and_then(|q| obj_i64(q, "preferred_quest_id"))
        .filter(|&id| id >= 0);
    let qt_pref_name = qt_pref.and_then(|id| {
        lookup
            .main_quest_title(id)
            .or_else(|| lookup.quest_name(id))
    });
    let qt_dest_ids =
        extract_i64_list(quest_targets_obj.and_then(|q| q.get("destination_map_ids")));
    let qt_dests: Vec<Value> = qt_dest_ids
        .iter()
        .map(|&id| {
            json!({
                "map_id": id,
                "map_name": lookup.map_name(id),
            })
        })
        .collect();

    let landmark_ids = extract_i64_list(field.get("unlocked_landmarks"));
    let landmarks_json: Vec<Value> = landmark_ids
        .iter()
        .map(|&id| {
            json!({
                "map_id": id,
                "map_name": lookup.map_name(id),
            })
        })
        .collect();

    // Build concise Chinese summary lines.
    let map_display = match (map_id, map_name.as_deref()) {
        (Some(id), Some(name)) => format!("地图 #{id}「{name}」"),
        (Some(id), None) => format!("地图 #{id}"),
        (None, Some(name)) => format!("地图「{name}」"),
        (None, None) => top_map.map_or_else(|| "未知地图".to_owned(), |lvl| format!("关卡 {lvl}")),
    };
    let area_part = area_id.map_or_else(String::new, |a| format!("（区域 {a}）"));
    let loc_part = loc_pair
        .as_ref()
        .map_or_else(|| "坐标未知".to_owned(), |(text, _)| format!("坐标 {text}"));
    let dir_part = match (player_direction_zh, player_direction_deg) {
        (Some(dir), Some(deg)) => format!("朝向 {dir}（{deg:.1}°）"),
        (Some(dir), None) => format!("朝向 {dir}"),
        (None, Some(deg)) => format!("朝向 {deg:.1}°"),
        (None, None) => "朝向未知".to_owned(),
    };
    let move_part = player_movement_zh.map_or_else(String::new, |m| format!("，移动方式：{m}"));
    let char_part = match (player_character_name.as_deref(), player_character_unit_id) {
        (Some(name), Some(cid)) => format!("，操作角色：{name}（#{cid}）"),
        (None, Some(cid)) => format!("，操作角色：#{cid}"),
        _ => String::new(),
    };

    let mut lines = vec![format!(
        "【野外】{map_display}{area_part}{char_part}，{loc_part}，{dir_part}{move_part}"
    )];

    let control_label = match can_control {
        Some(true) => "可操作",
        Some(false) => "不可操作",
        None => "未知",
    };
    let mut state_flags = Vec::new();
    if is_loading == Some(true) {
        state_flags.push("载入中");
    }
    if in_battle == Some(true) {
        state_flags.push("战斗中");
    }
    if in_event == Some(true) {
        state_flags.push("过场/事件中");
    }
    if in_dialog == Some(true) {
        state_flags.push("系统弹窗中");
    }
    if in_menu == Some(true) {
        state_flags.push("菜单打开");
    }
    if player_visible == Some(false) {
        state_flags.push("角色隐藏");
    }
    if state_flags.is_empty() {
        lines.push(format!("- 状态：{control_label}（无载入/过场/弹窗阻塞）"));
    } else {
        lines.push(format!(
            "- 状态：{control_label}（{}）",
            state_flags.join("、")
        ));
    }

    if let Some((interact_text, _)) = &interact_formatted {
        lines.push(format!("- 可交互对象：{interact_text}"));
    }

    if !chests_json.is_empty() {
        lines.push(format!(
            "- 本图宝箱：共 {} 个（未开 {unopened_count} / 已开 {opened_count}）：{}",
            chests_json.len(),
            chest_labels.join("；")
        ));
    }
    if !save_points_json.is_empty() {
        let tp_count = save_points_json
            .iter()
            .filter(|sp| sp["can_teleport"] == Value::Bool(true))
            .count();
        lines.push(format!(
            "- 本图存档点：{} 处（其中可传送 {tp_count} 处）",
            save_points_json.len()
        ));
    }
    if !exit_labels.is_empty() {
        lines.push(format!("- 连通出口：{}", exit_labels.join("、")));
    }
    if !notable_npcs.is_empty() {
        lines.push(format!("- 设施 NPC：{}", notable_npcs.join("、")));
    } else if !npcs_json.is_empty() {
        lines.push(format!("- 可见 NPC：{} 名", npcs_json.len()));
    }

    let payload = json!({
        "map_id": map_id,
        "map_name": map_name,
        "level_name": top_map,
        "area_id": area_id,
        "is_loading": is_loading,
        "in_battle": in_battle,
        "in_event": in_event,
        "in_dialog": in_dialog,
        "in_menu": in_menu,
        "can_control": can_control,
        "camera_deg": camera_deg,
        "camera_internal_rotation": camera_internal_rotation,
        "player": {
            "location": loc_pair.map(|(_, v)| v),
            "visible": player_visible,
            "unit_id": player_unit_id,
            "character_unit_id": player_character_unit_id,
            "character_name": player_character_name,
            "direction": player_direction,
            "direction_zh": player_direction_zh,
            "direction_deg": player_direction_deg,
            "movement_method": player_movement,
            "movement_method_zh": player_movement_zh,
            "speed_2d": player_speed,
        },
        "interact_target": interact_formatted.map(|(_, v)| v),
        "chests": chests_json,
        "chest_counts": {
            "total": chests_raw.len(),
            "unopened": unopened_count,
            "opened": opened_count,
        },
        "save_points": save_points_json,
        "transitions": triggers_json,
        "npcs": npcs_json,
        "quest_targets": {
            "main_quest_id": qt_main,
            "main_quest_name": qt_main_name,
            "preferred_quest_id": qt_pref,
            "preferred_quest_name": qt_pref_name,
            "destinations": qt_dests,
        },
        "unlocked_landmarks": landmarks_json,
    });

    SectionOutput {
        summary: lines.join("\n"),
        json: payload,
    }
}

/// Format a concise Chinese phrase describing `field.interact_target` for both the headline and field section.
pub(crate) fn describe_interact_brief(target: &Map<String, Value>, lookup: &LookupCtx) -> String {
    let kind = obj_str(target, "kind").unwrap_or("unknown");
    match kind {
        "treasure_box" => match obj_bool(target, "is_open") {
            Some(true) => "宝箱（已开）".to_owned(),
            Some(false) => "宝箱（未开）".to_owned(),
            None => "宝箱".to_owned(),
        },
        "save_point" => match obj_bool(target, "can_teleport") {
            Some(true) => "存档点（可传送）".to_owned(),
            Some(false) => "存档点（不可传送）".to_owned(),
            None => "存档点".to_owned(),
        },
        "npc" => {
            if let Some(shop_id) = obj_i64(target, "shop_id").filter(|&id| id >= 0) {
                match lookup.shop_name(shop_id) {
                    Some(name) => format!("NPC（商店 #{shop_id}「{name}」）"),
                    None => format!("NPC（商店 #{shop_id}）"),
                }
            } else if let Some(inn_id) = obj_i64(target, "inn_id").filter(|&id| id >= 0) {
                format!("NPC（旅店 #{inn_id}）")
            } else if let Some(itype) = obj_str(target, "interact_type") {
                format!("NPC（{}）", interact_type_zh(itype))
            } else {
                "NPC".to_owned()
            }
        }
        "gimmick" => {
            if let Some(itype) = obj_str(target, "interact_type") {
                format!("机关（{}）", interact_type_zh(itype))
            } else {
                "机关".to_owned()
            }
        }
        other => other.to_owned(),
    }
}

fn format_interact_target(target: &Map<String, Value>, lookup: &LookupCtx) -> (String, Value) {
    let kind = obj_str(target, "kind");
    let kind_zh = match kind {
        Some("treasure_box") => "宝箱",
        Some("save_point") => "存档点",
        Some("npc") => "NPC",
        Some("gimmick") => "机关",
        _ => "可交互物",
    };
    let unique_id = obj_i64(target, "unique_id");
    let event_id = obj_str(target, "event_id");
    let interact_type = obj_str(target, "interact_type");
    let interact_zh = interact_type.map(interact_type_zh);
    let is_open = obj_bool(target, "is_open");
    let can_teleport = obj_bool(target, "can_teleport");
    let shop_id = obj_i64(target, "shop_id").filter(|&id| id >= 0);
    let shop_name = shop_id.and_then(|id| lookup.shop_name(id));
    let inn_id = obj_i64(target, "inn_id").filter(|&id| id >= 0);
    let inn_price = inn_id.and_then(|id| lookup.inn_price(id));
    let loc = format_location(obj_object(target, "location"));

    let mut desc = describe_interact_brief(target, lookup);
    let mut extras = Vec::new();
    if let Some(uid) = unique_id {
        extras.push(format!("unique_id={uid}"));
    }
    if let Some(ev) = event_id {
        extras.push(format!("事件 {ev}"));
    }
    if let Some(price) = inn_price {
        extras.push(format!("住宿 {price}G"));
    }
    if let Some((loc_text, _)) = &loc {
        extras.push(format!("坐标 {loc_text}"));
    }
    if !extras.is_empty() {
        desc.push_str(&format!(" [{}]", extras.join("，")));
    }

    let val = json!({
        "kind": kind,
        "kind_zh": kind_zh,
        "unique_id": unique_id,
        "event_id": event_id,
        "interact_type": interact_type,
        "interact_type_zh": interact_zh,
        "is_open": is_open,
        "can_teleport": can_teleport,
        "shop_id": shop_id,
        "shop_name": shop_name,
        "inn_id": inn_id,
        "inn_price": inn_price,
        "location": loc.map(|(_, v)| v),
    });
    (desc, val)
}

pub(crate) fn resolve_field_map(
    raw_map_id: Option<i64>,
    top_map: Option<&str>,
    lookup: &LookupCtx,
) -> (Option<i64>, Option<String>) {
    if let Some(id) = raw_map_id
        && id >= 0
    {
        return (Some(id), lookup.map_name(id));
    }
    if let Some(level) = top_map
        && let Some((id, name)) = lookup.map_name_by_level(level)
    {
        return (Some(id), name);
    }
    (raw_map_id.filter(|&id| id >= 0), None)
}

fn render_v0_fallback(root: &Map<String, Value>, lookup: &LookupCtx) -> SectionOutput {
    let top_map = obj_str(root, "map");
    let (map_id, map_name) = resolve_field_map(None, top_map, lookup);
    let loc = format_location(obj_object(root, "pawn_location"));
    let map_desc = match (map_id, map_name.as_deref(), top_map) {
        (Some(id), Some(name), _) => format!("地图 #{id}「{name}」"),
        (Some(id), None, Some(lvl)) => format!("地图 #{id}（{lvl}）"),
        (_, _, Some(lvl)) => format!("关卡 {lvl}"),
        _ => "未知地图".to_owned(),
    };
    let loc_text = loc
        .as_ref()
        .map_or_else(|| "坐标未知".to_owned(), |(t, _)| format!("坐标 {t}"));
    let summary = format!("【野外】{map_desc}，{loc_text}（桥未提供 v1 field 主题）");
    let payload = json!({
        "available": false,
        "map_id": map_id,
        "map_name": map_name,
        "level_name": top_map,
        "player": {
            "location": loc.map(|(_, v)| v),
        },
    });
    SectionOutput {
        summary,
        json: payload,
    }
}

fn direction_zh(raw: &str) -> &'static str {
    match raw {
        "North" => "北",
        "South" => "南",
        "East" => "东",
        "West" => "西",
        "NorthEast" => "东北",
        "SouthEast" => "东南",
        "SouthWest" => "西南",
        "NorthWest" => "西北",
        _ => "未知",
    }
}

fn movement_method_zh(raw: &str) -> &'static str {
    match raw {
        "Walking" => "步行",
        "Chocobo" => "陆行鸟",
        "AirShip" => "飞空艇",
        "Ship" => "船",
        "MagickArmor" => "魔导装甲",
        _ => "未知",
    }
}
