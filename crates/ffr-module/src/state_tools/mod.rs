//! Model-facing live game-state tool (`read_game_state`) backed by FfrBridge v1 and `ffr-data`.
//!
//! Reads a fresh snapshot via [`FileChannel::read_fresh`], arbitrates the active scene into a
//! one-line Chinese summary, automatically selects relevant sections (or honors `sections`), and
//! enriches numeric IDs with static Chinese names from `ffr-data` when available.

mod battle;
mod field;
mod party;
mod ui;

use std::sync::Arc;
use std::time::Duration;

use agp_game::{Content, ToolCategory, ToolDescriptor, ToolOutput};
use agp_platform::{FileChannel, FileChannelError};
use ffr_data::battle::{BattleData, CommandSkillData, Element, EnemyProfile, Skill};
use ffr_data::world::{Item, ItemCategory, TreasureBox, WorldData};
use ffr_data::{MODEL_LANGUAGE, MapUnitPlayableData, ScenarioData};
use serde_json::{Map, Value, json};

use crate::data_cache::GameDataCache;
use crate::data_tools::{collapse_ws, element_zh, resolve_zh_clean};

/// Name of the live game-state tool.
pub const READ_GAME_STATE_TOOL: &str = "read_game_state";

/// Supported topic sections in `read_game_state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StateSection {
    Field,
    Battle,
    Ui,
    Party,
}

impl StateSection {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "field" => Some(Self::Field),
            "battle" => Some(Self::Battle),
            "ui" => Some(Self::Ui),
            "party" => Some(Self::Party),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Field => "field",
            Self::Battle => "battle",
            Self::Ui => "ui",
            Self::Party => "party",
        }
    }
}

/// Output produced by one topic renderer (Chinese summary block + structured JSON).
pub(crate) struct SectionOutput {
    pub summary: String,
    pub json: Value,
}

/// Resolved main scenario objective matching the current `progress` value via
/// [`ScenarioData::current_main_scenario`].
pub(crate) struct ActiveMainScenario {
    pub chapter_no: Option<i64>,
    pub is_prologue: bool,
    pub main_quest_id: Option<i64>,
    pub main_quest_title: Option<String>,
    pub row_key: Option<String>,
    pub min_progress: Option<i64>,
    pub next_progress: Option<i64>,
    pub goal: Option<String>,
    pub summary: Option<String>,
    pub destinations: Vec<(i64, Option<String>)>,
}

/// Resolved battle command, skill, or item entry.
pub(crate) struct ResolvedAction {
    pub id: i64,
    pub name: Option<String>,
    pub kind: &'static str,
}

impl ResolvedAction {
    pub(crate) fn label(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => format!("#{}", self.id),
        }
    }

    pub(crate) fn display_with_id(&self) -> String {
        match &self.name {
            Some(name) => format!("{name}（#{}）", self.id),
            None => format!("#{}", self.id),
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "kind": self.kind,
        })
    }
}

/// Best-effort static data lookup facade used across status tools and bridge command tools.
pub(crate) struct LookupCtx {
    world: Option<Arc<WorldData>>,
    battle: Option<Arc<BattleData>>,
    scenario: Option<Arc<ScenarioData>>,
    command_skills: Option<Arc<CommandSkillData>>,
    map_unit_playable: Option<Arc<MapUnitPlayableData>>,
}

impl LookupCtx {
    pub(crate) fn from_cache(
        cache: &GameDataCache,
        need_world: bool,
        need_battle: bool,
        need_scenario: bool,
    ) -> Self {
        let world = if need_world { cache.world().ok() } else { None };
        let battle = if need_battle {
            cache.battle().ok()
        } else {
            None
        };
        let scenario = if need_scenario {
            cache.scenario().ok()
        } else {
            None
        };
        let command_skills = if need_battle {
            cache.command_skills().ok()
        } else {
            None
        };
        let map_unit_playable = if need_world {
            cache.map_unit_playable().ok()
        } else {
            None
        };
        Self {
            world,
            battle,
            scenario,
            command_skills,
            map_unit_playable,
        }
    }

    pub(crate) fn map_name(&self, map_id: i64) -> Option<String> {
        let world = self.world.as_deref()?;
        let map = world.map_by_id(map_id)?;
        resolve_zh_clean(world.text_resolver(), &map.name)
    }

    pub(crate) fn map_name_by_level(&self, level_name: &str) -> Option<(i64, Option<String>)> {
        let world = self.world.as_deref()?;
        let map = world.map_by_level_name(level_name)?;
        let name = resolve_zh_clean(world.text_resolver(), &map.name);
        Some((map.id, name))
    }

    pub(crate) fn treasure_box(&self, map_id: i64, chest_id: i64) -> Option<&TreasureBox> {
        self.world.as_deref()?.treasure_box(map_id, chest_id)
    }

    pub(crate) fn unit_name(&self, unit_id: i64) -> Option<String> {
        let battle = self.battle.as_deref()?;
        battle
            .unit_name_in(unit_id, MODEL_LANGUAGE)
            .text
            .and_then(|t| collapse_ws(&t))
    }

    pub(crate) fn enemy_profile(&self, unit_id: i64) -> Option<EnemyProfile> {
        let battle = self.battle.as_deref()?;
        let unit = battle.unit_by_id(unit_id)?;
        Some(unit.to_enemy_profile(battle.unit_ai_by_id(unit_id)))
    }

    pub(crate) fn skill(&self, skill_id: i64) -> Option<&Skill> {
        self.battle.as_deref()?.skill_by_id(skill_id)
    }

    pub(crate) fn skill_name(&self, skill_id: i64) -> Option<String> {
        let battle = self.battle.as_deref()?;
        battle
            .skill_name_in(skill_id, MODEL_LANGUAGE)
            .text
            .and_then(|t| collapse_ws(&t))
    }

    pub(crate) fn item(&self, item_id: i64) -> Option<&Item> {
        self.world.as_deref()?.item_by_id(item_id)
    }

    pub(crate) fn item_name(&self, item_id: i64) -> Option<String> {
        let world = self.world.as_deref()?;
        let item = world.item_by_id(item_id)?;
        resolve_zh_clean(world.text_resolver(), &item.name)
    }

    pub(crate) fn shop_name(&self, shop_id: i64) -> Option<String> {
        let world = self.world.as_deref()?;
        let shop = world.shop_by_id(shop_id)?;
        resolve_zh_clean(world.text_resolver(), &shop.name)
    }

    pub(crate) fn inn_price(&self, inn_id: i64) -> Option<i64> {
        let world = self.world.as_deref()?;
        world
            .inns()
            .iter()
            .find(|inn| inn.id == inn_id)
            .map(|inn| inn.price)
    }

    pub(crate) fn playable_character_by_map_unit_id(
        &self,
        map_unit_id: i64,
    ) -> Option<(i64, Option<String>)> {
        let playable = self.map_unit_playable.as_deref()?;
        let resolved = playable.resolve_by_map_unit_id(map_unit_id)?;
        let name = playable
            .character_name_in(map_unit_id, MODEL_LANGUAGE)
            .text
            .and_then(|t| collapse_ws(&t));
        Some((resolved.unit_id, name))
    }

    pub(crate) fn main_quest_title(&self, main_quest_id: i64) -> Option<String> {
        let scenario = self.scenario.as_deref()?;
        scenario.main_quest_by_id(main_quest_id)?;
        scenario
            .main_quest_title_in(main_quest_id, MODEL_LANGUAGE)
            .text
            .and_then(|t| collapse_ws(&t))
    }

    pub(crate) fn quest_name(&self, quest_id: i64) -> Option<String> {
        let world = self.world.as_deref()?;
        if let Some(q) = world.quest_by_id(quest_id) {
            return resolve_zh_clean(world.text_resolver(), &q.title);
        }
        if let Some(qb) = world.quest_board_by_id(quest_id) {
            return resolve_zh_clean(world.text_resolver(), &qb.title);
        }
        None
    }

    pub(crate) fn chapter_name(&self, chapter_no: i64) -> Option<String> {
        if chapter_no == 0 {
            return Some("序章".to_owned());
        }
        let scenario = self.scenario.as_deref()?;
        scenario
            .chapter_name(chapter_no)
            .text
            .and_then(|t| collapse_ws(&t))
    }

    pub(crate) fn active_main_scenario(&self, progress: i64) -> Option<ActiveMainScenario> {
        let scenario = self.scenario.as_deref()?;
        let current = scenario.current_main_scenario(progress)?;
        let main_quest_title = current
            .main_quest_id
            .and_then(|qid| self.main_quest_title(qid));
        let (row_key, goal, summary) = match &current.entry {
            Some(entry) => {
                let texts = scenario.resolve_main_scenario_texts(entry);
                let goal = texts.text.text.and_then(|t| collapse_ws(&t));
                let summary = texts.summary_text.text.and_then(|t| collapse_ws(&t));
                (Some(entry.row_key.clone()), goal, summary)
            }
            None => (None, None, None),
        };
        let destinations = current
            .destination_map_ids
            .iter()
            .map(|&id| (id, self.map_name(id)))
            .collect();
        Some(ActiveMainScenario {
            chapter_no: current.chapter_no,
            is_prologue: current.is_prologue,
            main_quest_id: current.main_quest_id,
            main_quest_title,
            row_key,
            min_progress: current.progress_start,
            next_progress: current.next_progress,
            goal,
            summary,
            destinations,
        })
    }

    pub(crate) fn resolve_battle_action(
        &self,
        action_id: i64,
        is_item_context: bool,
    ) -> ResolvedAction {
        if is_item_context {
            if let Some(name) = self.item_name(action_id) {
                return ResolvedAction {
                    id: action_id,
                    name: Some(name),
                    kind: "item",
                };
            }
            if let Some(name) = self.skill_name(action_id) {
                return ResolvedAction {
                    id: action_id,
                    name: Some(name),
                    kind: "skill",
                };
            }
            return ResolvedAction {
                id: action_id,
                name: None,
                kind: "unknown",
            };
        }
        if let Some(commands) = self.command_skills.as_deref()
            && commands.command_by_id(action_id).is_some()
        {
            let name = commands
                .command_name_in(action_id, MODEL_LANGUAGE)
                .text
                .and_then(|t| collapse_ws(&t));
            return ResolvedAction {
                id: action_id,
                name,
                kind: "command",
            };
        }
        if let Some(name) = self.skill_name(action_id) {
            return ResolvedAction {
                id: action_id,
                name: Some(name),
                kind: "skill",
            };
        }
        ResolvedAction {
            id: action_id,
            name: None,
            kind: "unknown",
        }
    }
}

/// Build the [`ToolDescriptor`] for `read_game_state`.
pub fn read_game_state_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: READ_GAME_STATE_TOOL.to_owned(),
        description: "读取当前游戏实时状态（FfrBridge v1），结合静态数据表翻译为中文摘要与结构化 JSON，包含场面判断、野外/战斗/界面/队伍主题。默认按当前场面自动挑选主题；战斗中默认仅显示已揭示的敌人弱点。"
            .to_owned(),
        parameters: json!({
            "type": "object",
            "properties": {
                "sections": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["field", "battle", "ui", "party"]
                    },
                    "description": "可选，指定要返回的主题列表（field/battle/ui/party）；省略时按当前场面自动选择。"
                }
            },
            "additionalProperties": false
        }),
        category: ToolCategory::Status,
        side_effects: false,
        timeout_ms: None,
    }
}

/// Execute `read_game_state`.
pub async fn handle_read_game_state(
    channel: &FileChannel,
    cache: &GameDataCache,
    reveal_hidden_weaknesses: bool,
    args: Value,
) -> ToolOutput {
    let sections_req = match parse_sections_arg(&args) {
        Ok(sections) => sections,
        Err(message) => {
            let mut output = ToolOutput::new(vec![Content::text(message)]);
            output.is_error = true;
            return output;
        }
    };

    let snapshot = match channel.read_fresh().await {
        Ok(snapshot) => snapshot,
        Err(error) => return bridge_read_error(error),
    };

    let Some(root) = snapshot.data.as_object() else {
        let mut output = ToolOutput::new(vec![
            Content::text("读取当前游戏状态失败：state.json 的 data 字段不是 JSON 对象"),
            Content::json(json!({
                "ok": false,
                "stale": false,
                "error": "state.json 的 data 字段不是 JSON 对象",
            })),
        ]);
        output.is_error = true;
        return output;
    };

    let field_obj = obj_object(root, "field");
    let battle_obj = obj_object(root, "battle");
    let ui_obj = obj_object(root, "ui");
    let party_obj = obj_object(root, "party");
    let bridge_version = obj_str(root, "bridge_version").unwrap_or("0");
    let bridge_errors = extract_str_list(root.get("errors"));

    let selected = match sections_req {
        Some(explicit) => explicit,
        None => auto_select_sections(field_obj, battle_obj, ui_obj, party_obj),
    };

    let need_world = selected.contains(&StateSection::Field)
        || selected.contains(&StateSection::Ui)
        || selected.contains(&StateSection::Party)
        || selected.contains(&StateSection::Battle);
    let need_battle = selected.contains(&StateSection::Battle)
        || selected.contains(&StateSection::Party)
        || selected.contains(&StateSection::Ui)
        || battle_obj.and_then(|b| obj_bool(b, "in_battle")) == Some(true);
    let need_scenario = selected.contains(&StateSection::Party)
        || selected.contains(&StateSection::Field)
        || selected.contains(&StateSection::Ui);
    let lookup = LookupCtx::from_cache(cache, need_world, need_battle, need_scenario);

    let screen_kind = resolve_screen_kind(field_obj, battle_obj, ui_obj);
    let scene_summary = summarize_scene(
        screen_kind,
        root,
        field_obj,
        battle_obj,
        ui_obj,
        party_obj,
        &lookup,
    );

    let mut summary_blocks = vec![format!("场面：{scene_summary}")];
    let mut sections_json = Map::new();

    for section in &selected {
        let rendered = match section {
            StateSection::Field => field::render_field(root, field_obj, &lookup),
            StateSection::Battle => {
                battle::render_battle(battle_obj, &lookup, reveal_hidden_weaknesses)
            }
            StateSection::Ui => ui::render_ui(ui_obj, &lookup),
            StateSection::Party => party::render_party(party_obj, &lookup),
        };
        summary_blocks.push(rendered.summary);
        sections_json.insert(section.as_str().to_owned(), rendered.json);
    }

    if !bridge_errors.is_empty() {
        summary_blocks.push(format!(
            "提示：本次桥采样记录了 {} 条读取告警（详见 bridge_errors）。",
            bridge_errors.len()
        ));
    }

    let section_names: Vec<&str> = selected.iter().map(|s| s.as_str()).collect();
    let payload = json!({
        "ok": true,
        "bridge_version": bridge_version,
        "session": snapshot.session,
        "seq": snapshot.seq,
        "clock_ms": snapshot.clock_ms,
        "file_age_ms": snapshot.file_age.map(duration_millis),
        "screen_kind": screen_kind,
        "scene_summary": scene_summary,
        "sections": section_names,
        "reveal_hidden_weaknesses": reveal_hidden_weaknesses,
        "data_loaded": {
            "world": lookup.world.is_some(),
            "battle": lookup.battle.is_some(),
            "scenario": lookup.scenario.is_some(),
            "command_skills": lookup.command_skills.is_some(),
            "map_unit_playable": lookup.map_unit_playable.is_some(),
        },
        "state": Value::Object(sections_json),
        "bridge_errors": bridge_errors,
    });

    ToolOutput::new(vec![
        Content::text(summary_blocks.join("\n\n")),
        Content::json(payload),
    ])
}

fn parse_sections_arg(args: &Value) -> Result<Option<Vec<StateSection>>, String> {
    let Some(obj) = args.as_object() else {
        return Err("工具 `read_game_state` 的参数必须是 JSON 对象".to_owned());
    };
    let unknown: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|k| *k != "sections")
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "工具 `read_game_state` 收到未知参数：{}",
            unknown.join("、")
        ));
    }
    let Some(raw) = obj.get("sections") else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let Some(arr) = raw.as_array() else {
        return Err("工具 `read_game_state` 的参数 `sections` 必须是字符串数组".to_owned());
    };
    if arr.is_empty() {
        return Err(
            "工具 `read_game_state` 的参数 `sections` 不能为空数组，请至少指定一个主题或省略该参数"
                .to_owned(),
        );
    }
    let mut parsed = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(name) = item.as_str() else {
            return Err("工具 `read_game_state` 的参数 `sections` 只能包含字符串".to_owned());
        };
        let Some(sec) = StateSection::parse(name) else {
            return Err(format!(
                "工具 `read_game_state` 不支持主题 `{name}`，可选值为：field、battle、ui、party"
            ));
        };
        if !parsed.contains(&sec) {
            parsed.push(sec);
        }
    }
    Ok(Some(parsed))
}

fn bridge_read_error(error: FileChannelError) -> ToolOutput {
    let (stale, file_age_ms, text) = match &error {
        FileChannelError::Stale { file_age, .. } => (
            true,
            file_age.map(duration_millis),
            format!(
                "状态停止更新：文件桥快照已陈旧，拒绝返回旧数据（{error}）。请确认游戏未卡死或已加载 FfrBridge 模组。"
            ),
        ),
        _ => (false, None, format!("无法读取当前游戏状态：{error}")),
    };
    let mut output = ToolOutput::new(vec![
        Content::text(text),
        Content::json(json!({
            "ok": false,
            "stale": stale,
            "file_age_ms": file_age_ms,
            "error": error.to_string(),
        })),
    ]);
    output.is_error = true;
    output
}

fn resolve_screen_kind<'a>(
    field: Option<&'a Map<String, Value>>,
    battle: Option<&'a Map<String, Value>>,
    ui: Option<&'a Map<String, Value>>,
) -> &'a str {
    if let Some(kind) = ui.and_then(|u| obj_str(u, "screen_kind")) {
        return kind;
    }
    if field.and_then(|f| obj_bool(f, "is_loading")) == Some(true) {
        return "loading";
    }
    if battle.and_then(|b| obj_bool(b, "in_battle")) == Some(true)
        || field.and_then(|f| obj_bool(f, "in_battle")) == Some(true)
    {
        return "battle";
    }
    if field.is_some() {
        return "field";
    }
    "unknown"
}

fn auto_select_sections(
    field: Option<&Map<String, Value>>,
    battle: Option<&Map<String, Value>>,
    ui: Option<&Map<String, Value>>,
    party: Option<&Map<String, Value>>,
) -> Vec<StateSection> {
    let screen_kind = resolve_screen_kind(field, battle, ui);
    let in_battle = battle.and_then(|b| obj_bool(b, "in_battle")) == Some(true);
    let ui_blocked = ui
        .and_then(|u| obj_array(u, "input_blocked"))
        .is_some_and(|arr| !arr.is_empty());

    match screen_kind {
        "battle" => {
            let mut out = vec![StateSection::Battle];
            if ui_blocked
                || battle.and_then(|b| obj_bool(b, "in_tutorial")) == Some(true)
                || battle.and_then(|b| obj_bool(b, "in_talk_event")) == Some(true)
            {
                out.push(StateSection::Ui);
            }
            out
        }
        "battle_result" => vec![StateSection::Battle, StateSection::Ui, StateSection::Party],
        "field" => {
            let mut out = vec![StateSection::Field];
            if party.is_some() {
                out.push(StateSection::Party);
            }
            if ui_blocked {
                out.push(StateSection::Ui);
            }
            out
        }
        "talk" | "cutscene" | "choice" | "dialog" | "tutorial" | "movie" => {
            let mut out = vec![StateSection::Ui];
            if in_battle {
                out.push(StateSection::Battle);
            } else if field.is_some() {
                out.push(StateSection::Field);
            }
            out
        }
        "shop" | "main_menu" | "save_ui" | "system_menu" => {
            let mut out = vec![StateSection::Ui];
            if party.is_some() {
                out.push(StateSection::Party);
            }
            out
        }
        "savepoint" => {
            let mut out = vec![StateSection::Ui];
            if field.is_some() {
                out.push(StateSection::Field);
            }
            if party.is_some() {
                out.push(StateSection::Party);
            }
            out
        }
        "loading" => {
            let mut out = vec![StateSection::Ui];
            if field.is_some() {
                out.push(StateSection::Field);
            }
            out
        }
        "title" => vec![StateSection::Ui],
        _ => {
            let mut out = Vec::new();
            if field.is_some() {
                out.push(StateSection::Field);
            }
            if in_battle {
                out.push(StateSection::Battle);
            }
            if ui.is_some() {
                out.push(StateSection::Ui);
            }
            if party.is_some() {
                out.push(StateSection::Party);
            }
            if out.is_empty() {
                out.push(StateSection::Field);
            }
            out
        }
    }
}

fn summarize_scene(
    screen_kind: &str,
    root: &Map<String, Value>,
    field_obj: Option<&Map<String, Value>>,
    battle_obj: Option<&Map<String, Value>>,
    ui_obj: Option<&Map<String, Value>>,
    party_obj: Option<&Map<String, Value>>,
    lookup: &LookupCtx,
) -> String {
    let in_battle = battle_obj.and_then(|b| obj_bool(b, "in_battle")) == Some(true);
    let top_map = obj_str(root, "map");
    let raw_map_id = field_obj.and_then(|f| obj_i64(f, "map_id"));
    let (map_id, map_name) = field::resolve_field_map(raw_map_id, top_map, lookup);

    let mut headline = match screen_kind {
        "loading" => match (map_id, map_name.as_deref()) {
            (Some(id), Some(name)) => format!("载入中，地图 {id}（{name}）"),
            (Some(id), None) => format!("载入中，地图 {id}"),
            _ => "载入中".to_owned(),
        },
        "movie" => {
            let sub = ui_obj
                .and_then(|u| obj_object(u, "movie"))
                .and_then(|m| obj_str(m, "subtitle"));
            match sub {
                Some(text) => format!("过场影片播放中，字幕：「{text}」"),
                None => "过场影片播放中".to_owned(),
            }
        }
        "tutorial" => {
            let prefix = if in_battle { "战斗中，" } else { "" };
            if let Some(tut) = ui_obj.and_then(|u| obj_object(u, "tutorial")) {
                let caption = obj_str(tut, "caption").unwrap_or("教学");
                let page = obj_i64(tut, "page_index").unwrap_or(0) + 1;
                let total = obj_i64(tut, "page_count").unwrap_or(1);
                let action = if obj_bool(tut, "can_close") == Some(true) {
                    "可关闭"
                } else {
                    "需翻页"
                };
                format!("{prefix}教学弹窗「{caption}」（第 {page}/{total} 页，{action}）")
            } else {
                format!("{prefix}教学弹窗中")
            }
        }
        "dialog" | "choice" => {
            let label = if screen_kind == "choice" {
                "剧情选项"
            } else {
                "系统弹窗"
            };
            if let Some(ch) = ui_obj.and_then(|u| obj_object(u, "choice")) {
                let msg = obj_str(ch, "message").unwrap_or_default();
                let options = extract_str_list(ch.get("options"));
                let focus = obj_i64(ch, "focus_index")
                    .and_then(|i| usize::try_from(i).ok())
                    .and_then(|i| options.get(i))
                    .map(String::as_str);
                match focus {
                    Some(opt) if !msg.is_empty() => format!("{label}：{msg}，光标在「{opt}」"),
                    Some(opt) => format!("{label}，光标在「{opt}」"),
                    None if !msg.is_empty() => format!("{label}：{msg}"),
                    None => label.to_owned(),
                }
            } else {
                label.to_owned()
            }
        }
        "talk" => {
            let prefix = if in_battle {
                "战斗对话中"
            } else {
                "对话中"
            };
            if let Some(talk) = ui_obj.and_then(|u| obj_object(u, "talk")) {
                let speaker = obj_str(talk, "speaker");
                let wait = if obj_bool(talk, "waiting_key") == Some(true) {
                    "等待按键"
                } else {
                    "文字播放中"
                };
                match speaker {
                    Some(s) if !s.is_empty() => format!("{prefix}，说话人 {s}，{wait}"),
                    _ => format!("{prefix}，{wait}"),
                }
            } else {
                prefix.to_owned()
            }
        }
        "cutscene" => {
            let text = ui_obj
                .and_then(|u| obj_object(u, "talk"))
                .and_then(|t| obj_str(t, "text"));
            match text {
                Some(t) if !t.is_empty() => format!("过场演出中：「{t}」"),
                _ => "过场演出中".to_owned(),
            }
        }
        "shop" => {
            if let Some(shop) = ui_obj.and_then(|u| obj_object(u, "shop")) {
                let shop_id = obj_i64(shop, "shop_id").filter(|&id| id >= 0);
                let shop_name = shop_id.and_then(|id| lookup.shop_name(id));
                let title = match (shop_id, shop_name.as_deref()) {
                    (Some(id), Some(name)) => format!("商店 {id}（{name}）"),
                    (Some(id), None) => format!("商店 {id}"),
                    _ => "商店".to_owned(),
                };
                let mode = match obj_bool(shop, "is_buy") {
                    Some(true) => "购买",
                    Some(false) => "出售",
                    None => "选单",
                };
                let focus_item = obj_i64(shop, "item_focus_index")
                    .and_then(|i| usize::try_from(i).ok())
                    .and_then(|i| obj_array(shop, "items")?.get(i))
                    .and_then(Value::as_object)
                    .and_then(|item| {
                        obj_i64(item, "id")
                            .and_then(|id| lookup.item_name(id))
                            .or_else(|| obj_str(item, "name").map(str::to_owned))
                    });
                let money_part = party_obj
                    .and_then(|p| obj_i64(p, "money"))
                    .map_or_else(String::new, |g| format!("，金币 {g}G"));
                match focus_item {
                    Some(item) => {
                        format!("商店中，{title}（{mode}，光标在「{item}」{money_part}）")
                    }
                    None => format!("商店中，{title}（{mode}{money_part}）"),
                }
            } else {
                "商店界面".to_owned()
            }
        }
        "save_ui" => {
            if let Some(su) = ui_obj.and_then(|u| obj_object(u, "save_ui")) {
                let mode = if obj_str(su, "mode") == Some("load") {
                    "读档"
                } else {
                    "存档"
                };
                let slots = extract_i64_list(su.get("slot_indices"));
                let slot = obj_i64(su, "focus_index")
                    .and_then(|i| usize::try_from(i).ok())
                    .and_then(|i| slots.get(i))
                    .copied();
                match slot {
                    Some(s) => format!("{mode}界面，光标在槽位 {s}"),
                    None => format!("{mode}界面"),
                }
            } else {
                "存读档界面".to_owned()
            }
        }
        "system_menu" | "main_menu" | "savepoint" | "title" => {
            let label = ui::menu_kind_zh(screen_kind);
            if let Some(menu) = ui_obj.and_then(|u| obj_object(u, "menu")) {
                let entries = extract_str_list(menu.get("entries"));
                let focus = obj_i64(menu, "focus_index")
                    .and_then(|i| usize::try_from(i).ok())
                    .and_then(|i| entries.get(i));
                match focus {
                    Some(entry) => format!("{label}，光标在「{entry}」"),
                    None => label.to_owned(),
                }
            } else {
                ui::screen_kind_label_zh(screen_kind).to_owned()
            }
        }
        "battle_result" => {
            let has_lu = ui_obj
                .and_then(|u| obj_object(u, "battle_result"))
                .and_then(|br| obj_bool(br, "has_level_up_window"))
                .unwrap_or(false);
            let page_idx = ui_obj
                .and_then(|u| obj_object(u, "battle_result"))
                .and_then(|br| obj_i64(br, "page_index"))
                .or_else(|| {
                    battle_obj
                        .and_then(|b| obj_object(b, "result"))
                        .and_then(|r| obj_i64(r, "page_index"))
                });
            let page_desc = match (page_idx, has_lu) {
                (Some(0), _) => "报酬页",
                (Some(1), true) => "升级/习得技能页",
                (Some(1), false) => "成长页",
                _ => "结算页",
            };
            format!("战斗结算中（{page_desc}）")
        }
        "battle" => match battle_obj {
            Some(b) => battle::summarize_battle_headline(b, lookup),
            None => "战斗中".to_owned(),
        },
        "field" => {
            let map_part = match (map_id, map_name.as_deref()) {
                (Some(id), Some(name)) => format!("地图 {id}（{name}）"),
                (Some(id), None) => format!("地图 {id}"),
                (None, Some(name)) => format!("地图（{name}）"),
                (None, None) => {
                    top_map.map_or_else(|| "未知地图".to_owned(), |lvl| format!("关卡 {lvl}"))
                }
            };
            let interact_part = field_obj
                .and_then(|f| obj_object(f, "interact_target"))
                .map(|t| format!("，可交互：{}", field::describe_interact_brief(t, lookup)))
                .unwrap_or_default();
            let ctrl_part = match field_obj.and_then(|f| obj_bool(f, "can_control")) {
                Some(false) => "，当前不可操作",
                _ => "",
            };
            format!("野外，{map_part}{interact_part}{ctrl_part}")
        }
        _ => match (map_id, map_name.as_deref(), top_map) {
            (Some(id), Some(name), _) => format!("状态未细分，地图 {id}（{name}）"),
            (_, _, Some(lvl)) => format!("状态未细分，关卡 {lvl}"),
            _ => "状态未知（桥仅提供基础字段）".to_owned(),
        },
    };

    if ui_obj
        .and_then(|u| obj_array(u, "input_blocked"))
        .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some("WindowDeactivate")))
    {
        headline.push_str("（窗口失焦，输入已屏蔽）");
    }
    headline
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

// ---- Shared JSON & formatting helpers for `state_tools` and `bridge_commands` ----

pub(crate) fn obj_str<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str)
}

pub(crate) fn obj_i64(obj: &Map<String, Value>, key: &str) -> Option<i64> {
    obj.get(key).and_then(Value::as_i64)
}

pub(crate) fn obj_f64(obj: &Map<String, Value>, key: &str) -> Option<f64> {
    obj.get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
}

pub(crate) fn obj_bool(obj: &Map<String, Value>, key: &str) -> Option<bool> {
    obj.get(key).and_then(Value::as_bool)
}

pub(crate) fn obj_array<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a [Value]> {
    obj.get(key).and_then(Value::as_array).map(Vec::as_slice)
}

pub(crate) fn obj_object<'a>(
    obj: &'a Map<String, Value>,
    key: &str,
) -> Option<&'a Map<String, Value>> {
    obj.get(key).and_then(Value::as_object)
}

pub(crate) fn extract_i64_list(val: Option<&Value>) -> Vec<i64> {
    val.and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default()
}

pub(crate) fn extract_str_list(val: Option<&Value>) -> Vec<String> {
    val.and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn format_location(loc: Option<&Map<String, Value>>) -> Option<(String, Value)> {
    let loc = loc?;
    let x = obj_f64(loc, "x")?;
    let y = obj_f64(loc, "y")?;
    let z = obj_f64(loc, "z")?;
    Some((
        format!("({x:.1}, {y:.1}, {z:.1})"),
        json!({ "x": x, "y": y, "z": z }),
    ))
}

pub(crate) fn format_play_time(secs: f64) -> String {
    let total = secs.max(0.0).floor() as u64;
    let hours = total / 3600;
    let mins = (total % 3600) / 60;
    let rem = total % 60;
    format!("{hours:02}:{mins:02}:{rem:02}")
}

pub(crate) fn format_map_label(map_id: i64, name: Option<&str>) -> String {
    match name {
        Some(name) => format!("地图 #{map_id}「{name}」"),
        None => format!("地图 #{map_id}"),
    }
}

pub(crate) fn format_item_label(item_id: i64, name: Option<&str>) -> String {
    match name {
        Some(name) => format!("{name}（#{item_id}）"),
        None => format!("道具 #{item_id}"),
    }
}

pub(crate) fn parse_element_code(raw: &str) -> Option<Element> {
    let short = raw.rsplit_once("::").map_or(raw, |(_, s)| s);
    match short {
        "Fire" => Some(Element::Fire),
        "Ice" => Some(Element::Ice),
        "Wind" => Some(Element::Wind),
        "Earth" => Some(Element::Earth),
        "Thunder" => Some(Element::Thunder),
        "Water" => Some(Element::Water),
        "Light" => Some(Element::Light),
        "Dark" => Some(Element::Dark),
        "None" => Some(Element::None),
        _ => None,
    }
}

pub(crate) fn element_label_zh(raw: &str) -> String {
    parse_element_code(raw).map_or_else(|| raw.to_owned(), |elem| element_zh(elem).to_owned())
}

pub(crate) fn resistance_label_zh(raw: &str) -> &'static str {
    let short = raw.rsplit_once("::").map_or(raw, |(_, s)| s);
    match short {
        "Weakness" => "弱点",
        "Halved" => "减半",
        "Invalid" => "无效",
        "Absorb" => "吸收",
        "Reflect" => "反射",
        "None" => "通常",
        _ => "未知",
    }
}

pub(crate) fn interact_type_zh(raw: &str) -> &'static str {
    let short = raw.rsplit_once("::").map_or(raw, |(_, s)| s);
    match short {
        "Talk" | "Talk_Crowd" | "Talk_TalkPoint" => "对话",
        "Check" | "Check_Field" | "Check_Map" | "Check_Point" | "Check_Gimmick"
        | "Check_MerchantShip" => "调查",
        "Battle" => "战斗",
        "Treasure" => "宝箱",
        "MythicalBeast" => "幻兽",
        "Door" => "门",
        "Vehicle" | "Vehicle_Ship" => "载具",
        "Landing" => "降落点",
        "Shop" => "道具店",
        "Shop_Weapon" => "武器店",
        "Shop_Armor" => "防具店",
        "Shop_Accessory" => "饰品店",
        "Shop_Vision" => "幻兵店",
        "Inn" => "旅店",
        "QuestBoard" => "任务板",
        "Bank" => "仓库",
        "ChocoboShop" => "陆行鸟商店",
        "ChocoboRace" => "陆行鸟赛跑",
        "MusicShop" => "音乐商店",
        "DLCShop" => "DLC 商店",
        "Fortuneteller" => "占卜师",
        "Colosseum_Rank" | "Colosseum_Hunt" | "Colosseum_Boss" => "斗技场",
        "SavePoint" => "存档点",
        "Transition" | "Transition_Event" => "场景切换",
        "Landmark" => "地标",
        "City" => "城镇",
        "Dungeon" => "迷宫",
        _ => "交互",
    }
}

pub(crate) fn item_category_zh(category: ItemCategory) -> &'static str {
    match category {
        ItemCategory::Consumable => "消耗品",
        ItemCategory::Seed => "种子",
        ItemCategory::Equipment => "装备",
        ItemCategory::Vision => "幻兵",
        ItemCategory::Valuable => "贵重品",
        ItemCategory::Artifact => "圣遗物",
    }
}
