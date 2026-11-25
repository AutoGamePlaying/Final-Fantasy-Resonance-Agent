//! Battle topic formatter for `read_game_state` with weakness-fairness filtering.

use std::collections::{BTreeMap, BTreeSet};

use ffr_data::battle::{ALL_COMBAT_ELEMENTS, Element, Resistance};
use serde_json::{Map, Value, json};

use crate::data_tools::element_zh;
use crate::state_tools::{
    LookupCtx, SectionOutput, element_label_zh, extract_i64_list, format_item_label, obj_array,
    obj_bool, obj_i64, obj_object, obj_str, parse_element_code, resistance_label_zh,
};

/// Render the `battle` section from `data.battle`.
///
/// When `reveal_hidden_weaknesses` is `false` (the default fair-play mode), enemy weaknesses and
/// resistances whose in-game `analyzed` flag is `false` are withheld from both the Chinese summary
/// and the structured JSON; only the remaining unrevealed count is reported.
pub(crate) fn render_battle(
    battle: Option<&Map<String, Value>>,
    lookup: &LookupCtx,
    reveal_hidden_weaknesses: bool,
) -> SectionOutput {
    let Some(battle) = battle else {
        return SectionOutput {
            summary: "【战斗】桥未提供 v1 battle 主题。".to_owned(),
            json: json!({ "available": false }),
        };
    };

    let in_battle = obj_bool(battle, "in_battle").unwrap_or(false);
    let task_class = obj_str(battle, "task_class");
    let in_talk_event = obj_bool(battle, "in_talk_event");
    let in_tutorial = obj_bool(battle, "in_tutorial");

    if !in_battle {
        let task_desc = task_class.map_or_else(String::new, |t| format!("（当前任务 {t}）"));
        return SectionOutput {
            summary: format!("【战斗】当前不在战斗中{task_desc}。"),
            json: json!({
                "in_battle": false,
                "task_class": task_class,
                "in_talk_event": in_talk_event,
                "in_tutorial": in_tutorial,
            }),
        };
    }

    let phase = obj_str(battle, "phase");
    let phase_zh = phase.map(phase_label_zh);
    let is_event = obj_bool(battle, "is_event");
    let is_ui_visible = obj_bool(battle, "is_ui_visible");
    let working_job_count = obj_i64(battle, "working_job_count");
    let current_actor_uid = obj_i64(battle, "current_actor_uid").filter(|&uid| uid >= 0);
    let timeline_ids = extract_i64_list(battle.get("timeline"));
    let risk_uid_list = extract_i64_list(battle.get("risk_uids"));
    let risk_uid_set: BTreeSet<i64> = risk_uid_list.iter().copied().collect();

    let raw_units = obj_array(battle, "units").unwrap_or(&[]);
    let unit_labels = build_unit_label_map(raw_units, lookup);

    let cmd_obj = obj_object(battle, "cmd");
    let result_obj = obj_object(battle, "result");

    let blocking_reason = derive_blocking_reason(
        in_tutorial,
        in_talk_event,
        result_obj.is_some(),
        is_event,
        is_ui_visible,
        working_job_count,
        cmd_obj,
        current_actor_uid,
    );
    let can_act = blocking_reason.is_none();

    let current_actor_name = current_actor_uid
        .and_then(|uid| unit_labels.get(&uid).map(|info| info.display_name.clone()));

    let mut allies_json = Vec::new();
    let mut enemies_json = Vec::new();
    let mut ally_lines = Vec::new();
    let mut enemy_lines = Vec::new();
    let mut risk_warnings_json = Vec::new();
    let mut risk_lines = Vec::new();

    for raw in raw_units {
        let Some(u) = raw.as_object() else {
            continue;
        };
        let uid = obj_i64(u, "uid");
        let unit_id = obj_i64(u, "unit_id");
        let relation = obj_i64(u, "relation");
        let runtime_name = obj_str(u, "name");
        let resolved_name = uid
            .and_then(|id| unit_labels.get(&id).map(|info| info.display_name.clone()))
            .or_else(|| unit_id.and_then(|id| lookup.unit_name(id)))
            .or_else(|| runtime_name.map(str::to_owned));
        let name_display = resolved_name.as_deref().unwrap_or("未知单位");
        let is_boss = obj_bool(u, "is_boss").unwrap_or(false);
        let is_dead = obj_bool(u, "is_dead").unwrap_or(false);
        let hp = obj_i64(u, "hp");
        let max_hp = obj_i64(u, "max_hp");
        let mp = obj_i64(u, "mp");
        let max_mp = obj_i64(u, "max_mp");
        let bp = obj_i64(u, "bp");
        let runtime_max_bp = obj_i64(u, "max_bp");
        let lb = obj_i64(u, "lb");
        let lb_max = obj_i64(u, "lb_max");
        let lb_level = obj_i64(u, "lb_level");
        let is_break = obj_bool(u, "is_break").unwrap_or(false);
        let is_risky = obj_bool(u, "is_risky").unwrap_or(false)
            || uid.is_some_and(|id| risk_uid_set.contains(&id));
        let in_action = obj_bool(u, "in_action").unwrap_or(false);
        let in_ex_phase = obj_bool(u, "in_ex_phase").unwrap_or(false);
        let target_state = obj_str(u, "target_state");

        if relation == Some(2) {
            // Enemy unit.
            let profile = unit_id.and_then(|id| lookup.enemy_profile(id));
            let table_max_bp = profile.as_ref().map(|p| p.max_break_point);
            let effective_max_bp = runtime_max_bp.or(table_max_bp);
            let scheduled_skill_id = obj_i64(u, "scheduled_skill_id").filter(|&id| id > 0);
            let scheduled_skill_name = scheduled_skill_id.and_then(|id| lookup.skill_name(id));

            let affinity = build_enemy_affinity(
                unit_id,
                obj_array(u, "weaknesses"),
                profile.as_ref(),
                reveal_hidden_weaknesses,
            );

            let mut tags = Vec::new();
            if is_boss {
                tags.push("Boss");
            }
            if is_dead {
                tags.push("已击倒");
            }
            if is_break {
                tags.push("破防中");
            }
            if is_risky {
                tags.push("危险蓄力");
            }
            if target_state == Some("Main") {
                tags.push("主目标");
            } else if target_state == Some("Sub") {
                tags.push("副目标");
            }
            let tag_str = if tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", tags.join("/"))
            };
            let uid_str = uid.map_or_else(|| "?".to_owned(), |v| v.to_string());
            let unit_id_str = unit_id.map_or_else(|| "?".to_owned(), |v| format!("#{v}"));
            let hp_str = format_pair(hp, max_hp);
            let bp_str = format_pair(bp, effective_max_bp);

            let mut line = format!(
                "  - [uid={uid_str}] {name_display}（{unit_id_str}）{tag_str}：HP {hp_str}，破防槽 {bp_str}；{}",
                affinity.summary
            );
            if is_risky {
                let skill_desc = match (scheduled_skill_id, scheduled_skill_name.as_deref()) {
                    (Some(sid), Some(sname)) => format!("「{sname}」（#{sid}）"),
                    (Some(sid), None) => format!("技能 #{sid}"),
                    (None, _) => "危险招式".to_owned(),
                };
                line.push_str(&format!("；蓄力预告：{skill_desc}"));
                risk_lines.push(format!(
                    "[uid={uid_str}] {name_display} 蓄力 {skill_desc}（剩余破防槽 {bp_str}）"
                ));
                risk_warnings_json.push(json!({
                    "uid": uid,
                    "unit_id": unit_id,
                    "name": resolved_name,
                    "is_break": is_break,
                    "bp": bp,
                    "max_bp": effective_max_bp,
                    "scheduled_skill_id": scheduled_skill_id,
                    "scheduled_skill_name": scheduled_skill_name,
                }));
            }
            enemy_lines.push(line);

            enemies_json.push(json!({
                "uid": uid,
                "unit_id": unit_id,
                "name": resolved_name,
                "runtime_name": runtime_name,
                "is_boss": is_boss,
                "is_dead": is_dead,
                "hp": hp,
                "max_hp": max_hp,
                "bp": bp,
                "max_bp": effective_max_bp,
                "table_max_bp": table_max_bp,
                "is_break": is_break,
                "is_risky": is_risky,
                "scheduled_skill_id": scheduled_skill_id,
                "scheduled_skill_name": scheduled_skill_name,
                "in_action": in_action,
                "target_state": target_state,
                "weaknesses": affinity.weaknesses_json,
                "resistances": affinity.resistances_json,
            }));
        } else {
            // Friendly unit (relation == 1 or unspecified).
            let mut tags = Vec::new();
            if in_action || uid.is_some() && uid == current_actor_uid {
                tags.push("当前行动");
            }
            if in_ex_phase {
                tags.push("追加阶段");
            }
            if is_dead {
                tags.push("无法战斗");
            }
            if target_state == Some("Main") {
                tags.push("主目标");
            }
            let tag_str = if tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", tags.join("/"))
            };
            let uid_str = uid.map_or_else(|| "?".to_owned(), |v| v.to_string());
            let unit_id_str = unit_id.map_or_else(|| "?".to_owned(), |v| format!("#{v}"));
            let hp_str = format_pair(hp, max_hp);
            let mp_str = format_pair(mp, max_mp);
            let lb_part = match (lb, lb_max, lb_level) {
                (Some(cur), Some(max), Some(lv)) if max > 0 => {
                    format!("，LB {cur}/{max}（Lv.{lv}）")
                }
                _ => String::new(),
            };
            ally_lines.push(format!(
                "  - [uid={uid_str}] {name_display}（{unit_id_str}）{tag_str}：HP {hp_str}，MP {mp_str}{lb_part}"
            ));
            allies_json.push(json!({
                "uid": uid,
                "unit_id": unit_id,
                "name": resolved_name,
                "runtime_name": runtime_name,
                "is_dead": is_dead,
                "hp": hp,
                "max_hp": max_hp,
                "mp": mp,
                "max_mp": max_mp,
                "bp": bp,
                "max_bp": runtime_max_bp,
                "lb": lb,
                "lb_max": lb_max,
                "lb_level": lb_level,
                "in_action": in_action,
                "in_ex_phase": in_ex_phase,
                "target_state": target_state,
            }));
        }
    }

    let timeline_json: Vec<Value> = timeline_ids
        .iter()
        .map(|&uid| {
            let info = unit_labels.get(&uid);
            json!({
                "uid": uid,
                "unit_id": info.and_then(|i| i.unit_id),
                "name": info.map(|i| i.display_name.as_str()),
                "side": info.map(|i| i.side),
            })
        })
        .collect();
    let timeline_text: Vec<String> = timeline_ids
        .iter()
        .take(8)
        .map(|&uid| match unit_labels.get(&uid) {
            Some(info) => format!("{}[{}]", info.display_name, uid),
            None => format!("uid={uid}"),
        })
        .collect();

    let (cmd_summary, cmd_json) = cmd_obj
        .map(|cmd| format_cmd(cmd, &unit_labels, lookup))
        .unzip();

    let (result_summary, result_json) = result_obj.map(|res| format_result(res, lookup)).unzip();

    let mut lines = Vec::new();
    let phase_label = phase_zh.unwrap_or("未知阶段");
    let act_desc = match &blocking_reason {
        None => match (current_actor_uid, current_actor_name.as_deref()) {
            (Some(uid), Some(name)) => format!("可操作，当前行动者：{name}（uid={uid}）"),
            (Some(uid), None) => format!("可操作，当前行动者 uid={uid}"),
            (None, _) => "可操作".to_owned(),
        },
        Some(reason) => format!("暂不可操作（{reason}）"),
    };
    lines.push(format!("【战斗】{phase_label}，{act_desc}"));
    if !timeline_text.is_empty() {
        lines.push(format!("- 行动顺序：{}", timeline_text.join(" → ")));
    }
    if !risk_lines.is_empty() {
        lines.push(format!("- 危险预告：{}", risk_lines.join("；")));
    }
    if let Some(cmd_line) = cmd_summary {
        lines.push(format!("- 指令菜单：{cmd_line}"));
    }
    if !ally_lines.is_empty() {
        lines.push("- 我方：".to_owned());
        lines.extend(ally_lines);
    }
    if !enemy_lines.is_empty() {
        lines.push("- 敌方：".to_owned());
        lines.extend(enemy_lines);
    }
    if let Some(res_line) = result_summary {
        lines.push(format!("- 战斗结算：{res_line}"));
    }

    let payload = json!({
        "in_battle": true,
        "task_class": task_class,
        "phase": phase,
        "phase_zh": phase_zh,
        "can_act": can_act,
        "blocking_reason": blocking_reason,
        "in_talk_event": in_talk_event,
        "in_tutorial": in_tutorial,
        "is_event": is_event,
        "is_ui_visible": is_ui_visible,
        "working_job_count": working_job_count,
        "current_actor_uid": current_actor_uid,
        "current_actor_name": current_actor_name,
        "reveal_hidden_weaknesses": reveal_hidden_weaknesses,
        "timeline": timeline_json,
        "risk_uids": risk_uid_list,
        "risk_warnings": risk_warnings_json,
        "cmd": cmd_json,
        "allies": allies_json,
        "enemies": enemies_json,
        "result": result_json,
    });

    SectionOutput {
        summary: lines.join("\n"),
        json: payload,
    }
}

/// Produce a one-line Chinese headline for active battle scenes.
pub(crate) fn summarize_battle_headline(battle: &Map<String, Value>, lookup: &LookupCtx) -> String {
    let phase = obj_str(battle, "phase");
    let phase_prefix = match phase {
        Some("Additional") => "追加阶段，",
        Some("Finish") => "终结阶段，",
        _ => "",
    };

    if obj_bool(battle, "in_tutorial") == Some(true) {
        return "战斗中，教学弹窗阻塞".to_owned();
    }
    if obj_bool(battle, "in_talk_event") == Some(true) {
        return "战斗中，对话或剧情事件进行中".to_owned();
    }
    if let Some(res) = obj_object(battle, "result") {
        let page = match obj_i64(res, "page_index") {
            Some(0) => "报酬页",
            Some(1) => "成长/升级页",
            _ => "结算页",
        };
        return format!("战斗结算中（{page}）");
    }

    let raw_units = obj_array(battle, "units").unwrap_or(&[]);
    let unit_labels = build_unit_label_map(raw_units, lookup);
    let any_break = raw_units.iter().any(|u| {
        u.as_object().is_some_and(|obj| {
            obj_i64(obj, "relation") == Some(2)
                && obj_bool(obj, "is_dead") != Some(true)
                && obj_bool(obj, "is_break") == Some(true)
        })
    });
    let break_suffix = if any_break { "，敌方破防中" } else { "" };

    if obj_bool(battle, "is_event") == Some(true)
        || obj_bool(battle, "is_ui_visible") == Some(false)
    {
        return format!("战斗中，{phase_prefix}演出进行中（不可操作{break_suffix}）");
    }
    if let Some(jobs) = obj_i64(battle, "working_job_count")
        && jobs > 0
    {
        return format!("战斗中，{phase_prefix}行动执行中（队列 {jobs}{break_suffix}）");
    }

    let cmd = obj_object(battle, "cmd");
    if let Some(cmd) = cmd
        && obj_bool(cmd, "display") == Some(true)
    {
        let actor_uid = obj_i64(battle, "current_actor_uid").filter(|&u| u >= 0);
        let actor = actor_uid
            .and_then(|uid| unit_labels.get(&uid).map(|i| i.display_name.as_str()))
            .unwrap_or("我方角色");
        let ancestors = extract_i64_list(cmd.get("ancestors"));
        let prefer_item = ancestors.contains(&7);

        if obj_str(cmd, "state") == Some("Target")
            && let Some(target) = obj_object(cmd, "target")
        {
            let skill_label = obj_i64(target, "skill_id")
                .map(|id| lookup.resolve_battle_action(id, prefer_item).label())
                .unwrap_or_else(|| "未知招式".to_owned());
            let focus_uid = obj_i64(target, "focus_uid").filter(|&u| u >= 0);
            let focus_label = focus_uid
                .and_then(|uid| unit_labels.get(&uid))
                .map(|info| format!("{}[uid={}]", info.display_name, info.uid))
                .unwrap_or_else(|| "未知目标".to_owned());
            let aoe_note = if obj_bool(target, "is_all") == Some(true) {
                "，全体化"
            } else {
                ""
            };
            return format!(
                "战斗中，{phase_prefix}轮到{actor}选目标（招式「{skill_label}」，光标在 {focus_label}{aoe_note}{break_suffix}）"
            );
        }

        let focus_label = obj_i64(cmd, "focus_action_id")
            .map(|id| lookup.resolve_battle_action(id, prefer_item).label())
            .unwrap_or_else(|| "未知项".to_owned());
        return format!(
            "战斗中，{phase_prefix}轮到{actor}选指令（光标在「{focus_label}」{break_suffix}）"
        );
    }

    format!("战斗中，{phase_prefix}回合过渡等待中{break_suffix}")
}

struct UnitLabelInfo {
    uid: i64,
    unit_id: Option<i64>,
    display_name: String,
    side: &'static str,
}

fn build_unit_label_map(raw_units: &[Value], lookup: &LookupCtx) -> BTreeMap<i64, UnitLabelInfo> {
    let mut map = BTreeMap::new();
    for raw in raw_units {
        let Some(u) = raw.as_object() else {
            continue;
        };
        let Some(uid) = obj_i64(u, "uid") else {
            continue;
        };
        let unit_id = obj_i64(u, "unit_id");
        let name = unit_id
            .and_then(|id| lookup.unit_name(id))
            .or_else(|| obj_str(u, "name").map(str::to_owned))
            .unwrap_or_else(|| match unit_id {
                Some(id) => format!("单位 #{id}"),
                None => format!("uid={uid}"),
            });
        let side = match obj_i64(u, "relation") {
            Some(1) => "ally",
            Some(2) => "enemy",
            _ => "unknown",
        };
        map.insert(
            uid,
            UnitLabelInfo {
                uid,
                unit_id,
                display_name: name,
                side,
            },
        );
    }
    map
}

struct EnemyAffinityOutput {
    summary: String,
    weaknesses_json: Value,
    resistances_json: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ResistanceEntry {
    element_code: String,
    element_zh: String,
    resistance_code: String,
    resistance_zh: &'static str,
}

fn build_enemy_affinity(
    unit_id: Option<i64>,
    runtime_list: Option<&[Value]>,
    profile: Option<&ffr_data::battle::EnemyProfile>,
    reveal_hidden: bool,
) -> EnemyAffinityOutput {
    let mut revealed_weak: BTreeSet<Element> = BTreeSet::new();
    let mut runtime_hidden_weak: BTreeSet<Element> = BTreeSet::new();
    let mut revealed_res: BTreeMap<Element, ResistanceEntry> = BTreeMap::new();
    let mut runtime_hidden_res: BTreeMap<Element, ResistanceEntry> = BTreeMap::new();

    if let Some(items) = runtime_list {
        for item in items {
            let Some(obj) = item.as_object() else {
                continue;
            };
            let Some(elem_raw) = obj_str(obj, "element") else {
                continue;
            };
            let Some(res_raw) = obj_str(obj, "resistance") else {
                continue;
            };
            let analyzed = obj_bool(obj, "analyzed").unwrap_or(false);
            let Some(elem) = parse_element_code(elem_raw) else {
                continue;
            };
            if elem == Element::None || res_raw == "None" {
                continue;
            }
            if res_raw == "Weakness" {
                if analyzed {
                    revealed_weak.insert(elem);
                } else {
                    runtime_hidden_weak.insert(elem);
                }
            } else {
                let entry = ResistanceEntry {
                    element_code: element_code(elem).to_owned(),
                    element_zh: element_label_zh(elem_raw),
                    resistance_code: res_raw.to_owned(),
                    resistance_zh: resistance_label_zh(res_raw),
                };
                if analyzed {
                    revealed_res.insert(elem, entry);
                } else {
                    runtime_hidden_res.insert(elem, entry);
                }
            }
        }
    }

    // Merge static table weaknesses & resistances to calculate total unrevealed count accurately.
    let mut all_weak: BTreeSet<Element> = revealed_weak.clone();
    all_weak.extend(runtime_hidden_weak.iter().copied());
    if let Some(p) = profile {
        all_weak.extend(p.weaknesses.iter().copied());
    }
    let unrevealed_weak: Vec<Element> = ALL_COMBAT_ELEMENTS
        .iter()
        .copied()
        .filter(|e| all_weak.contains(e) && !revealed_weak.contains(e))
        .collect();
    let revealed_weak_ordered: Vec<Element> = ALL_COMBAT_ELEMENTS
        .iter()
        .copied()
        .filter(|e| revealed_weak.contains(e))
        .collect();
    let all_weak_ordered: Vec<Element> = ALL_COMBAT_ELEMENTS
        .iter()
        .copied()
        .filter(|e| all_weak.contains(e))
        .collect();

    let mut hidden_res: BTreeMap<Element, ResistanceEntry> = runtime_hidden_res;
    if let Some(p) = profile {
        for &elem in &ALL_COMBAT_ELEMENTS {
            if revealed_res.contains_key(&elem) || hidden_res.contains_key(&elem) {
                continue;
            }
            let Some(res) = p.element_resistances.get(&elem).copied() else {
                continue;
            };
            let res_code = match res {
                Resistance::Halved => "Halved",
                Resistance::Invalid => "Invalid",
                Resistance::Absorb => "Absorb",
                Resistance::Weakness | Resistance::None => continue,
            };
            hidden_res.insert(
                elem,
                ResistanceEntry {
                    element_code: element_code(elem).to_owned(),
                    element_zh: element_zh(elem).to_owned(),
                    resistance_code: res_code.to_owned(),
                    resistance_zh: resistance_label_zh(res_code),
                },
            );
        }
    }
    for elem in revealed_res.keys() {
        hidden_res.remove(elem);
    }

    let revealed_zh: Vec<&str> = revealed_weak_ordered
        .iter()
        .copied()
        .map(element_zh)
        .collect();
    let revealed_codes: Vec<&str> = revealed_weak_ordered
        .iter()
        .copied()
        .map(element_code)
        .collect();
    let unrevealed_count = unrevealed_weak.len();

    let revealed_res_vec: Vec<&ResistanceEntry> = ALL_COMBAT_ELEMENTS
        .iter()
        .filter_map(|e| revealed_res.get(e))
        .collect();
    let hidden_res_vec: Vec<&ResistanceEntry> = ALL_COMBAT_ELEMENTS
        .iter()
        .filter_map(|e| hidden_res.get(e))
        .collect();

    // Summary text & JSON obey the fairness flag strictly: when `reveal_hidden == false`,
    // never emit unrevealed element names anywhere in text or JSON.
    let mut summary_parts = Vec::new();
    let (weaknesses_json, resistances_json) = if reveal_hidden {
        let unrevealed_zh: Vec<&str> = unrevealed_weak.iter().copied().map(element_zh).collect();
        let unrevealed_codes: Vec<&str> =
            unrevealed_weak.iter().copied().map(element_code).collect();
        let all_zh: Vec<&str> = all_weak_ordered.iter().copied().map(element_zh).collect();
        let all_codes: Vec<&str> = all_weak_ordered.iter().copied().map(element_code).collect();

        if all_zh.is_empty() {
            summary_parts.push("弱点：无".to_owned());
        } else if unrevealed_zh.is_empty() {
            summary_parts.push(format!(
                "已揭示弱点：{}（已全部揭示）",
                revealed_zh.join("、")
            ));
        } else {
            let rev_text = if revealed_zh.is_empty() {
                "无".to_owned()
            } else {
                revealed_zh.join("、")
            };
            summary_parts.push(format!(
                "已揭示弱点：{rev_text}；未揭示弱点（已开启揭示开关）：{}",
                unrevealed_zh.join("、")
            ));
        }

        if !revealed_res_vec.is_empty() {
            summary_parts.push(format!(
                "已揭示抗性：{}",
                format_resistance_entries(&revealed_res_vec)
            ));
        }
        if !hidden_res_vec.is_empty() {
            summary_parts.push(format!(
                "未揭示抗性（已开启揭示开关）：{}",
                format_resistance_entries(&hidden_res_vec)
            ));
        }

        (
            json!({
                "revealed": revealed_zh,
                "revealed_elements": revealed_codes,
                "unrevealed": unrevealed_zh,
                "unrevealed_elements": unrevealed_codes,
                "all": all_zh,
                "all_elements": all_codes,
                "unrevealed_count": unrevealed_count,
                "hidden_revealed": true,
            }),
            json!({
                "revealed": resistance_entries_json(&revealed_res_vec),
                "unrevealed": resistance_entries_json(&hidden_res_vec),
                "unrevealed_count": hidden_res_vec.len(),
                "hidden_revealed": true,
            }),
        )
    } else {
        if revealed_zh.is_empty() && unrevealed_count == 0 {
            summary_parts.push("弱点：无".to_owned());
        } else if unrevealed_count == 0 {
            summary_parts.push(format!(
                "已揭示弱点：{}（已全部揭示）",
                revealed_zh.join("、")
            ));
        } else {
            let rev_text = if revealed_zh.is_empty() {
                "无".to_owned()
            } else {
                revealed_zh.join("、")
            };
            let read_enemy_hint = match unit_id {
                Some(id) => format!("完整弱点可用 read_enemy(unit_id={id}) 查"),
                None => "完整弱点可用 read_enemy 按编号查".to_owned(),
            };
            summary_parts.push(format!(
                "已揭示弱点：{rev_text}（数据表另有 {unrevealed_count} 个未揭示弱点，{read_enemy_hint}）"
            ));
        }

        if !revealed_res_vec.is_empty() {
            summary_parts.push(format!(
                "已揭示抗性：{}",
                format_resistance_entries(&revealed_res_vec)
            ));
        }

        (
            json!({
                "revealed": revealed_zh,
                "revealed_elements": revealed_codes,
                "unrevealed_count": unrevealed_count,
                "hidden_revealed": false,
            }),
            json!({
                "revealed": resistance_entries_json(&revealed_res_vec),
                "unrevealed_count": hidden_res_vec.len(),
                "hidden_revealed": false,
            }),
        )
    };

    EnemyAffinityOutput {
        summary: summary_parts.join("；"),
        weaknesses_json,
        resistances_json,
    }
}

fn format_resistance_entries(entries: &[&ResistanceEntry]) -> String {
    entries
        .iter()
        .map(|e| format!("{}({})", e.element_zh, e.resistance_zh))
        .collect::<Vec<_>>()
        .join("、")
}

fn resistance_entries_json(entries: &[&ResistanceEntry]) -> Vec<Value> {
    entries
        .iter()
        .map(|e| {
            json!({
                "element": e.element_code,
                "element_zh": e.element_zh,
                "resistance": e.resistance_code,
                "resistance_zh": e.resistance_zh,
            })
        })
        .collect()
}

fn format_cmd(
    cmd: &Map<String, Value>,
    units: &BTreeMap<i64, UnitLabelInfo>,
    lookup: &LookupCtx,
) -> (String, Value) {
    let display = obj_bool(cmd, "display").unwrap_or(false);
    let cmd_type = obj_str(cmd, "type");
    let cmd_state = obj_str(cmd, "state");
    let ancestor_ids = extract_i64_list(cmd.get("ancestors"));
    let prefer_item = ancestor_ids.contains(&7);
    let focus_index = obj_i64(cmd, "focus_index");
    let focus_action_id = obj_i64(cmd, "focus_action_id").filter(|&id| id >= 0);
    let focus_action = focus_action_id.map(|id| lookup.resolve_battle_action(id, prefer_item));

    let ancestors_json: Vec<Value> = ancestor_ids
        .iter()
        .map(|&id| lookup.resolve_battle_action(id, false).to_json())
        .collect();

    let primary_ids = extract_i64_list(cmd.get("primary_ids"));
    let primary_actions: Vec<Value> = primary_ids
        .iter()
        .map(|&id| lookup.resolve_battle_action(id, false).to_json())
        .collect();

    let secondary_ids = extract_i64_list(cmd.get("secondary_ids"));
    let secondary_actions: Vec<Value> = secondary_ids
        .iter()
        .map(|&id| {
            let resolved = lookup.resolve_battle_action(id, prefer_item);
            let mut obj = resolved.to_json();
            if resolved.kind == "skill"
                && let Some(skill) = lookup.skill(id)
                && let Some(map) = obj.as_object_mut()
            {
                map.insert("element".to_owned(), json!(element_zh(skill.element)));
                map.insert("cost".to_owned(), json!(skill.cost));
                map.insert("break_damage".to_owned(), json!(skill.break_damage));
            }
            obj
        })
        .collect();

    let target_obj = obj_object(cmd, "target");
    let target_json = target_obj.map(|t| {
        let skill_id = obj_i64(t, "skill_id").filter(|&id| id >= 0);
        let skill_resolved = skill_id.map(|id| lookup.resolve_battle_action(id, prefer_item));
        let focus_uid = obj_i64(t, "focus_uid").filter(|&uid| uid >= 0);
        let focus_name = focus_uid.and_then(|uid| units.get(&uid).map(|i| i.display_name.as_str()));
        let target_uids = extract_i64_list(t.get("target_uids"));
        let all_target_uids = extract_i64_list(t.get("all_target_uids"));
        let targets: Vec<Value> = target_uids
            .iter()
            .map(|&uid| unit_ref_json(uid, units))
            .collect();
        let all_targets: Vec<Value> = all_target_uids
            .iter()
            .map(|&uid| unit_ref_json(uid, units))
            .collect();
        json!({
            "skill_id": skill_id,
            "skill_name": skill_resolved.as_ref().and_then(|r| r.name.as_deref()),
            "focus_uid": focus_uid,
            "focus_name": focus_name,
            "target_uids": target_uids,
            "targets": targets,
            "all_target_uids": all_target_uids,
            "all_targets": all_targets,
            "can_aoe": obj_bool(t, "can_aoe"),
            "is_all": obj_bool(t, "is_all"),
        })
    });

    let summary = if !display {
        "未打开".to_owned()
    } else if cmd_state == Some("Target")
        && let Some(t) = &target_json
    {
        let skill_label = match (t["skill_id"].as_i64(), t["skill_name"].as_str()) {
            (Some(id), Some(name)) => format!("「{name}」（#{id}）"),
            (Some(id), None) => format!("#{id}"),
            _ => "未知招式".to_owned(),
        };
        let focus_desc = match (t["focus_uid"].as_i64(), t["focus_name"].as_str()) {
            (Some(uid), Some(name)) => format!("{name}[uid={uid}]"),
            (Some(uid), None) => format!("uid={uid}"),
            _ => "无".to_owned(),
        };
        let aoe = match (t["can_aoe"].as_bool(), t["is_all"].as_bool()) {
            (Some(true), Some(true)) => "，当前全体化",
            (Some(true), Some(false)) => "，可切换全体化",
            _ => "",
        };
        format!("选目标中，招式 {skill_label}，光标在 {focus_desc}{aoe}")
    } else {
        let focus_desc = focus_action
            .as_ref()
            .map_or_else(|| "无".to_owned(), |r| r.display_with_id());
        let idx_part = focus_index.map_or_else(String::new, |i| format!("索引 {i}，"));
        let mut parts = vec![format!("已打开（列表态，{idx_part}光标：{focus_desc}）")];
        if !ancestor_ids.is_empty() {
            let path: Vec<String> = ancestor_ids
                .iter()
                .map(|&id| lookup.resolve_battle_action(id, false).label())
                .collect();
            parts.push(format!("层级：{}", path.join(" > ")));
        }
        if !secondary_ids.is_empty() {
            let items: Vec<String> = secondary_ids
                .iter()
                .map(|&id| {
                    lookup
                        .resolve_battle_action(id, prefer_item)
                        .display_with_id()
                })
                .collect();
            parts.push(format!("二级列表：{}", items.join("、")));
        } else if !primary_ids.is_empty() {
            let items: Vec<String> = primary_ids
                .iter()
                .map(|&id| lookup.resolve_battle_action(id, false).display_with_id())
                .collect();
            parts.push(format!("一级列表：{}", items.join("、")));
        }
        parts.join("；")
    };

    let json_val = json!({
        "display": display,
        "type": cmd_type,
        "state": cmd_state,
        "ancestors": ancestors_json,
        "focus_index": focus_index,
        "focus_action_id": focus_action_id,
        "focus_action_name": focus_action.as_ref().and_then(|r| r.name.as_deref()),
        "primary_actions": primary_actions,
        "secondary_actions": secondary_actions,
        "target": target_json,
    });
    (summary, json_val)
}

fn format_result(res: &Map<String, Value>, lookup: &LookupCtx) -> (String, Value) {
    let page_index = obj_i64(res, "page_index");
    let page_name = match page_index {
        Some(0) => "报酬页",
        Some(1) => "成长/升级页",
        _ => "结算页",
    };
    let total_exp = obj_i64(res, "total_exp");
    let total_money = obj_i64(res, "total_money");
    let total_ap = obj_i64(res, "total_ap");

    let enemy_ids = extract_i64_list(res.get("enemy_ids"));
    let enemies_json: Vec<Value> = enemy_ids
        .iter()
        .map(|&id| {
            json!({
                "unit_id": id,
                "name": lookup.unit_name(id),
            })
        })
        .collect();

    let raw_items = obj_array(res, "items").unwrap_or(&[]);
    let mut items_json = Vec::with_capacity(raw_items.len());
    let mut item_labels = Vec::new();
    for raw in raw_items {
        let Some(item) = raw.as_object() else {
            continue;
        };
        let Some(item_id) = obj_i64(item, "item_id") else {
            continue;
        };
        let num = obj_i64(item, "num").unwrap_or(1);
        let name = lookup.item_name(item_id);
        item_labels.push(format!(
            "{}×{num}",
            format_item_label(item_id, name.as_deref())
        ));
        items_json.push(json!({
            "item_id": item_id,
            "name": name,
            "num": num,
        }));
    }

    let mut parts = vec![page_name.to_owned()];
    if let Some(exp) = total_exp {
        parts.push(format!("经验 +{exp}"));
    }
    if let Some(money) = total_money {
        parts.push(format!("金币 +{money}G"));
    }
    if let Some(ap) = total_ap {
        parts.push(format!("AP +{ap}"));
    }
    if !item_labels.is_empty() {
        parts.push(format!("掉落：{}", item_labels.join("、")));
    }

    let payload = json!({
        "page_index": page_index,
        "page_name": page_name,
        "total_exp": total_exp,
        "total_money": total_money,
        "total_ap": total_ap,
        "enemies": enemies_json,
        "items": items_json,
    });
    (parts.join("，"), payload)
}

fn unit_ref_json(uid: i64, units: &BTreeMap<i64, UnitLabelInfo>) -> Value {
    let info = units.get(&uid);
    json!({
        "uid": uid,
        "unit_id": info.and_then(|i| i.unit_id),
        "name": info.map(|i| i.display_name.as_str()),
        "side": info.map(|i| i.side),
    })
}

#[allow(clippy::too_many_arguments)]
fn derive_blocking_reason(
    in_tutorial: Option<bool>,
    in_talk_event: Option<bool>,
    has_result: bool,
    is_event: Option<bool>,
    is_ui_visible: Option<bool>,
    working_job_count: Option<i64>,
    cmd: Option<&Map<String, Value>>,
    current_actor_uid: Option<i64>,
) -> Option<String> {
    if in_tutorial == Some(true) {
        return Some("教学弹窗".to_owned());
    }
    if in_talk_event == Some(true) {
        return Some("对话/剧情事件".to_owned());
    }
    if has_result {
        return Some("战斗结算".to_owned());
    }
    if is_event == Some(true) || is_ui_visible == Some(false) {
        return Some("演出/动画进行中".to_owned());
    }
    if let Some(jobs) = working_job_count
        && jobs > 0
    {
        return Some(format!("招式执行中（队列 {jobs}）"));
    }
    let cmd_display = cmd.and_then(|c| obj_bool(c, "display")).unwrap_or(false);
    if !cmd_display {
        return Some("指令菜单未显示".to_owned());
    }
    if current_actor_uid.is_none() {
        return Some("当前无行动者".to_owned());
    }
    None
}

fn phase_label_zh(raw: &str) -> &'static str {
    match raw {
        "Normal" => "常规阶段",
        "Additional" => "追加阶段",
        "Finish" => "终结阶段",
        _ => "未知阶段",
    }
}

fn element_code(element: Element) -> &'static str {
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

fn format_pair(cur: Option<i64>, max: Option<i64>) -> String {
    match (cur, max) {
        (Some(c), Some(m)) => format!("{c}/{m}"),
        (Some(c), None) => format!("{c}/?"),
        (None, Some(m)) => format!("?/{m}"),
        (None, None) => "未知".to_owned(),
    }
}
