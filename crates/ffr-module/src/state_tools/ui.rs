//! UI topic formatter for `read_game_state`.

use serde_json::{Map, Value, json};

use crate::state_tools::{
    LookupCtx, SectionOutput, extract_i64_list, extract_str_list, obj_array, obj_bool, obj_i64,
    obj_object, obj_str,
};

/// Render the `ui` section from `data.ui`.
pub(crate) fn render_ui(ui: Option<&Map<String, Value>>, lookup: &LookupCtx) -> SectionOutput {
    let Some(ui) = ui else {
        return SectionOutput {
            summary: "【界面】桥未提供 v1 ui 主题。".to_owned(),
            json: json!({ "available": false }),
        };
    };

    let screen_kind = obj_str(ui, "screen_kind").unwrap_or("unknown");
    let screen_kind_zh = screen_kind_label_zh(screen_kind);
    let active_layer = obj_str(ui, "active_layer");
    let active_widget = obj_str(ui, "active_widget");
    let action_guide_tag = obj_str(ui, "action_guide_tag");
    let input_blocked = extract_str_list(ui.get("input_blocked"));
    let window_deactivated = input_blocked.iter().any(|t| t == "WindowDeactivate");

    let mut lines = vec![format!("【界面】{screen_kind_zh}（{screen_kind}）")];
    if !input_blocked.is_empty() {
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

    let movie_json = obj_object(ui, "movie").map(|m| {
        let active = obj_bool(m, "active").unwrap_or(true);
        let subtitle = obj_str(m, "subtitle");
        match subtitle {
            Some(sub) => lines.push(format!("- 过场影片字幕：{sub}")),
            None => lines.push("- 过场影片播放中（无字幕）".to_owned()),
        }
        json!({ "active": active, "subtitle": subtitle })
    });

    let talk_json = obj_object(ui, "talk").map(|t| {
        let event_id = obj_str(t, "event_id");
        let window_class = obj_str(t, "window_class");
        let is_cutscene = obj_bool(t, "is_cutscene").unwrap_or(false);
        let speaker = obj_str(t, "speaker");
        let text = obj_str(t, "text");
        let waiting_key = obj_bool(t, "waiting_key").unwrap_or(false);
        let text_finished = obj_bool(t, "text_finished");
        let can_skip = obj_bool(t, "can_skip").unwrap_or(false);
        let is_auto = obj_bool(t, "is_auto").unwrap_or(false);

        let wait_label = if is_cutscene {
            "过场演出"
        } else if waiting_key {
            "等待按键"
        } else {
            "文字播放中"
        };
        let speaker_part = speaker.map_or_else(String::new, |s| format!("说话人「{s}」，"));
        let text_part = text.map_or_else(String::new, |s| format!("内容：「{s}」"));
        lines.push(format!("- 对话：{speaker_part}{wait_label}；{text_part}"));
        if let Some(ev) = event_id {
            lines.push(format!(
                "- 事件：{ev}（可跳过={}，自动播放={}）",
                yes_no(can_skip),
                yes_no(is_auto)
            ));
        }

        json!({
            "event_id": event_id,
            "window_class": window_class,
            "is_cutscene": is_cutscene,
            "speaker": speaker,
            "text": text,
            "waiting_key": waiting_key,
            "text_finished": text_finished,
            "can_skip": can_skip,
            "is_auto": is_auto,
        })
    });

    let choice_json = obj_object(ui, "choice").map(|c| {
        let source = obj_str(c, "source");
        let dialog_tag = obj_str(c, "dialog_tag");
        let choice_tag = obj_str(c, "choice_tag");
        let message = obj_str(c, "message").unwrap_or_default();
        let options = extract_str_list(c.get("options"));
        let focus_index = obj_i64(c, "focus_index");
        let focused_option = focus_index
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| options.get(i))
            .cloned();

        let kind_label = if source == Some("event_choice") {
            "剧情选项"
        } else {
            "系统弹窗"
        };
        let focus_text = focused_option
            .as_deref()
            .map_or_else(|| "无".to_owned(), |o| format!("「{o}」"));
        lines.push(format!(
            "- {kind_label}：{message}（光标：{focus_text}，选项：{}）",
            options.join(" / ")
        ));

        json!({
            "source": source,
            "dialog_tag": dialog_tag,
            "choice_tag": choice_tag,
            "message": message,
            "options": options,
            "focus_index": focus_index,
            "focused_option": focused_option,
        })
    });

    let tutorial_json = obj_object(ui, "tutorial").map(|t| {
        let name = obj_str(t, "name");
        let page_index = obj_i64(t, "page_index").unwrap_or(0);
        let page_count = obj_i64(t, "page_count").unwrap_or(1);
        let caption = obj_str(t, "caption").unwrap_or_default();
        let text = obj_str(t, "text").unwrap_or_default();
        let can_close = obj_bool(t, "can_close").unwrap_or(false);
        let close_label = if can_close { "可关闭" } else { "需翻页" };
        lines.push(format!(
            "- 教学「{caption}」（第 {}/{page_count} 页，{close_label}）：{text}",
            page_index + 1
        ));
        json!({
            "name": name,
            "page_index": page_index,
            "page_number": page_index + 1,
            "page_count": page_count,
            "caption": caption,
            "text": text,
            "can_close": can_close,
        })
    });

    let menu_json = obj_object(ui, "menu").map(|m| {
        let kind = obj_str(m, "kind").unwrap_or("menu");
        let scene_stack = extract_str_list(m.get("scene_stack"));
        let entries = extract_str_list(m.get("entries"));
        let focus_index = obj_i64(m, "focus_index");
        let focused_entry = focus_index
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| entries.get(i))
            .cloned();
        let sub_state = obj_object(m, "sub_state").map(|sub| enrich_menu_sub_state(sub, lookup));

        let focus_part = focused_entry
            .as_deref()
            .map_or_else(|| "未知".to_owned(), |e| format!("「{e}」"));
        let stack_part = if scene_stack.is_empty() {
            String::new()
        } else {
            format!("，页面栈：{}", scene_stack.join(" > "))
        };
        lines.push(format!(
            "- 菜单（{}）：光标在 {focus_part}{stack_part}",
            menu_kind_zh(kind)
        ));
        if !entries.is_empty() {
            lines.push(format!("- 可选项：{}", entries.join("、")));
        }

        json!({
            "kind": kind,
            "kind_zh": menu_kind_zh(kind),
            "scene_stack": scene_stack,
            "entries": entries,
            "focus_index": focus_index,
            "focused_entry": focused_entry,
            "sub_state": sub_state,
        })
    });

    let shop_json = obj_object(ui, "shop").map(|s| {
        let (shop_lines, val) = format_shop(s, lookup);
        lines.extend(shop_lines);
        val
    });

    let save_ui_json = obj_object(ui, "save_ui").map(|su| {
        let mode = obj_str(su, "mode").unwrap_or("save");
        let mode_zh = if mode == "load" { "读档" } else { "存档" };
        let slot_indices = extract_i64_list(su.get("slot_indices"));
        let focus_index = obj_i64(su, "focus_index");
        let focused_slot = focus_index
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| slot_indices.get(i))
            .copied();
        let slot_desc =
            focused_slot.map_or_else(|| "未知槽位".to_owned(), |slot| format!("槽位 {slot}"));
        lines.push(format!(
            "- {mode_zh}界面：光标在 {slot_desc}（可见槽位共 {} 个）",
            slot_indices.len()
        ));
        json!({
            "mode": mode,
            "mode_zh": mode_zh,
            "slot_indices": slot_indices,
            "focus_index": focus_index,
            "focused_slot": focused_slot,
        })
    });

    let battle_result_json = obj_object(ui, "battle_result").map(|br| {
        let page_index = obj_i64(br, "page_index");
        let has_level_up = obj_bool(br, "has_level_up_window").unwrap_or(false);
        let page_name = match (page_index, has_level_up) {
            (Some(0), _) => "报酬页",
            (Some(1), true) => "升级/习得技能页",
            (Some(1), false) => "成长页",
            _ => "结算页",
        };
        lines.push(format!("- 战斗结算界面：{page_name}"));
        json!({
            "page_index": page_index,
            "page_name": page_name,
            "has_level_up_window": has_level_up,
        })
    });

    let payload = json!({
        "screen_kind": screen_kind,
        "screen_kind_zh": screen_kind_zh,
        "active_layer": active_layer,
        "active_widget": active_widget,
        "action_guide_tag": action_guide_tag,
        "input_blocked": input_blocked,
        "window_deactivated": window_deactivated,
        "movie": movie_json,
        "talk": talk_json,
        "choice": choice_json,
        "tutorial": tutorial_json,
        "menu": menu_json,
        "shop": shop_json,
        "save_ui": save_ui_json,
        "battle_result": battle_result_json,
    });

    SectionOutput {
        summary: lines.join("\n"),
        json: payload,
    }
}

fn format_shop(shop: &Map<String, Value>, lookup: &LookupCtx) -> (Vec<String>, Value) {
    let shop_id = obj_i64(shop, "shop_id").filter(|&id| id >= 0);
    let shop_name = shop_id.and_then(|id| lookup.shop_name(id));
    let shop_type = obj_str(shop, "shop_type");
    let shop_type_zh = shop_type.map(shop_type_label_zh);
    let scene_stack = extract_str_list(shop.get("scene_stack"));
    let is_buy = obj_bool(shop, "is_buy");
    let mode_zh = match is_buy {
        Some(true) => "购买",
        Some(false) => "出售",
        None => "菜单选择",
    };
    let category_index = obj_i64(shop, "category_index");
    let categories = extract_str_list(shop.get("categories"));
    let cmd_focus_index = obj_i64(shop, "cmd_focus_index");
    let item_focus_index = obj_i64(shop, "item_focus_index");
    let tooltip_item_id = obj_i64(shop, "tooltip_item_id").filter(|&id| id >= 0);
    let tooltip_item_name = tooltip_item_id.and_then(|id| lookup.item_name(id));
    let total_amount = obj_i64(shop, "total_amount");

    let raw_items = obj_array(shop, "items").unwrap_or(&[]);
    let mut items_json = Vec::with_capacity(raw_items.len());
    let mut item_lines = Vec::new();
    for (idx, raw) in raw_items.iter().enumerate() {
        let Some(item) = raw.as_object() else {
            continue;
        };
        let id = obj_i64(item, "id");
        let runtime_name = obj_str(item, "name");
        let name = id
            .and_then(|iid| lookup.item_name(iid))
            .or_else(|| runtime_name.map(str::to_owned));
        let price = obj_i64(item, "price");
        let owned = obj_i64(item, "owned");
        let stock = obj_i64(item, "stock");
        let limit = obj_i64(item, "limit");

        let cursor = if item_focus_index == Some(idx as i64) {
            "👉 "
        } else {
            ""
        };
        let name_display = name.as_deref().unwrap_or("未知道具");
        let id_part = id.map_or_else(String::new, |iid| format!("（#{iid}）"));
        let price_part = price.map_or_else(|| "价格未知".to_owned(), |p| format!("{p}G"));
        let owned_part = owned.map_or_else(String::new, |o| format!("，持有 {o}"));
        let limit_part = match (stock, limit) {
            (Some(st), Some(lm)) if lm > 0 => format!("，剩余限购 {st}/{lm}"),
            _ => String::new(),
        };
        item_lines.push(format!(
            "  - {cursor}{name_display}{id_part}：{price_part}{owned_part}{limit_part}"
        ));

        items_json.push(json!({
            "id": id,
            "name": name,
            "price": price,
            "owned": owned,
            "stock": stock,
            "limit": limit,
        }));
    }

    let shop_title = match (shop_id, shop_name.as_deref(), shop_type_zh) {
        (Some(id), Some(name), Some(kind)) => format!("商店 #{id}「{name}」（{kind}）"),
        (Some(id), Some(name), None) => format!("商店 #{id}「{name}」"),
        (Some(id), None, Some(kind)) => format!("商店 #{id}（{kind}）"),
        (Some(id), None, None) => format!("商店 #{id}"),
        (None, _, Some(kind)) => kind.to_owned(),
        (None, _, None) => "商店".to_owned(),
    };
    let focused_item_desc = item_focus_index
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| items_json.get(i))
        .and_then(|item| item["name"].as_str())
        .or(tooltip_item_name.as_deref());

    let mut lines = vec![format!("- {shop_title}，模式：{mode_zh}")];
    if let Some(focus) = focused_item_desc {
        lines.push(format!("- 当前光标商品：「{focus}」"));
    }
    if !item_lines.is_empty() {
        lines.push(format!("- 当前页商品（共 {} 件）：", item_lines.len()));
        lines.extend(item_lines);
    }

    let val = json!({
        "shop_id": shop_id,
        "shop_name": shop_name,
        "shop_type": shop_type,
        "shop_type_zh": shop_type_zh,
        "scene_stack": scene_stack,
        "is_buy": is_buy,
        "mode_zh": mode_zh,
        "category_index": category_index,
        "categories": categories,
        "cmd_focus_index": cmd_focus_index,
        "item_focus_index": item_focus_index,
        "tooltip_item_id": tooltip_item_id,
        "tooltip_item_name": tooltip_item_name,
        "total_amount": total_amount,
        "items": items_json,
    });
    (lines, val)
}

fn enrich_menu_sub_state(sub: &Map<String, Value>, lookup: &LookupCtx) -> Value {
    let mut out = sub.clone();
    if let Some(uid) = obj_i64(sub, "unit_id").filter(|&id| id >= 0) {
        out.insert("unit_name".to_owned(), json!(lookup.unit_name(uid)));
    }
    if let Some(uid) = obj_i64(sub, "target_unit_id").filter(|&id| id >= 0) {
        out.insert("target_unit_name".to_owned(), json!(lookup.unit_name(uid)));
    }
    if let Some(qid) = obj_i64(sub, "focus_quest_id").filter(|&id| id >= 0)
        && let Some(qname) = lookup
            .main_quest_title(qid)
            .or_else(|| lookup.quest_name(qid))
    {
        out.insert("focus_quest_name".to_owned(), json!(qname));
    }
    if let Some(actions) = obj_array(sub, "actions") {
        let enriched: Vec<Value> = actions
            .iter()
            .map(|raw| {
                let Some(obj) = raw.as_object() else {
                    return raw.clone();
                };
                let mut item = obj.clone();
                let cat = obj_str(obj, "category");
                if let Some(id) = obj_i64(obj, "id") {
                    let name = match cat {
                        Some("Item") => lookup.item_name(id),
                        Some("Skill") => lookup.skill_name(id),
                        _ => lookup.skill_name(id).or_else(|| lookup.item_name(id)),
                    };
                    item.insert("name".to_owned(), json!(name));
                }
                Value::Object(item)
            })
            .collect();
        out.insert("actions".to_owned(), Value::Array(enriched));
    }
    Value::Object(out)
}

pub(crate) fn screen_kind_label_zh(kind: &str) -> &'static str {
    match kind {
        "loading" => "载入中",
        "movie" => "过场影片",
        "tutorial" => "教学弹窗",
        "dialog" => "系统弹窗",
        "choice" => "剧情选项",
        "talk" => "对话中",
        "cutscene" => "过场演出",
        "save_ui" => "存读档界面",
        "system_menu" => "系统菜单",
        "shop" => "商店界面",
        "savepoint" => "存档点菜单",
        "main_menu" => "主菜单",
        "battle_result" => "战斗结算",
        "battle" => "战斗中",
        "title" => "标题画面",
        "field" => "野外探索",
        _ => "未知画面",
    }
}

pub(crate) fn menu_kind_zh(kind: &str) -> &'static str {
    match kind {
        "system_menu" => "系统菜单",
        "main_menu" => "主菜单",
        "savepoint" => "存档点菜单",
        "title" => "标题菜单",
        _ => "菜单",
    }
}

pub(crate) fn shop_type_label_zh(raw: &str) -> &'static str {
    match raw {
        "ToolShop" => "道具店",
        "WeaponShop" => "武器店",
        "ArmorShop" => "防具店",
        "AccessoryShop" => "饰品店",
        "VariousShops" => "综合商店",
        "Other" => "商店",
        _ => "商店",
    }
}

fn yes_no(flag: bool) -> &'static str {
    if flag { "是" } else { "否" }
}
