//! On-demand read-only FfrBridge v1 command tools (`ToolCategory::Status`).
//!
//! Exposes `read_save_slots`, `read_inventory`, `read_event_flags`, `read_input_state`, and
//! `query_nav_path`, forwarding each call via [`FileChannel::send`] and translating the response
//! into a Chinese summary plus structured JSON enriched with `ffr-data`.

use std::collections::{BTreeMap, BTreeSet};

use agp_game::{CallContext, Content, ToolCategory, ToolDescriptor, ToolOutput};
use agp_platform::FileChannel;
use ffr_data::FlagConditionEntry;
use serde_json::{Map, Value, json};

use crate::data_cache::GameDataSource;
use crate::state_tools::{
    LookupCtx, extract_i64_list, extract_str_list, format_item_label, format_location,
    format_map_label, format_play_time, item_category_zh, obj_array, obj_bool, obj_f64, obj_i64,
    obj_object, obj_str,
};

const INVENTORY_SUMMARY_LIMIT: usize = 50;
const EVENT_FLAGS_SUMMARY_LIMIT: usize = 64;

/// Names of all on-demand bridge command tools exposed by this module.
pub const BRIDGE_COMMAND_TOOL_NAMES: [&str; 5] = [
    "read_save_slots",
    "read_inventory",
    "read_event_flags",
    "read_input_state",
    "query_nav_path",
];

/// Returns `true` if `name` is one of the on-demand read-only bridge command tools.
pub fn is_bridge_command_tool(name: &str) -> bool {
    BRIDGE_COMMAND_TOOL_NAMES.contains(&name)
}

/// Build descriptors for all 5 on-demand read-only bridge command tools.
pub fn bridge_command_tool_descriptors() -> Vec<ToolDescriptor> {
    let empty_params = json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    });
    let point_schema = json!({
        "type": "object",
        "properties": {
            "x": { "type": "number", "description": "世界坐标 X" },
            "y": { "type": "number", "description": "世界坐标 Y" },
            "z": { "type": "number", "description": "世界坐标 Z" }
        },
        "required": ["x", "y", "z"],
        "additionalProperties": false
    });

    vec![
        ToolDescriptor {
            name: "read_save_slots".to_owned(),
            description: "向游戏内文件桥发送只读 save_slots 命令，读取全部存档槽（0=自动槽、1..10=手动槽）的占用、章节、地图、游玩时长与队伍摘要。"
                .to_owned(),
            parameters: empty_params.clone(),
            category: ToolCategory::Status,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "read_inventory".to_owned(),
            description: "向游戏内文件桥发送只读 inventory 命令，读取完整未截断物品栏清单并结合静态道具表补全中文名与分类。"
                .to_owned(),
            parameters: empty_params,
            category: ToolCategory::Status,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "read_event_flags".to_owned(),
            description: "向游戏内文件桥发送只读 event_flags 命令，读取 2048 位事件旗标中全部已置位的 0-based 编号；可传入 flag_ids 检查指定旗标状态。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "flag_ids": {
                        "type": "array",
                        "items": { "type": "integer", "minimum": 0 },
                        "description": "可选，指定要核查的 0-based 事件旗标编号列表；省略时返回全部已置位编号。"
                    }
                },
                "additionalProperties": false
            }),
            category: ToolCategory::Status,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "read_input_state".to_owned(),
            description: "向游戏内文件桥发送只读 input_state 命令，读取当前激活的 Enhanced Input 上下文、输入屏蔽令牌与动作键位绑定。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "mapping_names": {
                        "type": "array",
                        "items": { "type": "string", "minLength": 1 },
                        "description": "可选，指定要查询物理键码的 MappingName 列表；省略时返回常用默认键位集合。"
                    },
                    "slot": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "可选，按键绑定槽位号（默认 0）。"
                    }
                },
                "additionalProperties": false
            }),
            category: ToolCategory::Status,
            side_effects: false,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: "query_nav_path".to_owned(),
            description: "向游戏内文件桥发送只读 query_nav_path 命令，基于引擎导航网格同步计算两点间的可达性、路径总长与路点序列。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "start": point_schema.clone(),
                    "goal": point_schema
                },
                "required": ["start", "goal"],
                "additionalProperties": false
            }),
            category: ToolCategory::Status,
            side_effects: false,
            timeout_ms: None,
        },
    ]
}

/// Dispatch one on-demand read-only bridge command tool call.
pub async fn call_bridge_command(
    channel: &FileChannel,
    source: &GameDataSource,
    name: &str,
    args: Value,
    ctx: &CallContext,
) -> ToolOutput {
    let Some(obj) = args.as_object() else {
        return arg_error(format!("工具 `{name}` 的参数必须是 JSON 对象"));
    };

    match name {
        "read_save_slots" => handle_read_save_slots(channel, source, obj, ctx).await,
        "read_inventory" => handle_read_inventory(channel, source, obj, ctx).await,
        "read_event_flags" => handle_read_event_flags(channel, source, obj, ctx).await,
        "read_input_state" => handle_read_input_state(channel, obj, ctx).await,
        "query_nav_path" => handle_query_nav_path(channel, obj, ctx).await,
        _ => arg_error(format!("未知桥命令工具 `{name}`")),
    }
}

async fn handle_read_save_slots(
    channel: &FileChannel,
    source: &GameDataSource,
    args: &Map<String, Value>,
    ctx: &CallContext,
) -> ToolOutput {
    if !args.is_empty() {
        return arg_error("工具 `read_save_slots` 不接受参数，请传入空对象 {}".to_owned());
    }
    let result = match send_bridge_op(channel, "save_slots", json!({}), ctx).await {
        Ok(val) => val,
        Err(output) => return output,
    };
    let Some(res) = result.as_object() else {
        return bridge_format_error("save_slots", "回复 result 不是对象");
    };

    let lookup = LookupCtx::from_source(source, true, true, true);
    let slot_count = obj_i64(res, "slot_count");
    let last_saved_slot = obj_i64(res, "last_saved_slot").filter(|&s| s >= 0);
    let raw_slots = obj_array(res, "slots").unwrap_or(&[]);

    let mut slots_json = Vec::with_capacity(raw_slots.len());
    let mut occupied_lines = Vec::new();
    let mut empty_indices = Vec::new();

    for raw in raw_slots {
        let Some(slot) = raw.as_object() else {
            continue;
        };
        let slot_index = obj_i64(slot, "slot_index").unwrap_or(-1);
        let is_auto = obj_bool(slot, "is_auto_save").unwrap_or(slot_index == 0);
        let exists = obj_bool(slot, "exists").unwrap_or(false);
        let load_status = obj_i64(slot, "load_status");
        let main_progress = obj_i64(slot, "main_progress");
        let chapter = obj_i64(slot, "chapter");
        let chapter_name = chapter.and_then(|ch| lookup.chapter_name(ch));
        let current_map_id = obj_i64(slot, "current_map_id").filter(|&id| id >= 0);
        let current_map_name = current_map_id.and_then(|id| lookup.map_name(id));
        let play_level = obj_i64(slot, "play_level");
        let play_time = obj_f64(slot, "play_time");
        let play_time_text = play_time.map(format_play_time);
        let date_time_ticks = obj_str(slot, "date_time_ticks");
        let difficulty = obj_i64(slot, "difficulty");
        let difficulty_zh = difficulty.map(difficulty_label_zh);
        let is_cleared = obj_bool(slot, "is_cleared");
        let party_unit_ids = extract_i64_list(slot.get("party_unit_ids"));
        let party_members: Vec<Value> = party_unit_ids
            .iter()
            .map(|&id| {
                json!({
                    "unit_id": id,
                    "name": lookup.unit_name(id),
                })
            })
            .collect();

        if exists {
            let slot_kind = if is_auto { "自动" } else { "手动" };
            let ch_desc = match (chapter, chapter_name.as_deref()) {
                (Some(0), _) => "序章".to_owned(),
                (Some(ch), Some(name)) => format!("第 {ch} 章「{name}」"),
                (Some(ch), None) => format!("第 {ch} 章"),
                (None, _) => "章节未知".to_owned(),
            };
            let map_desc = current_map_id.map_or_else(
                || "地图未知".to_owned(),
                |id| format_map_label(id, current_map_name.as_deref()),
            );
            let lv_desc = play_level.map_or_else(String::new, |lv| format!("，Lv.{lv}"));
            let time_desc = play_time_text
                .as_deref()
                .map_or_else(String::new, |t| format!("，时长 {t}"));
            let party_names: Vec<String> = party_unit_ids
                .iter()
                .map(|&id| lookup.unit_name(id).unwrap_or_else(|| format!("#{id}")))
                .collect();
            let party_desc = if party_names.is_empty() {
                String::new()
            } else {
                format!("，队伍：{}", party_names.join("、"))
            };
            occupied_lines.push(format!(
                "- 槽 {slot_index}（{slot_kind}）：{ch_desc}，{map_desc}{lv_desc}{time_desc}{party_desc}"
            ));
        } else if slot_index >= 0 {
            empty_indices.push(slot_index.to_string());
        }

        slots_json.push(json!({
            "slot_index": slot_index,
            "is_auto_save": is_auto,
            "exists": exists,
            "load_status": load_status,
            "main_progress": main_progress,
            "chapter": chapter,
            "chapter_name": chapter_name,
            "current_map_id": current_map_id,
            "current_map_name": current_map_name,
            "play_level": play_level,
            "play_time": play_time,
            "play_time_text": play_time_text,
            "date_time_ticks": date_time_ticks,
            "difficulty": difficulty,
            "difficulty_zh": difficulty_zh,
            "is_cleared": is_cleared,
            "party_unit_ids": party_unit_ids,
            "party_members": party_members,
        }));
    }

    let total = slot_count.unwrap_or(slots_json.len() as i64);
    let last_desc = last_saved_slot.map_or_else(|| "无".to_owned(), |s| format!("槽 {s}"));
    let mut lines = vec![format!(
        "存档槽共 {total} 个（已占用 {} 个，空槽 {} 个；最近存档：{last_desc}）：",
        occupied_lines.len(),
        empty_indices.len()
    )];
    lines.extend(occupied_lines);
    if !empty_indices.is_empty() {
        lines.push(format!("- 空槽位：{}", empty_indices.join("、")));
    }

    ToolOutput::new(vec![
        Content::text(lines.join("\n")),
        Content::json(json!({
            "ok": true,
            "slot_count": total,
            "last_saved_slot": last_saved_slot,
            "occupied_count": slots_json.iter().filter(|s| s["exists"] == Value::Bool(true)).count(),
            "empty_count": empty_indices.len(),
            "slots": slots_json,
        })),
    ])
}

async fn handle_read_inventory(
    channel: &FileChannel,
    source: &GameDataSource,
    args: &Map<String, Value>,
    ctx: &CallContext,
) -> ToolOutput {
    if !args.is_empty() {
        return arg_error("工具 `read_inventory` 不接受参数，请传入空对象 {}".to_owned());
    }
    let result = match send_bridge_op(channel, "inventory", json!({}), ctx).await {
        Ok(val) => val,
        Err(output) => return output,
    };
    let Some(res) = result.as_object() else {
        return bridge_format_error("inventory", "回复 result 不是对象");
    };

    let lookup = LookupCtx::from_source(source, true, false, false);
    let count = obj_i64(res, "count");
    let total_qty = obj_i64(res, "total_qty");
    let hash = obj_str(res, "hash");
    let raw_items = obj_array(res, "items").unwrap_or(&[]);

    let mut items_json = Vec::with_capacity(raw_items.len());
    let mut item_lines = Vec::new();
    for (idx, raw) in raw_items.iter().enumerate() {
        let Some(entry) = raw.as_object() else {
            continue;
        };
        let Some(item_id) = obj_i64(entry, "item_id") else {
            continue;
        };
        let num = obj_i64(entry, "num").unwrap_or(0);
        let item_meta = lookup.item(item_id);
        let name = lookup.item_name(item_id);
        let category_zh = item_meta.map(|it| item_category_zh(it.category));

        if idx < INVENTORY_SUMMARY_LIMIT {
            let label = format_item_label(item_id, name.as_deref());
            let cat_part = category_zh.map_or_else(String::new, |c| format!(" [{c}]"));
            item_lines.push(format!("- {label}{cat_part} ×{num}"));
        }

        items_json.push(json!({
            "item_id": item_id,
            "name": name,
            "category_zh": category_zh,
            "num": num,
        }));
    }

    let eff_count = count.unwrap_or(items_json.len() as i64);
    let qty_part = total_qty.map_or_else(String::new, |q| format!("，总数量 {q}"));
    let hash_part = hash.map_or_else(String::new, |h| format!("，校验 {h}"));
    let mut lines = vec![format!(
        "物品栏共 {eff_count} 种道具{qty_part}{hash_part}："
    )];
    if item_lines.is_empty() {
        lines.push("- 物品栏为空。".to_owned());
    } else {
        lines.extend(item_lines);
        if items_json.len() > INVENTORY_SUMMARY_LIMIT {
            lines.push(format!(
                "- ……其余 {} 项详见结构化 JSON。",
                items_json.len() - INVENTORY_SUMMARY_LIMIT
            ));
        }
    }

    ToolOutput::new(vec![
        Content::text(lines.join("\n")),
        Content::json(json!({
            "ok": true,
            "count": eff_count,
            "total_qty": total_qty,
            "hash": hash,
            "items": items_json,
        })),
    ])
}

async fn handle_read_event_flags(
    channel: &FileChannel,
    source: &GameDataSource,
    args: &Map<String, Value>,
    ctx: &CallContext,
) -> ToolOutput {
    if let Err(msg) = reject_unknown_keys("read_event_flags", args, &["flag_ids"]) {
        return arg_error(msg);
    }
    let filter_ids = match parse_optional_nonneg_i64_array("read_event_flags", args, "flag_ids") {
        Ok(ids) => ids,
        Err(msg) => return arg_error(msg),
    };

    let result = match send_bridge_op(channel, "event_flags", json!({}), ctx).await {
        Ok(val) => val,
        Err(output) => return output,
    };
    let Some(res) = result.as_object() else {
        return bridge_format_error("event_flags", "回复 result 不是对象");
    };

    let total = obj_i64(res, "total");
    let true_count = obj_i64(res, "true_count");
    let hash = obj_str(res, "hash");
    let true_ids = extract_i64_list(res.get("true_ids"));
    let true_set: BTreeSet<i64> = true_ids.iter().copied().collect();

    let eff_true_count = true_count.unwrap_or(true_ids.len() as i64);
    let total_part = total.map_or_else(String::new, |t| format!("共 {t} 位，"));
    let hash_part = hash.map_or_else(String::new, |h| format!("（校验 {h}）"));
    let mut lines = vec![format!(
        "事件旗标：{total_part}已置位 {eff_true_count} 个{hash_part}"
    )];

    let checked_json = filter_ids.as_ref().map(|requested| {
        let mut checked = Vec::with_capacity(requested.len());
        let mut checked_labels = Vec::with_capacity(requested.len());
        for &flag_id in requested {
            let is_set = true_set.contains(&flag_id);
            checked_labels.push(format!("{flag_id}={}", if is_set { "开" } else { "关" }));
            checked.push(json!({
                "flag_id": flag_id,
                "is_set": is_set,
            }));
        }
        lines.push(format!("- 指定旗标核查：{}", checked_labels.join("、")));
        checked
    });

    if checked_json.is_none() {
        if true_ids.is_empty() {
            lines.push("- 当前没有已置位的事件旗标。".to_owned());
        } else {
            let shown: Vec<String> = true_ids
                .iter()
                .take(EVENT_FLAGS_SUMMARY_LIMIT)
                .map(ToString::to_string)
                .collect();
            let suffix = if true_ids.len() > EVENT_FLAGS_SUMMARY_LIMIT {
                format!("……等共 {} 个", true_ids.len())
            } else {
                String::new()
            };
            lines.push(format!("- 已置位编号：{}{suffix}", shown.join("、")));
        }
    }

    let lookup_ids: &[i64] = filter_ids.as_deref().unwrap_or(&true_ids);
    let related_conditions = match source {
        GameDataSource::Db(db_cache) => db_cache
            .open_with_availability()
            .ok()
            .filter(|(_, avail)| avail.flag_conditions)
            .map(|(db, _)| {
                let mut matched = BTreeMap::new();
                for &flag_id in lookup_ids {
                    if let Ok(entries) = db.flag_conditions_for_flag(flag_id) {
                        for entry in entries {
                            matched.entry(entry.row_key.clone()).or_insert(entry);
                        }
                    }
                }
                format_related_flag_conditions(matched.values(), &true_set, &mut lines)
            }),
        GameDataSource::Json(cache) => cache.flag_conditions().ok().map(|flag_data| {
            let mut matched = BTreeMap::new();
            for &flag_id in lookup_ids {
                for entry in flag_data.conditions_for_flag(flag_id) {
                    matched.entry(entry.row_key.as_str()).or_insert(entry);
                }
            }
            format_related_flag_conditions(matched.values().copied(), &true_set, &mut lines)
        }),
    };

    let mut payload = json!({
        "ok": true,
        "total": total,
        "true_count": eff_true_count,
        "hash": hash,
        "true_ids": true_ids,
    });
    if let Some(obj) = payload.as_object_mut() {
        if let Some(checked) = checked_json {
            obj.insert("checked".to_owned(), Value::Array(checked));
        }
        if let Some(conditions) = related_conditions {
            obj.insert("related_conditions".to_owned(), Value::Array(conditions));
        }
    }

    ToolOutput::new(vec![
        Content::text(lines.join("\n")),
        Content::json(payload),
    ])
}

fn format_related_flag_conditions<'a>(
    entries: impl ExactSizeIterator<Item = &'a FlagConditionEntry>,
    true_set: &BTreeSet<i64>,
    lines: &mut Vec<String>,
) -> Vec<Value> {
    let total = entries.len();
    let mut cond_json = Vec::with_capacity(total);
    let mut cond_labels = Vec::new();
    for entry in entries {
        let satisfied = entry.evaluate(|fid| true_set.contains(&fid));
        if cond_labels.len() < 16 {
            cond_labels.push(format!(
                "{}({})={}",
                entry.row_key,
                entry.condition,
                if satisfied { "满足" } else { "未满足" }
            ));
        }
        cond_json.push(json!({
            "name": entry.row_key,
            "condition": entry.condition,
            "flag_ids": entry.flag_ids(),
            "satisfied": satisfied,
        }));
    }
    if !cond_labels.is_empty() {
        let suffix = if total > cond_labels.len() {
            format!("……等共 {total} 项")
        } else {
            String::new()
        };
        lines.push(format!(
            "- 关联旗标条件：{}{suffix}",
            cond_labels.join("、")
        ));
    }
    cond_json
}

async fn handle_read_input_state(
    channel: &FileChannel,
    args: &Map<String, Value>,
    ctx: &CallContext,
) -> ToolOutput {
    if let Err(msg) = reject_unknown_keys("read_input_state", args, &["mapping_names", "slot"]) {
        return arg_error(msg);
    }

    let mut bridge_args = Map::new();
    if let Some(raw_names) = args.get("mapping_names")
        && !raw_names.is_null()
    {
        let Some(arr) = raw_names.as_array() else {
            return arg_error(
                "工具 `read_input_state` 的参数 `mapping_names` 必须是字符串数组".to_owned(),
            );
        };
        if arr.is_empty() {
            return arg_error(
                "工具 `read_input_state` 的参数 `mapping_names` 不能为空数组".to_owned(),
            );
        }
        let mut names = Vec::with_capacity(arr.len());
        for item in arr {
            let Some(s) = item.as_str().map(str::trim) else {
                return arg_error(
                    "工具 `read_input_state` 的参数 `mapping_names` 只能包含字符串".to_owned(),
                );
            };
            if s.is_empty() {
                return arg_error(
                    "工具 `read_input_state` 的参数 `mapping_names` 不能包含空字符串".to_owned(),
                );
            }
            names.push(Value::String(s.to_owned()));
        }
        bridge_args.insert("mapping_names".to_owned(), Value::Array(names));
    }

    if let Some(raw_slot) = args.get("slot")
        && !raw_slot.is_null()
    {
        let Some(slot) = raw_slot.as_i64() else {
            return arg_error("工具 `read_input_state` 的参数 `slot` 必须是非负整数".to_owned());
        };
        if slot < 0 {
            return arg_error("工具 `read_input_state` 的参数 `slot` 必须大于等于 0".to_owned());
        }
        bridge_args.insert("slot".to_owned(), json!(slot));
    }

    let result = match send_bridge_op(channel, "input_state", Value::Object(bridge_args), ctx).await
    {
        Ok(val) => val,
        Err(output) => return output,
    };
    let Some(res) = result.as_object() else {
        return bridge_format_error("input_state", "回复 result 不是对象");
    };

    let active_imcs = extract_str_list(res.get("active_imcs"));
    let swap_decide = obj_bool(res, "swap_decide");
    let input_enabled = obj_bool(res, "input_enabled");
    let input_blocked = extract_str_list(res.get("input_blocked"));
    let window_deactivated = input_blocked.iter().any(|t| t == "WindowDeactivate");
    let key_bindings = obj_object(res, "key_bindings").cloned().unwrap_or_default();

    let enabled_zh = opt_bool_zh(input_enabled, "开启", "关闭");
    let swap_zh = opt_bool_zh(swap_decide, "是", "否");
    let mut lines = vec![format!(
        "输入状态：输入总开关={enabled_zh}，确认/取消互换={swap_zh}"
    )];
    if input_blocked.is_empty() {
        lines.push("- 输入屏蔽：无".to_owned());
    } else {
        let focus_hint = if window_deactivated {
            "（窗口失焦，需先恢复焦点）"
        } else {
            ""
        };
        lines.push(format!(
            "- 输入屏蔽：{}{focus_hint}",
            input_blocked.join("、")
        ));
    }
    if active_imcs.is_empty() {
        lines.push("- 激活输入上下文（IMC）：无".to_owned());
    } else {
        lines.push(format!(
            "- 激活输入上下文（IMC）：{}",
            active_imcs.join("、")
        ));
    }
    if !key_bindings.is_empty() {
        let mut pairs: Vec<(&String, &Value)> = key_bindings.iter().collect();
        pairs.sort_by_key(|(a, _)| *a);
        let formatted: Vec<String> = pairs
            .into_iter()
            .map(|(k, v)| match v.as_str() {
                Some(key) => format!("{k}={key}"),
                None => format!("{k}=未绑定"),
            })
            .collect();
        lines.push(format!("- 按键绑定：{}", formatted.join("，")));
    }

    ToolOutput::new(vec![
        Content::text(lines.join("\n")),
        Content::json(json!({
            "ok": true,
            "active_imcs": active_imcs,
            "swap_decide": swap_decide,
            "input_enabled": input_enabled,
            "input_blocked": input_blocked,
            "window_deactivated": window_deactivated,
            "key_bindings": Value::Object(key_bindings),
        })),
    ])
}

async fn handle_query_nav_path(
    channel: &FileChannel,
    args: &Map<String, Value>,
    ctx: &CallContext,
) -> ToolOutput {
    if let Err(msg) = reject_unknown_keys("query_nav_path", args, &["start", "goal"]) {
        return arg_error(msg);
    }
    let start = match parse_point_arg("query_nav_path", args, "start") {
        Ok(pt) => pt,
        Err(msg) => return arg_error(msg),
    };
    let goal = match parse_point_arg("query_nav_path", args, "goal") {
        Ok(pt) => pt,
        Err(msg) => return arg_error(msg),
    };

    let bridge_args = json!({
        "start": { "x": start.0, "y": start.1, "z": start.2 },
        "goal": { "x": goal.0, "y": goal.1, "z": goal.2 },
    });
    let result = match send_bridge_op(channel, "query_nav_path", bridge_args, ctx).await {
        Ok(val) => val,
        Err(output) => return output,
    };
    let Some(res) = result.as_object() else {
        return bridge_format_error("query_nav_path", "回复 result 不是对象");
    };

    let valid = obj_bool(res, "valid").unwrap_or(false);
    let partial = obj_bool(res, "partial").unwrap_or(false);
    let length = obj_f64(res, "length").unwrap_or(0.0);
    let raw_points = obj_array(res, "points").unwrap_or(&[]);
    let points_json: Vec<Value> = raw_points
        .iter()
        .filter_map(|p| format_location(p.as_object()).map(|(_, v)| v))
        .collect();

    let start_str = format!("({:.1}, {:.1}, {:.1})", start.0, start.1, start.2);
    let goal_str = format!("({:.1}, {:.1}, {:.1})", goal.0, goal.1, goal.2);
    let summary = if valid && !partial {
        format!(
            "导航路径可达：起点 {start_str} → 终点 {goal_str}，路径全长 {length:.1}，共 {} 个路点。",
            points_json.len()
        )
    } else if valid && partial {
        format!(
            "导航路径仅部分可达：起点 {start_str} → 终点 {goal_str}，已规划长度 {length:.1}，共 {} 个路点（未能完整抵达终点）。",
            points_json.len()
        )
    } else {
        format!("导航路径不可达：无法从 {start_str} 寻路到 {goal_str}。")
    };

    ToolOutput::new(vec![
        Content::text(summary),
        Content::json(json!({
            "ok": true,
            "valid": valid,
            "partial": partial,
            "length": length,
            "point_count": points_json.len(),
            "start": { "x": start.0, "y": start.1, "z": start.2 },
            "goal": { "x": goal.0, "y": goal.1, "z": goal.2 },
            "points": points_json,
        })),
    ])
}

async fn send_bridge_op(
    channel: &FileChannel,
    op: &str,
    args: Value,
    ctx: &CallContext,
) -> Result<Value, ToolOutput> {
    let ack = channel
        .send(op, args, &ctx.cancellation)
        .await
        .map_err(|error| {
            let mut output = ToolOutput::new(vec![
                Content::text(format!("执行桥命令 `{op}` 失败：{error}")),
                Content::json(json!({
                    "ok": false,
                    "op": op,
                    "error": error.to_string(),
                })),
            ]);
            output.is_error = true;
            output
        })?;

    if !ack.ok {
        let reason = ack.error.unwrap_or_else(|| "模组未返回错误原因".to_owned());
        let mut output = ToolOutput::new(vec![
            Content::text(format!("游戏内模组执行 `{op}` 失败：{reason}")),
            Content::json(json!({
                "ok": false,
                "op": op,
                "error": reason,
            })),
        ]);
        output.is_error = true;
        return Err(output);
    }

    ack.result
        .ok_or_else(|| bridge_format_error(op, "模组返回 ok=true 但缺少 result 字段"))
}

fn bridge_format_error(op: &str, reason: &str) -> ToolOutput {
    let message = format!("桥命令 `{op}` 回复格式异常：{reason}");
    let mut output = ToolOutput::new(vec![
        Content::text(&message),
        Content::json(json!({
            "ok": false,
            "op": op,
            "error": message,
        })),
    ]);
    output.is_error = true;
    output
}

fn arg_error(message: String) -> ToolOutput {
    let mut output = ToolOutput::new(vec![Content::text(message)]);
    output.is_error = true;
    output
}

fn reject_unknown_keys(
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

fn parse_optional_nonneg_i64_array(
    tool: &str,
    args: &Map<String, Value>,
    key: &str,
) -> Result<Option<Vec<i64>>, String> {
    let Some(raw) = args.get(key) else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let Some(arr) = raw.as_array() else {
        return Err(format!("工具 `{tool}` 的参数 `{key}` 必须是整数数组"));
    };
    if arr.is_empty() {
        return Err(format!(
            "工具 `{tool}` 的参数 `{key}` 不能为空数组，请至少提供一个编号或省略该参数"
        ));
    }
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(id) = item.as_i64() else {
            return Err(format!("工具 `{tool}` 的参数 `{key}` 只能包含非负整数"));
        };
        if id < 0 {
            return Err(format!("工具 `{tool}` 的参数 `{key}` 不能包含负数：{id}"));
        }
        if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(Some(out))
}

fn parse_point_arg(
    tool: &str,
    args: &Map<String, Value>,
    key: &str,
) -> Result<(f64, f64, f64), String> {
    let Some(obj) = args.get(key).and_then(Value::as_object) else {
        return Err(format!(
            "工具 `{tool}` 缺少必需坐标对象 `{key}`（需包含 x、y、z）"
        ));
    };
    reject_unknown_keys(tool, obj, &["x", "y", "z"])?;
    let x =
        obj_f64(obj, "x").ok_or_else(|| format!("工具 `{tool}` 的 `{key}.x` 必须是有限数字"))?;
    let y =
        obj_f64(obj, "y").ok_or_else(|| format!("工具 `{tool}` 的 `{key}.y` 必须是有限数字"))?;
    let z =
        obj_f64(obj, "z").ok_or_else(|| format!("工具 `{tool}` 的 `{key}.z` 必须是有限数字"))?;
    Ok((x, y, z))
}

fn difficulty_label_zh(code: i64) -> &'static str {
    match code {
        0 => "简单",
        1 => "普通",
        2 => "困难",
        _ => "未知",
    }
}

fn opt_bool_zh(value: Option<bool>, yes: &'static str, no: &'static str) -> &'static str {
    match value {
        Some(true) => yes,
        Some(false) => no,
        None => "未知",
    }
}
