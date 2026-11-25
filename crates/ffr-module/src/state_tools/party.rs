//! Party & progression topic formatter for `read_game_state`.

use serde_json::{Map, Value, json};

use crate::state_tools::{
    LookupCtx, SectionOutput, extract_i64_list, format_item_label, format_map_label,
    format_play_time, obj_array, obj_bool, obj_f64, obj_i64, obj_object, obj_str,
};

/// Render the `party` section from `data.party`, enriched with `ffr-data` units, items, maps,
/// chapters, main scenario objectives, and quests.
pub(crate) fn render_party(
    party: Option<&Map<String, Value>>,
    lookup: &LookupCtx,
) -> SectionOutput {
    let Some(party) = party else {
        return SectionOutput {
            summary: "【队伍】桥未提供 v1 party 主题。".to_owned(),
            json: json!({ "available": false }),
        };
    };

    let save_ready = obj_bool(party, "save_ready").unwrap_or(false);
    let money = obj_i64(party, "money");
    let play_time = obj_f64(party, "play_time");
    let play_time_text = play_time.map(format_play_time);
    let progress = obj_i64(party, "progress");
    let active_objective = progress
        .filter(|&p| p > 0)
        .and_then(|p| lookup.active_main_scenario(p));
    let chapter = obj_i64(party, "chapter")
        .or_else(|| active_objective.as_ref().and_then(|obj| obj.chapter_no));
    let chapter_name = chapter.and_then(|ch| lookup.chapter_name(ch));
    let main_quest_id = obj_i64(party, "main_quest_id")
        .filter(|&id| id > 0)
        .or_else(|| active_objective.as_ref().and_then(|obj| obj.main_quest_id));
    let main_quest_name = main_quest_id.and_then(|id| {
        lookup
            .main_quest_title(id)
            .or_else(|| lookup.quest_name(id))
    });
    let last_saved_slot = obj_i64(party, "last_saved_slot").filter(|&slot| slot >= 0);

    let dest_ids = extract_i64_list(party.get("destination_map_ids"));
    let destinations_json: Vec<Value> = dest_ids
        .iter()
        .map(|&id| {
            json!({
                "map_id": id,
                "map_name": lookup.map_name(id),
            })
        })
        .collect();
    let dest_labels: Vec<String> = dest_ids
        .iter()
        .map(|&id| format_map_label(id, lookup.map_name(id).as_deref()))
        .collect();

    let stats_map = obj_object(party, "party_stats");
    let raw_members = obj_array(party, "party").unwrap_or(&[]);
    let mut members_json = Vec::with_capacity(raw_members.len());
    let mut member_lines = Vec::with_capacity(raw_members.len());

    for raw in raw_members {
        let Some(m) = raw.as_object() else {
            continue;
        };
        let Some(unit_id) = obj_i64(m, "unit_id") else {
            continue;
        };
        let name = lookup.unit_name(unit_id);
        let name_display = name.as_deref().unwrap_or("未知角色");
        let level = obj_i64(m, "level");
        let hp = obj_i64(m, "hp");
        let max_hp = obj_i64(m, "max_hp");
        let mp = obj_i64(m, "mp");
        let max_mp = obj_i64(m, "max_mp");
        let exp = obj_i64(m, "exp");
        let weapon_id = obj_i64(m, "weapon_id").filter(|&id| id >= 0);
        let weapon_name = weapon_id.and_then(|id| lookup.item_name(id));
        let armor_id = obj_i64(m, "armor_id").filter(|&id| id >= 0);
        let armor_name = armor_id.and_then(|id| lookup.item_name(id));
        let acc_ids = extract_i64_list(m.get("accessory_ids"));
        let accessories_json: Vec<Value> = acc_ids
            .iter()
            .map(|&id| {
                json!({
                    "item_id": id,
                    "name": lookup.item_name(id),
                })
            })
            .collect();
        let vision_id = obj_i64(m, "vision_id").filter(|&id| id >= 0);
        let vision_name = vision_id.and_then(|id| lookup.item_name(id));
        let join_status = obj_i64(m, "join_status");
        let stats = stats_map
            .and_then(|sm| sm.get(&unit_id.to_string()))
            .cloned();

        let lv_str = level.map_or_else(|| "Lv.?".to_owned(), |lv| format!("Lv.{lv}"));
        let hp_str = format_pair(hp, max_hp);
        let mp_str = format_pair(mp, max_mp);
        let mut equip_parts = Vec::new();
        if let Some(wid) = weapon_id {
            equip_parts.push(format!(
                "武器：{}",
                format_item_label(wid, weapon_name.as_deref())
            ));
        }
        if let Some(aid) = armor_id {
            equip_parts.push(format!(
                "防具：{}",
                format_item_label(aid, armor_name.as_deref())
            ));
        }
        if let Some(vid) = vision_id {
            equip_parts.push(format!(
                "幻兵：{}",
                format_item_label(vid, vision_name.as_deref())
            ));
        }
        let equip_suffix = if equip_parts.is_empty() {
            String::new()
        } else {
            format!("（{}）", equip_parts.join("，"))
        };
        member_lines.push(format!(
            "  - {name_display}（#{unit_id}）{lv_str}：HP {hp_str}，MP {mp_str}{equip_suffix}"
        ));

        members_json.push(json!({
            "unit_id": unit_id,
            "name": name,
            "level": level,
            "hp": hp,
            "max_hp": max_hp,
            "mp": mp,
            "max_mp": max_mp,
            "exp": exp,
            "weapon_id": weapon_id,
            "weapon_name": weapon_name,
            "armor_id": armor_id,
            "armor_name": armor_name,
            "accessories": accessories_json,
            "vision_id": vision_id,
            "vision_name": vision_name,
            "join_status": join_status,
            "stats": stats,
        }));
    }

    let guest_json = obj_i64(party, "guest_unit_id")
        .filter(|&id| id >= 0)
        .map(|id| {
            json!({
                "unit_id": id,
                "name": lookup.unit_name(id),
            })
        });

    let inv_json = obj_object(party, "inventory").map(|inv| {
        let changed: Vec<Value> = obj_array(inv, "changed")
            .unwrap_or(&[])
            .iter()
            .filter_map(|entry| {
                let obj = entry.as_object()?;
                let item_id = obj_i64(obj, "item_id")?;
                Some(json!({
                    "item_id": item_id,
                    "name": lookup.item_name(item_id),
                    "before": obj_i64(obj, "before"),
                    "after": obj_i64(obj, "after"),
                    "delta": obj_i64(obj, "delta"),
                }))
            })
            .collect();
        json!({
            "count": obj_i64(inv, "count"),
            "total_qty": obj_i64(inv, "total_qty"),
            "hash": obj_str(inv, "hash"),
            "changed": changed,
        })
    });

    let flags_json = obj_object(party, "event_flags").map(|ef| {
        json!({
            "total": obj_i64(ef, "total"),
            "true_count": obj_i64(ef, "true_count"),
            "hash": obj_str(ef, "hash"),
            "added": extract_i64_list(ef.get("added")),
            "removed": extract_i64_list(ef.get("removed")),
        })
    });

    // Build concise Chinese summary.
    let mut lines = Vec::new();
    if !save_ready {
        lines.push("【队伍】尚未进入有效存档（标题占位数据）。".to_owned());
    } else {
        let chapter_desc = match (chapter, chapter_name.as_deref()) {
            (Some(0), _) => "序章".to_owned(),
            (Some(ch), Some(name)) => format!("第 {ch} 章「{name}」"),
            (Some(ch), None) => format!("第 {ch} 章"),
            (None, _) => "章节未知".to_owned(),
        };
        let money_desc = money.map_or_else(|| "金币未知".to_owned(), |g| format!("金币 {g}G"));
        let time_desc = play_time_text
            .as_deref()
            .map_or_else(String::new, |t| format!("，游玩时长 {t}"));
        let prog_desc = progress.map_or_else(String::new, |p| format!("（主线进度 {p}）"));
        lines.push(format!(
            "【队伍】{chapter_desc}{prog_desc}，{money_desc}{time_desc}"
        ));

        if let Some(obj) = &active_objective {
            if obj.is_prologue {
                lines.push("- 主线阶段：序章（尚无主线目标条目）".to_owned());
            } else if obj.row_key.is_some() {
                let goal_text = obj.goal.as_deref().unwrap_or("无中文目标");
                lines.push(format!("- 主线目标：{goal_text}"));
                if let Some(summary) = &obj.summary {
                    lines.push(format!("- 主线梗概：{summary}"));
                }
            }
        }
        if !dest_labels.is_empty() {
            lines.push(format!("- 目的地地图：{}", dest_labels.join("、")));
        }
        if let Some(qid) = main_quest_id {
            match &main_quest_name {
                Some(name) => lines.push(format!("- 当前主线任务：#{qid}「{name}」")),
                None => lines.push(format!("- 当前主线任务编号：#{qid}")),
            }
        }
        if !member_lines.is_empty() {
            lines.push("- 队伍成员：".to_owned());
            lines.extend(member_lines);
        }
        if let Some(guest) = &guest_json {
            let gid = guest["unit_id"].as_i64().unwrap_or(-1);
            let gname = guest["name"].as_str().unwrap_or("未知客座");
            lines.push(format!("- 客座成员：{gname}（#{gid}）"));
        }
    }

    let main_objective_json = active_objective.map(|obj| {
        json!({
            "chapter_no": obj.chapter_no,
            "is_prologue": obj.is_prologue,
            "main_quest_id": obj.main_quest_id,
            "main_quest_title": obj.main_quest_title,
            "row_key": obj.row_key,
            "min_progress": obj.min_progress,
            "next_progress": obj.next_progress,
            "goal": obj.goal,
            "summary": obj.summary,
            "destinations": obj.destinations.iter().map(|(id, name)| json!({
                "map_id": id,
                "map_name": name,
            })).collect::<Vec<_>>(),
        })
    });

    let payload = json!({
        "save_ready": save_ready,
        "money": money,
        "play_time": play_time,
        "play_time_text": play_time_text,
        "chapter": chapter,
        "chapter_name": chapter_name,
        "progress": progress,
        "main_objective": main_objective_json,
        "main_quest_id": main_quest_id,
        "main_quest_name": main_quest_name,
        "destinations": destinations_json,
        "last_saved_slot": last_saved_slot,
        "members": members_json,
        "guest": guest_json,
        "inventory_summary": inv_json,
        "event_flags_summary": flags_json,
    });

    SectionOutput {
        summary: lines.join("\n"),
        json: payload,
    }
}

fn format_pair(cur: Option<i64>, max: Option<i64>) -> String {
    match (cur, max) {
        (Some(c), Some(m)) => format!("{c}/{m}"),
        (Some(c), None) => format!("{c}/?"),
        (None, Some(m)) => format!("?/{m}"),
        (None, None) => "未知".to_owned(),
    }
}
