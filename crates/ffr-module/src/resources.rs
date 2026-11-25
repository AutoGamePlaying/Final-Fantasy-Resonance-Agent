//! Read-only paginated JSON resources exposed via `resources/list` and `resources/read`.

use agp_game::{Content, GameError, ResourceDescriptor, ResourcePage, ResourceRange, Result};
use ffr_data::{MODEL_LANGUAGE, main_quest_id_for_chapter};
use serde_json::{Value, json};

use crate::data_cache::GameDataCache;
use crate::data_tools::{
    available_location_code, collapse_ws, cost_type_code, damage_type_code, element_zh,
    elements_zh_vec, format_skill_target, resolve_zh_clean,
};

pub const URI_CHAPTERS: &str = "ffr://chapters";
pub const URI_MAPS: &str = "ffr://maps";
pub const URI_ENEMIES: &str = "ffr://enemies";
pub const URI_SKILLS: &str = "ffr://skills";

/// List all read-only catalog resources exposed by `ffr-module`.
pub fn list_resources(cache: &GameDataCache) -> Vec<ResourceDescriptor> {
    let entries = [
        (
            URI_CHAPTERS,
            "章节与主线表",
            "全部章节名称与主线剧情条目（含目的地地图编号），紧凑 JSON 格式。",
        ),
        (
            URI_MAPS,
            "地图一览表",
            "全部地图编号、中文名、关卡名、类型、传送能力与宝箱数量，紧凑 JSON 格式。",
        ),
        (
            URI_ENEMIES,
            "敌人一览表",
            "全部敌方单位的编号、中文名、HP、破防槽、弱点、抗性与是否 Boss，紧凑 JSON 格式。支持按章节筛选：ffr://enemies?chapter=N（N>=1）。",
        ),
        (
            URI_SKILLS,
            "技能一览表",
            "全部技能的编号、中文名、属性、威力、破防削减、消耗与目标，紧凑 JSON 格式。",
        ),
    ];

    entries
        .into_iter()
        .map(|(uri, name, description)| {
            let size_hint = cache
                .require_data_dir()
                .ok()
                .and_then(|_| get_resource_body(cache, uri).ok())
                .map(|body| body.len() as u64);
            ResourceDescriptor {
                uri: uri.to_owned(),
                name: name.to_owned(),
                description: description.to_owned(),
                mime_type: "application/json".to_owned(),
                size_hint,
            }
        })
        .collect()
}

/// Read one UTF-8-safe byte page from a read-only resource.
pub fn read_resource(
    cache: &GameDataCache,
    uri: &str,
    range: ResourceRange,
) -> Result<ResourcePage> {
    if let Some(query) = uri.strip_prefix("ffr://enemies?") {
        let chapter_no = parse_enemies_chapter_query(query)?;
        let cache_key = format!("{URI_ENEMIES}?chapter={chapter_no}");
        let body = cache
            .get_or_build_resource(&cache_key, |c| {
                build_enemies_for_chapter_json(c, chapter_no)
            })
            .map_err(|message| GameError::InvalidMessage {
                operation: "读取资源".to_owned(),
                message,
            })?;
        return slice_utf8_page(uri, body.as_str(), range);
    }

    if !matches!(uri, URI_CHAPTERS | URI_MAPS | URI_ENEMIES | URI_SKILLS) {
        return Err(GameError::InvalidMessage {
            operation: "读取资源".to_owned(),
            message: format!("没有这个资源：{uri}"),
        });
    }

    let body = get_resource_body(cache, uri).map_err(|message| GameError::InvalidMessage {
        operation: "读取资源".to_owned(),
        message,
    })?;

    slice_utf8_page(uri, body.as_str(), range)
}

fn get_resource_body(
    cache: &GameDataCache,
    uri: &str,
) -> std::result::Result<std::sync::Arc<String>, String> {
    match uri {
        URI_CHAPTERS => cache.get_or_build_resource(URI_CHAPTERS, build_chapters_json),
        URI_MAPS => cache.get_or_build_resource(URI_MAPS, build_maps_json),
        URI_ENEMIES => cache.get_or_build_resource(URI_ENEMIES, build_enemies_json),
        URI_SKILLS => cache.get_or_build_resource(URI_SKILLS, build_skills_json),
        _ => Err(format!("没有这个资源：{uri}")),
    }
}

fn slice_utf8_page(uri: &str, body: &str, range: ResourceRange) -> Result<ResourcePage> {
    let total = body.len() as u64;
    if range.offset >= total {
        return Ok(ResourcePage {
            uri: uri.to_owned(),
            content: Content::text(""),
            offset: range.offset,
            next_offset: None,
            total_size: Some(total),
        });
    }

    if range.length == 0 {
        return Err(GameError::InvalidMessage {
            operation: "读取资源".to_owned(),
            message: "读取资源的 length 必须大于 0".to_owned(),
        });
    }

    let start = usize::try_from(range.offset).map_err(|_| GameError::InvalidMessage {
        operation: "读取资源".to_owned(),
        message: format!("资源偏移超出范围：{}", range.offset),
    })?;
    if !body.is_char_boundary(start) {
        return Err(GameError::InvalidMessage {
            operation: "读取资源".to_owned(),
            message: format!(
                "偏移 {} 落在多字节 UTF-8 字符中间，请使用上一页返回的 next_offset",
                range.offset
            ),
        });
    }

    let length = usize::try_from(range.length).unwrap_or(usize::MAX);
    let mut end = start.saturating_add(length).min(body.len());
    while end > start && !body.is_char_boundary(end) {
        end -= 1;
    }
    if end == start {
        return Err(GameError::InvalidMessage {
            operation: "读取资源".to_owned(),
            message: format!(
                "读取长度 {} 字节不足以容纳偏移 {} 处的完整 UTF-8 字符，请增大 length（至少 4 字节）",
                range.length, range.offset
            ),
        });
    }

    let next_offset = (end < body.len()).then_some(end as u64);
    Ok(ResourcePage {
        uri: uri.to_owned(),
        content: Content::text(&body[start..end]),
        offset: range.offset,
        next_offset,
        total_size: Some(total),
    })
}

fn build_chapters_json(cache: &GameDataCache) -> std::result::Result<String, String> {
    let scenario = cache.scenario()?;
    let mut chapters = Vec::with_capacity(scenario.chapters.len());
    for ch in &scenario.chapters {
        let name = scenario
            .chapter_name(ch.chapter_no)
            .text
            .and_then(|t| collapse_ws(&t));
        let main_scenario: Vec<Value> = scenario
            .chapter_main_scenario(ch.chapter_no)
            .into_iter()
            .map(|entry| {
                let texts = scenario.resolve_main_scenario_texts(entry);
                let text = texts.text.text.and_then(|t| collapse_ws(&t));
                let summary = texts.summary_text.text.and_then(|t| collapse_ws(&t));
                let destination_map_ids: Vec<i64> = entry
                    .destinations
                    .iter()
                    .map(|dest| dest.destination_id)
                    .collect();
                json!({
                    "row_key": entry.row_key,
                    "min_progress": entry.min_progress,
                    "text": text,
                    "summary": summary,
                    "destination_map_ids": destination_map_ids,
                })
            })
            .collect();
        chapters.push(json!({
            "chapter_no": ch.chapter_no,
            "main_quest_id": main_quest_id_for_chapter(ch.chapter_no),
            "row_key": ch.row_key,
            "name": name,
            "main_scenario": main_scenario,
        }));
    }
    serde_json::to_string(&json!({ "chapters": chapters }))
        .map_err(|error| format!("序列化章节资源失败：{error}"))
}

fn build_maps_json(cache: &GameDataCache) -> std::result::Result<String, String> {
    let world = cache.world()?;
    let maps: Vec<Value> = world
        .maps()
        .iter()
        .map(|map| {
            let name = resolve_zh_clean(world.text_resolver(), &map.name);
            let facilities: Vec<&str> = map
                .facilities
                .iter()
                .map(|fac| fac.interact_type.as_str())
                .collect();
            json!({
                "map_id": map.id,
                "name": name,
                "level_name": map.level_name,
                "map_type": map.map_type,
                "island": map.island,
                "danger_level": map.danger_level,
                "teleportable": world.is_teleport_destination(map.id),
                "treasure_count": world.treasure_boxes_in_map(map.id).len(),
                "facilities": facilities,
            })
        })
        .collect();
    serde_json::to_string(&json!({ "maps": maps }))
        .map_err(|error| format!("序列化地图资源失败：{error}"))
}

fn parse_enemies_chapter_query(query: &str) -> Result<i64> {
    let invalid = |msg: String| GameError::InvalidMessage {
        operation: "读取资源".to_owned(),
        message: msg,
    };
    let Some(value) = query.strip_prefix("chapter=") else {
        return Err(invalid(format!(
            "不支持的敌人筛选查询 `{query}`，请使用 `ffr://enemies?chapter=N`（N 为大于等于 1 的章节号）"
        )));
    };
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid(format!(
            "敌人筛选参数 `chapter={value}` 无效，请传入大于等于 1 的整数章节号"
        )));
    }
    let chapter_no = value
        .parse::<i64>()
        .map_err(|_| invalid(format!("敌人筛选参数 `chapter={value}` 超出整数范围")))?;
    if chapter_no < 1 {
        return Err(invalid("敌人筛选参数 `chapter` 必须大于等于 1".to_owned()));
    }
    Ok(chapter_no)
}

fn format_enemy_catalog_entry(
    battle: &ffr_data::battle::BattleData,
    profile: &ffr_data::battle::EnemyProfile,
    chapters: Option<&[i64]>,
) -> Value {
    let name = battle
        .unit_name_in(profile.unit_id, MODEL_LANGUAGE)
        .text
        .and_then(|t| collapse_ws(&t));
    json!({
        "unit_id": profile.unit_id,
        "name": name,
        "level": profile.level,
        "is_boss": profile.is_boss,
        "max_hp": profile.max_hp,
        "max_break_point": profile.max_break_point,
        "weaknesses": elements_zh_vec(&profile.weaknesses),
        "halved": elements_zh_vec(&profile.halved),
        "invalid": elements_zh_vec(&profile.invalid),
        "absorb": elements_zh_vec(&profile.absorb),
        "high_risk_skill_ids": profile.high_risk_skill_ids,
        "chapters": chapters,
    })
}

fn build_enemies_json(cache: &GameDataCache) -> std::result::Result<String, String> {
    let battle = cache.battle()?;
    let chapter_index = cache.enemy_chapters().ok();
    let mut enemies = Vec::new();
    for unit in &battle.units {
        if !unit.category.is_enemy_category() {
            continue;
        }
        let profile = unit.to_enemy_profile(battle.unit_ai_by_id(unit.id));
        let chapters = chapter_index
            .as_deref()
            .map(|idx| idx.chapters_for_enemy(profile.unit_id));
        enemies.push(format_enemy_catalog_entry(&battle, &profile, chapters));
    }
    serde_json::to_string(&json!({ "enemies": enemies }))
        .map_err(|error| format!("序列化敌人资源失败：{error}"))
}

fn build_enemies_for_chapter_json(
    cache: &GameDataCache,
    chapter_no: i64,
) -> std::result::Result<String, String> {
    let battle = cache.battle()?;
    let index = cache.enemy_chapters()?;
    let profiles = index.enemy_profiles_in_chapter(&battle, chapter_no);
    let enemies: Vec<Value> = profiles
        .iter()
        .map(|profile| {
            format_enemy_catalog_entry(
                &battle,
                profile,
                Some(index.chapters_for_enemy(profile.unit_id)),
            )
        })
        .collect();
    serde_json::to_string(&json!({
        "chapter": chapter_no,
        "enemies": enemies,
        "note": "章节归属由区域遇敌表、章节主/支线剧情战与陷阱宝箱静态推导，不含斗技场/强敌等无章节归属战斗。",
    }))
    .map_err(|error| format!("序列化第 {chapter_no} 章敌人资源失败：{error}"))
}

fn build_skills_json(cache: &GameDataCache) -> std::result::Result<String, String> {
    let battle = cache.battle()?;
    let skills: Vec<Value> = battle
        .skills
        .iter()
        .map(|skill| {
            let name = battle
                .skill_name_in(skill.id, MODEL_LANGUAGE)
                .text
                .and_then(|t| collapse_ws(&t));
            json!({
                "skill_id": skill.id,
                "name": name,
                "element": element_zh(skill.element),
                "damage_type": damage_type_code(skill.damage_type),
                "power": skill.power,
                "break_damage": skill.break_damage,
                "cost": skill.cost,
                "cost_type": cost_type_code(skill.cost_type),
                "target": format_skill_target(
                    skill.default_target_relation,
                    skill.target_type,
                    skill.default_target_state,
                ),
                "available_location": available_location_code(skill.available_location),
            })
        })
        .collect();
    serde_json::to_string(&json!({ "skills": skills }))
        .map_err(|error| format!("序列化技能资源失败：{error}"))
}
