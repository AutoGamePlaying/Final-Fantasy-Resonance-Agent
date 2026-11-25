//! Read-only paginated JSON resources exposed via `resources/list` and `resources/read`.

use std::sync::Arc;

use agp_game::{Content, GameError, ResourceDescriptor, ResourcePage, ResourceRange, Result};
use ffr_data::battle::EnemyProfile;
use ffr_data::{MODEL_LANGUAGE, main_quest_id_for_chapter};
use serde_json::{Value, json};

use crate::data_cache::{DbDataCache, GameDataCache, GameDataSource};
use crate::data_tools::{
    available_location_code, collapse_ws, cost_type_code, damage_type_code, element_zh,
    elements_zh_vec, format_skill_target, resolve_zh_clean,
};
use crate::data_tools_db::resolve_db_zh_clean;

pub const URI_CHAPTERS: &str = "ffr://chapters";
pub const URI_MAPS: &str = "ffr://maps";
pub const URI_ENEMIES: &str = "ffr://enemies";
pub const URI_SKILLS: &str = "ffr://skills";
pub const URI_DB_SCHEMA: &str = "ffr://db/schema";

/// List all read-only catalog resources exposed by `ffr-module`.
pub fn list_resources(source: &GameDataSource) -> Vec<ResourceDescriptor> {
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
        (
            URI_DB_SCHEMA,
            "游戏数据库表结构",
            "本地 ffr-db SQLite 数据库全部表、列名、类型与中文说明，供 query_game_db 编写只读 SQL 查询参考。",
        ),
    ];

    let available = source.is_available();
    entries
        .into_iter()
        .map(|(uri, name, description)| {
            let size_hint = if available {
                get_resource_body(source, uri)
                    .ok()
                    .map(|body| body.len() as u64)
            } else {
                None
            };
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
    source: &GameDataSource,
    uri: &str,
    range: ResourceRange,
) -> Result<ResourcePage> {
    if let Some(query) = uri.strip_prefix("ffr://enemies?") {
        let chapter_no = parse_enemies_chapter_query(query)?;
        let cache_key = format!("{URI_ENEMIES}?chapter={chapter_no}");
        let body = match source {
            GameDataSource::Db(db_cache) => db_cache.get_or_build_resource(&cache_key, |c| {
                build_enemies_for_chapter_json_db(c, chapter_no)
            }),
            GameDataSource::Json(cache) => cache.get_or_build_resource(&cache_key, |c| {
                build_enemies_for_chapter_json(c, chapter_no)
            }),
        }
        .map_err(|message| GameError::InvalidMessage {
            operation: "读取资源".to_owned(),
            message,
        })?;
        return slice_utf8_page(uri, body.as_str(), range);
    }

    if !matches!(
        uri,
        URI_CHAPTERS | URI_MAPS | URI_ENEMIES | URI_SKILLS | URI_DB_SCHEMA
    ) {
        return Err(GameError::InvalidMessage {
            operation: "读取资源".to_owned(),
            message: format!("没有这个资源：{uri}"),
        });
    }

    let body = get_resource_body(source, uri).map_err(|message| GameError::InvalidMessage {
        operation: "读取资源".to_owned(),
        message,
    })?;

    slice_utf8_page(uri, body.as_str(), range)
}

fn get_resource_body(
    source: &GameDataSource,
    uri: &str,
) -> std::result::Result<Arc<String>, String> {
    match source {
        GameDataSource::Db(db_cache) => match uri {
            URI_CHAPTERS => db_cache.get_or_build_resource(URI_CHAPTERS, build_chapters_json_db),
            URI_MAPS => db_cache.get_or_build_resource(URI_MAPS, build_maps_json_db),
            URI_ENEMIES => db_cache.get_or_build_resource(URI_ENEMIES, build_enemies_json_db),
            URI_SKILLS => db_cache.get_or_build_resource(URI_SKILLS, build_skills_json_db),
            URI_DB_SCHEMA => {
                db_cache.get_or_build_resource(URI_DB_SCHEMA, |_| build_db_schema_json())
            }
            _ => Err(format!("没有这个资源：{uri}")),
        },
        GameDataSource::Json(cache) => match uri {
            URI_CHAPTERS => cache.get_or_build_resource(URI_CHAPTERS, build_chapters_json),
            URI_MAPS => cache.get_or_build_resource(URI_MAPS, build_maps_json),
            URI_ENEMIES => cache.get_or_build_resource(URI_ENEMIES, build_enemies_json),
            URI_SKILLS => cache.get_or_build_resource(URI_SKILLS, build_skills_json),
            URI_DB_SCHEMA => cache.get_or_build_resource(URI_DB_SCHEMA, |_| build_db_schema_json()),
            _ => Err(format!("没有这个资源：{uri}")),
        },
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

fn format_enemy_catalog_entry_parts(
    profile: &EnemyProfile,
    name: Option<String>,
    chapters: Option<&[i64]>,
) -> Value {
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

fn format_enemy_catalog_entry(
    battle: &ffr_data::battle::BattleData,
    profile: &EnemyProfile,
    chapters: Option<&[i64]>,
) -> Value {
    let name = battle
        .unit_name_in(profile.unit_id, MODEL_LANGUAGE)
        .text
        .and_then(|t| collapse_ws(&t));
    format_enemy_catalog_entry_parts(profile, name, chapters)
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

// ---- `ffr-db` SQLite resource builders ----

fn build_chapters_json_db(cache: &DbDataCache) -> std::result::Result<String, String> {
    let (db, avail) = cache.open_with_availability()?;
    if !avail.scenario {
        return Err(
            "游戏数据库中缺少章节与主线剧情数据，请使用完整导出重新运行 `ffr-db build`".to_owned(),
        );
    }
    let chapter_rows = db
        .chapters()
        .map_err(|err| format!("查询章节列表失败：{err}"))?;
    let mut chapters = Vec::with_capacity(chapter_rows.len());
    for ch in &chapter_rows {
        let name = resolve_db_zh_clean(&db, &ch.name);
        let entries = db
            .main_scenario_by_chapter(ch.chapter_no)
            .map_err(|err| format!("查询第 {} 章主线剧情失败：{err}", ch.chapter_no))?;
        let main_scenario: Vec<Value> = entries
            .iter()
            .map(|entry| {
                let text = resolve_db_zh_clean(&db, &entry.text);
                let summary = resolve_db_zh_clean(&db, &entry.summary_text);
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

fn build_maps_json_db(cache: &DbDataCache) -> std::result::Result<String, String> {
    let (db, avail) = cache.open_with_availability()?;
    if !avail.world {
        return Err(
            "游戏数据库中缺少世界地图数据，请使用完整导出重新运行 `ffr-db build`".to_owned(),
        );
    }
    let catalog = db
        .maps_catalog()
        .map_err(|err| format!("查询地图一览失败：{err}"))?;
    let maps: Vec<Value> = catalog
        .iter()
        .map(|entry| {
            let map = &entry.map;
            let name = entry.name_zh.as_deref().and_then(collapse_ws);
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
                "teleportable": entry.teleportable,
                "treasure_count": entry.treasure_count,
                "facilities": facilities,
            })
        })
        .collect();
    serde_json::to_string(&json!({ "maps": maps }))
        .map_err(|error| format!("序列化地图资源失败：{error}"))
}

fn build_enemies_json_db(cache: &DbDataCache) -> std::result::Result<String, String> {
    let (db, avail) = cache.open_with_availability()?;
    if !avail.battle {
        return Err("游戏数据库中缺少战斗数据，请使用完整导出重新运行 `ffr-db build`".to_owned());
    }
    let rows = db
        .all_enemies_catalog()
        .map_err(|err| format!("查询敌人一览失败：{err}"))?;
    let chapters_map = if avail.enemy_chapters {
        Some(
            db.all_enemy_chapters()
                .map_err(|err| format!("查询敌人章节归属失败：{err}"))?,
        )
    } else {
        None
    };

    let mut enemies = Vec::with_capacity(rows.len());
    for row in rows {
        let profile = row.unit.to_enemy_profile(row.ai.as_ref());
        let name = row.name_zh.as_deref().and_then(collapse_ws);
        let chapters = chapters_map
            .as_ref()
            .map(|map| map.get(&profile.unit_id).map(Vec::as_slice).unwrap_or(&[]));
        enemies.push(format_enemy_catalog_entry_parts(&profile, name, chapters));
    }
    serde_json::to_string(&json!({ "enemies": enemies }))
        .map_err(|error| format!("序列化敌人资源失败：{error}"))
}

fn build_enemies_for_chapter_json_db(
    cache: &DbDataCache,
    chapter_no: i64,
) -> std::result::Result<String, String> {
    let (db, avail) = cache.open_with_availability()?;
    if !avail.battle {
        return Err("游戏数据库中缺少战斗数据，请使用完整导出重新运行 `ffr-db build`".to_owned());
    }
    if !avail.enemy_chapters {
        return Err(
            "游戏数据库中缺少敌人与章节映射数据，请使用完整导出重新运行 `ffr-db build`".to_owned(),
        );
    }
    let profiles = db
        .enemies_in_chapter(chapter_no)
        .map_err(|err| format!("查询第 {chapter_no} 章敌人失败：{err}"))?;
    let mut enemies = Vec::with_capacity(profiles.len());
    for profile in &profiles {
        let name = resolve_db_zh_clean(&db, &profile.name);
        let chapters = db
            .chapters_of_enemy(profile.unit_id)
            .map_err(|err| format!("查询敌人 {} 的章节归属失败：{err}", profile.unit_id))?;
        enemies.push(format_enemy_catalog_entry_parts(
            profile,
            name,
            Some(&chapters),
        ));
    }
    serde_json::to_string(&json!({
        "chapter": chapter_no,
        "enemies": enemies,
        "note": "章节归属由区域遇敌表、章节主/支线剧情战与陷阱宝箱静态推导，不含斗技场/强敌等无章节归属战斗。",
    }))
    .map_err(|error| format!("序列化第 {chapter_no} 章敌人资源失败：{error}"))
}

fn build_skills_json_db(cache: &DbDataCache) -> std::result::Result<String, String> {
    let (db, avail) = cache.open_with_availability()?;
    if !avail.battle {
        return Err(
            "游戏数据库中缺少战斗与技能数据，请使用完整导出重新运行 `ffr-db build`".to_owned(),
        );
    }
    let rows = db
        .all_skills_catalog()
        .map_err(|err| format!("查询技能一览失败：{err}"))?;
    let skills: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            let name = row.name_zh.as_deref().and_then(collapse_ws);
            let skill = row.skill;
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

// ---- `ffr://db/schema` builder ----

#[derive(Debug, Clone)]
struct SchemaTableInfo {
    area: &'static str,
    name: String,
    is_virtual: bool,
    columns: Vec<(String, String)>,
}

fn build_db_schema_json() -> std::result::Result<String, String> {
    let tables: Vec<Value> = parse_all_schema_tables()
        .into_iter()
        .map(|tbl| {
            let description = table_description_zh(&tbl.name).unwrap_or("游戏数据库表。");
            let cols: Vec<Value> = tbl
                .columns
                .into_iter()
                .map(|(col_name, col_type)| {
                    json!({
                        "name": col_name,
                        "type": col_type,
                    })
                })
                .collect();
            json!({
                "area": tbl.area,
                "name": tbl.name,
                "virtual": tbl.is_virtual,
                "description": description,
                "columns": cols,
            })
        })
        .collect();

    serde_json::to_string(&json!({
        "schema_version": ffr_db::SCHEMA_VERSION,
        "table_count": tables.len(),
        "note": "配合 query_game_db 使用；仅支持单条 SELECT/WITH/VALUES 只读查询（上限 200 行、2 秒超时、输出约 20 KB 截断）。",
        "tables": tables,
    }))
    .map_err(|error| format!("序列化数据库表结构资源失败：{error}"))
}

fn parse_all_schema_tables() -> Vec<SchemaTableInfo> {
    let mut out = Vec::new();
    for part in ffr_db::SCHEMA_PARTS {
        out.extend(parse_tables_from_ddl(part.name, part.tables_sql));
    }
    out
}

fn parse_tables_from_ddl(area: &'static str, sql: &str) -> Vec<SchemaTableInfo> {
    let mut tables = Vec::new();
    // Strip `-- ...` line comments before splitting DDL statements by `;`.
    let cleaned: String = sql
        .lines()
        .map(|line| match line.find("--") {
            Some(idx) => &line[..idx],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n");

    for stmt in cleaned.split(';') {
        let trimmed = stmt.trim();
        if trimmed.is_empty() {
            continue;
        }
        let upper = trimmed.to_ascii_uppercase();
        let (is_virtual, after_kw) = if upper.starts_with("CREATE VIRTUAL TABLE ") {
            (true, &trimmed["CREATE VIRTUAL TABLE ".len()..])
        } else if upper.starts_with("CREATE TABLE ") {
            (false, &trimmed["CREATE TABLE ".len()..])
        } else {
            continue;
        };
        let Some(open_paren) = after_kw.find('(') else {
            continue;
        };
        let Some(close_paren) = after_kw.rfind(')') else {
            continue;
        };
        if close_paren <= open_paren {
            continue;
        }
        let header = after_kw[..open_paren].trim();
        let Some(name) = header.split_whitespace().next() else {
            continue;
        };
        let body = &after_kw[open_paren + 1..close_paren];
        let columns = parse_column_defs(body, is_virtual);
        tables.push(SchemaTableInfo {
            area,
            name: name.to_owned(),
            is_virtual,
            columns,
        });
    }
    tables
}

fn parse_column_defs(body: &str, is_virtual: bool) -> Vec<(String, String)> {
    let mut cols = Vec::new();
    for raw_item in split_top_level_commas(body) {
        let item = raw_item.trim();
        if item.is_empty() {
            continue;
        }
        if is_virtual && item.contains('=') {
            // Skip FTS5 options like `content='text'`, `tokenize='trigram'`.
            continue;
        }
        let upper = item.to_ascii_uppercase();
        if upper.starts_with("PRIMARY KEY")
            || upper.starts_with("FOREIGN KEY")
            || upper.starts_with("UNIQUE")
            || upper.starts_with("CHECK")
            || upper.starts_with("CONSTRAINT")
        {
            continue;
        }
        let mut parts = item.split_whitespace();
        let Some(col_name) = parts.next() else {
            continue;
        };
        let col_type = if is_virtual {
            match parts.next() {
                Some(modifier) if modifier.eq_ignore_ascii_case("UNINDEXED") => {
                    "UNINDEXED".to_owned()
                }
                _ => "VIRTUAL".to_owned(),
            }
        } else {
            parts.next().unwrap_or("ANY").to_owned()
        };
        cols.push((col_name.to_owned(), col_type));
    }
    cols
}

fn split_top_level_commas(input: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0_usize;
    let mut in_single = false;
    let mut start = 0_usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '\'' => in_single = !in_single,
            '(' if !in_single => depth += 1,
            ')' if !in_single => depth = depth.saturating_sub(1),
            ',' if !in_single && depth == 0 => {
                out.push(&input[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }
    if start <= input.len() {
        out.push(&input[start..]);
    }
    out
}

fn table_description_zh(name: &str) -> Option<&'static str> {
    Some(match name {
        // 00_meta
        "meta" => "数据库构建元信息键值对（schema_version、游戏构建号、构建时间等）。",
        "source" => "各构建阶段读取的数据源目录、文件数与清单哈希。",
        "stage_report" => "各构建阶段的执行状态、写入行数、耗时与备注。",
        "build_issue" => "构建时记录的悬空引用或异常明细（照实保留不静默丢弃）。",
        // 01_text
        "text" => "多语言本地化文本表，按命名空间 ns、键 key 与语言 lang 索引。",
        "text_language" => "数据库已导入的本地化语言代码清单（如 zh-Hans、ja、en）。",
        "string_table" => "UE 字符串表路径到命名空间的映射。",
        "string_table_entry" => "字符串表的源语言条目（table_id、key、source）。",
        "text_fts" => "基于 FTS5 trigram 的全语言文本子串全文索引虚拟表。",
        // 02_datatable
        "dt_table" => "全部原始 DataTable / CompositeDataTable 的路径、行结构与行数索引。",
        "dt_row" => "全部原始数据表的逐行 JSON 文档（doc），可用 SQLite JSON 函数检索。",
        // 03_battle
        "unit" => "全部角色、敌人、Boss、召唤兽与幻兵的基础数值、弱点与掉落文档。",
        "unit_name" => "单位的多语言名称对照表（unit_id、lang、name）。",
        "unit_name_fts" => "单位名称 FTS5 trigram 模糊检索虚拟表。",
        "unit_resistance" => "单位对各战斗属性（火冰风土雷水光暗）的弱点与抗性关系表。",
        "skill" => "全部技能与魔法的属性、威力、破防削减、消耗与目标定义。",
        "skill_name" => "技能的多语言名称对照表（skill_id、lang、name）。",
        "skill_name_fts" => "技能名称 FTS5 trigram 模糊检索虚拟表。",
        "unit_skill" => "单位与招式的多对多关系表（区分习得来源与等级）。",
        "command_skill" => "战斗指令菜单项（招式/道具/幻兵召唤等）定义表。",
        "battle_group" => "战斗遭遇组定义（是否 Boss 战、可否逃跑、赏金战、AP 等）。",
        "battle_group_member" => "战斗组包含的敌人槽位与单位编号关系表。",
        "encounter" => "各区域遇敌池中的战斗组与遇敌权重。",
        "unit_ai" => "敌人 AI 配置与高危/斗气招式（high_risk_skill_ids）。",
        "battle_param" => "全局战斗破防倍率与破防增伤参数（单行 id=1）。",
        "enemy_chapter" => "敌人出没章节的静态推导来源（遇敌池/剧情战/陷阱箱）。",
        // 04_world
        "map" => "全部地图的编号、中文名、关卡名、类型、危险度与设施列表。",
        "island" => "世界地图大区/岛屿名称与显示条件。",
        "region" => "地图区域编号与所属岛屿。",
        "map_release_condition" => "各岛屿在大地图上的解锁条件。",
        "save_point" => "按地图类型定义的存档点能力规则（存档、帐篷、传送）。",
        "save_point_command" => "存档点菜单指令配置与消耗道具。",
        "teleport_point" => "世界地图传送地标坐标与飞空艇降落点。",
        "teleport_point_target" => "世界地图传送地标对应的目标地图与条件。",
        "map_link" => "由数据表推导的地图连接（大地图出入口与传送地标）。",
        "treasure_box" => "静态宝箱表：各地图宝箱的道具、数量、金币与陷阱战斗组。",
        "item" => "全部道具、装备、种子、幻兵、贵重品与圣遗物的价格和属性。",
        "shop" => "商店基本信息表（shop_id、店名与完整 doc）。",
        "shop_item" => "商店货单明细：道具编号、实际售价、限购数量与上架条件。",
        "various_shop" => "综合商店柜台定义（包含多个子商店）。",
        "various_shop_member" => "综合商店与其展开后的单个子商店映射关系。",
        "inn" => "旅店编号与住宿价格。",
        "quest" => "支线任务标题、推导章节、委托人、目标地图与完成条件。",
        "quest_prize" => "支线任务完成奖励道具与数量。",
        "quest_board" => "委托板任务定义（含占位项标记）。",
        "quest_board_reward" => "委托板任务奖励道具、数量与金币。",
        "chapter" => "主线章节号与章节名称。",
        "main_scenario" => "各章主线剧情进度阶段、目标文字与目的地地图。",
        "main_quest" => "主线章节合成的主线任务编号与标题。",
        "flag_condition" => "命名旗标条件表（CDT_FlagCondition）的条件表达式。",
        "flag_condition_term" => "命名旗标条件拆解出的单个事件旗标编号与期望真假值。",
        "playable_unit" => "野外可操作角色 map_unit_id 到战斗 unit_id 与名称的映射。",
        "item_drop" => "敌人普通/稀有掉落与偷窃道具的概率反查表。",
        "world_name" => "地图与道具的多语言小写归一化名称索引表。",
        "world_name_fts" => "地图与道具名称的 FTS5 trigram 模糊检索虚拟表。",
        // 05_assets
        "asset" => "AssetRegistry 全部游戏资源路径、类型、体积与导出状态。",
        "asset_tag" => "资源注册表的键值标签。",
        "asset_ref" => "资源之间的硬引用、软引用与脚本引用关系图。",
        "config_entry" => "游戏 Config/*.ini 配置段键值条目。",
        // 06_reflection
        "ue_class" => "从 .jmap 反射转储解析的 UE 类、结构体与枚举定义。",
        "ue_property" => "UE 类与结构体的字段名、类型、偏移与大小。",
        "ue_function" => "UE 类的函数签名、标志与参数列表。",
        "ue_enum_value" => "UE 枚举的枚举项名称与整型值。",
        // 07_input_ui
        "input_action" => "Enhanced Input 动作资产（IA_*）与值类型。",
        "input_context" => "Enhanced Input 映射上下文资产（IMC_*）。",
        "input_mapping" => "输入上下文中动作到按键、修饰器与触发器的绑定。",
        "input_set" => "输入集合与设备/上下文的组合关系。",
        "input_set_action" => "输入集合中的语义名到 InputAction 的映射。",
        "data_asset" => "导出的通用 DataAsset 文档。",
        "blueprint" => "导出的蓝图类默认值与父类信息。",
        "blueprint_component" => "蓝图声明的组件树与组件类。",
        "widget_node" => "UMG 界面蓝图的控件树层级与控件类。",
        // 08_levels
        "level" => "导出的关卡（.umap）元数据、子关卡类型与归属地图。",
        "level_map" => "关卡与地图的多对多归属关系（含共享子关卡）。",
        "actor" => "关卡中摆放的 Actor 位置、旋转、缩放、类与属性。",
        "actor_rtree" => "基于 R*Tree 的 3D 空间边界框索引虚拟表，用于按坐标查附近对象。",
        "level_object" => {
            "分类后的关卡对象（NPC、宝箱、出口、存档点、商店、旅店、事件触发、遇敌区域等）。"
        }
        "map_edge" => "地图连通图有向边（普通出口、同图传送、存档点传送）及坐标与条件。",
        "transition_point" => "各地图换图落点（point_id）的坐标与朝向。",
        "map_shop" => "地图与商店的展开映射表（含综合商店展开，关联对应 NPC level_object）。",
        // 09_sequences
        "sequence" => "过场动画 LevelSequence 的路径、时长、跳过标记与导演蓝图信息。",
        "sequence_binding" => "过场动画绑定的角色/Actor（含 map_unit_id）。",
        "sequence_track" => "过场动画的轨道类型与起止帧范围。",
        "sequence_event" => "过场动画中的事件关键帧（触发函数、对话事件、旗标）。",
        // 10_snapshots
        "snapshot" => "可选真机 R1 场面快照元信息（部署库默认为空）。",
        "snapshot_object" => "真机快照采集的运行时对象属性。",
        "snapshot_widget" => "真机快照采集的运行时界面控件状态。",
        "snapshot_text" => "真机快照采集的屏幕可见文字。",
        "marker" => "真机探查过程记录的时间线标记。",
        // 11_links
        "talk_event" => "对话事件主表：事件类型、台词表路径、过场关联、跳过确认与触发战斗组。",
        "talk_event_next" => "对话事件的连续后续事件与选项分支跳转关系。",
        "talk_line" => "按原始顺序排列的对话台词行（说话人、情绪、文本键与语音）。",
        "level_object_talk" => "关卡对象（NPC/事件触发等）到对话事件的触发关联。",
        "map_unit" => "地图 NPC 单位外貌资产与名称定义（DT_MapUnitAsset）。",
        "class_ref" => "跨区域类名归一化到 ue_class 与 blueprint 的统一映射。",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_schema_resource_covers_every_table_with_chinese_description_and_columns() {
        let tables = parse_all_schema_tables();
        assert!(
            tables.len() >= 70,
            "expected all ffr-db tables, got {}",
            tables.len()
        );
        for tbl in &tables {
            assert!(
                table_description_zh(&tbl.name).is_some(),
                "missing Chinese description for table `{}` in area `{}`",
                tbl.name,
                tbl.area
            );
            assert!(
                !tbl.columns.is_empty(),
                "parsed 0 columns for table `{}`",
                tbl.name
            );
        }
        let body = build_db_schema_json().expect("build ffr://db/schema JSON");
        let parsed: Value = serde_json::from_str(&body).expect("valid JSON");
        assert_eq!(parsed["table_count"], tables.len());
    }
}
