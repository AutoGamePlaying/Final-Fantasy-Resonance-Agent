//! Tests for FfrBridge v1 live state (`read_game_state`), weakness fairness switch,
//! staleness handling, and the 5 on-demand read-only bridge command tools.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agp_core::RunId;
use agp_game::{CallContext, Content, GameModule, ToolOutput};
use common::{
    TestDir, build_merged_fixture_export, call_data_ok, load_bridge_v1_fixture, module,
    module_with_data, write_json_atomic, write_state,
};
use ffr_module::FfrModule;
use serde_json::{Value, json};
use tokio::fs;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;

async fn write_v1_fixture(dir: &Path, name: &str) {
    let value = load_bridge_v1_fixture(name);
    write_json_atomic(&dir.join("state.json"), &value).await;
}

async fn call_status_err(module: &FfrModule, name: &str, args: Value) -> (String, Option<Value>) {
    let output: ToolOutput = module
        .call_tool(name, args, &CallContext::new(RunId::new()))
        .await
        .expect("tool error is returned as ToolOutput");
    assert!(
        output.is_error,
        "expected {name} to fail, got ok: {:?}",
        output.content
    );
    match output.content.as_slice() {
        [Content::Text { text }] => (text.clone(), None),
        [Content::Text { text }, Content::Json { value }] => (text.clone(), Some(value.clone())),
        other => panic!("unexpected error output for `{name}`: {other:?}"),
    }
}

struct RecordedCommand {
    op: String,
    args: Value,
}

type AckHandler = Arc<dyn Fn(&str, &Value) -> (bool, Value, Option<String>) + Send + Sync>;

struct CommandFakeMod {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
    recorded: Arc<Mutex<Vec<RecordedCommand>>>,
}

impl CommandFakeMod {
    async fn start(dir: &Path, handler: AckHandler) -> Self {
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let recorded_clone = Arc::clone(&recorded);
        let (stop, mut stopped) = watch::channel(false);
        let dir_owned: PathBuf = dir.to_owned();

        let task = tokio::spawn(async move {
            loop {
                if let Ok(bytes) = fs::read(dir_owned.join("command.json")).await
                    && let Ok(cmd) = serde_json::from_slice::<Value>(&bytes)
                {
                    let id = cmd["id"].clone();
                    let op = cmd["op"].as_str().unwrap_or_default().to_owned();
                    let args = cmd["args"].clone();
                    recorded_clone.lock().await.push(RecordedCommand {
                        op: op.clone(),
                        args: args.clone(),
                    });

                    let (ok, result, error) = handler(&op, &args);
                    let ack = json!({
                        "v": 1,
                        "id": id,
                        "ok": ok,
                        "result": if ok { result } else { Value::Null },
                        "error": error,
                    });
                    write_json_atomic(&dir_owned.join("ack.json"), &ack).await;
                    let _ = fs::remove_file(dir_owned.join("command.json")).await;
                }

                tokio::select! {
                    res = stopped.changed() => {
                        if res.is_err() || *stopped.borrow() {
                            return;
                        }
                    }
                    () = tokio::time::sleep(Duration::from_millis(5)) => {}
                }
            }
        });

        Self {
            stop,
            task,
            recorded,
        }
    }

    async fn recorded(&self) -> Vec<(String, Value)> {
        self.recorded
            .lock()
            .await
            .iter()
            .map(|c| (c.op.clone(), c.args.clone()))
            .collect()
    }

    async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

#[tokio::test]
async fn read_game_state_covers_all_six_scene_fixtures_with_data_enrichment() {
    let export = TestDir::new();
    build_merged_fixture_export(export.path());

    // 1. Field scene: auto-selects ["field", "party"], resolves map, chest, exit, party, objective.
    {
        let bridge = TestDir::new();
        write_v1_fixture(bridge.path(), "field.json").await;
        let module = module_with_data(bridge.path(), export.path());

        let (text, val) = call_data_ok(&module, "read_game_state", json!({})).await;
        assert_eq!(val["ok"], true);
        assert_eq!(val["screen_kind"], "field");
        assert_eq!(val["sections"], json!(["field", "party"]));
        assert!(
            text.contains("野外，地图 3001（微光洞窟），可交互：宝箱（未开）"),
            "{text}"
        );
        assert!(text.contains("(120.5, -340.0, 10.0)"), "{text}");
        assert!(text.contains("朝向 北"), "{text}");
        assert!(text.contains("可操作"), "{text}");
        assert!(text.contains("清露草药"), "{text}");
        assert!(text.contains("微风港镇"), "{text}");
        assert!(text.contains("第 1 章「启程之风」"), "{text}");
        assert!(text.contains("探索微光洞窟深处"), "{text}");
        assert!(text.contains("合成英雄"), "{text}");
        assert!(text.contains("玄铁大剑"), "{text}");
        assert!(text.contains("操作角色：合成英雄"), "{text}");
        assert_eq!(val["state"]["field"]["map_id"], 3001);
        assert_eq!(val["state"]["field"]["map_name"], "微光洞窟");
        assert_eq!(val["state"]["field"]["player"]["unit_id"], 10);
        assert_eq!(val["state"]["field"]["player"]["character_unit_id"], 9001);
        assert_eq!(
            val["state"]["field"]["player"]["character_name"],
            "合成英雄"
        );
        assert_eq!(
            val["state"]["field"]["quest_targets"]["main_quest_name"],
            "第1章启程之风"
        );
        assert_eq!(val["state"]["field"]["interact_target"]["is_open"], false);
        assert_eq!(val["state"]["field"]["chests"][1]["is_trap"], true);
        assert_eq!(val["state"]["party"]["money"], 1250);
        assert_eq!(val["state"]["party"]["main_quest_id"], 1001);
        assert_eq!(val["state"]["party"]["main_quest_name"], "第1章启程之风");
        assert_eq!(val["state"]["party"]["members"][0]["name"], "合成英雄");
        assert_eq!(
            val["state"]["party"]["main_objective"]["goal"],
            "探索微光洞窟深处"
        );
        assert_eq!(
            val["state"]["party"]["main_objective"]["main_quest_id"],
            1001
        );
        assert_eq!(val["data_loaded"]["map_unit_playable"], true);
    }

    // 2. Battle command menu scene: auto-selects ["battle"], resolves command & danger warning.
    {
        let bridge = TestDir::new();
        write_v1_fixture(bridge.path(), "battle-cmd.json").await;
        let module = module_with_data(bridge.path(), export.path());

        let (text, val) = call_data_ok(&module, "read_game_state", json!({})).await;
        assert_eq!(val["screen_kind"], "battle");
        assert_eq!(val["sections"], json!(["battle"]));
        assert!(
            text.contains("战斗中，轮到合成英雄选指令（光标在「招式」）"),
            "{text}"
        );
        assert!(text.contains("炽核爆裂"), "{text}");
        assert!(text.contains("破防槽 350/500"), "{text}");
        assert!(text.contains("破防槽 40/60"), "{text}");
        assert_eq!(val["data_loaded"]["command_skills"], true);
        assert_eq!(val["state"]["battle"]["can_act"], true);
        assert_eq!(val["state"]["battle"]["cmd"]["focus_action_id"], 1);
        assert_eq!(val["state"]["battle"]["cmd"]["focus_action_name"], "招式");
        assert_eq!(
            val["state"]["battle"]["cmd"]["primary_actions"][2]["name"],
            "道具"
        );
        assert_eq!(
            val["state"]["battle"]["risk_warnings"][0]["scheduled_skill_name"],
            "炽核爆裂"
        );
    }

    // 3. Battle target selection scene: resolves skill, focused target, AoE toggle.
    {
        let bridge = TestDir::new();
        write_v1_fixture(bridge.path(), "battle-target.json").await;
        let module = module_with_data(bridge.path(), export.path());

        let (text, val) = call_data_ok(&module, "read_game_state", json!({})).await;
        assert!(
            text.contains("选目标（招式「试作冰刺」，光标在 熔核守护者[uid=3]）"),
            "{text}"
        );
        assert!(text.contains("可切换全体化"), "{text}");
        assert_eq!(val["state"]["battle"]["cmd"]["state"], "Target");
        assert_eq!(
            val["state"]["battle"]["cmd"]["target"]["skill_name"],
            "试作冰刺"
        );
        assert_eq!(
            val["state"]["battle"]["cmd"]["target"]["focus_name"],
            "熔核守护者"
        );
        assert_eq!(val["state"]["battle"]["cmd"]["target"]["can_aoe"], true);
        assert_eq!(val["state"]["battle"]["cmd"]["target"]["is_all"], false);
    }

    // 4. Battle break / Additional phase scene: shows break, LB ready, secondary skill list.
    {
        let bridge = TestDir::new();
        write_v1_fixture(bridge.path(), "battle-break.json").await;
        let module = module_with_data(bridge.path(), export.path());

        let (text, val) = call_data_ok(&module, "read_game_state", json!({})).await;
        assert!(text.contains("追加阶段"), "{text}");
        assert!(text.contains("破防中"), "{text}");
        assert!(text.contains("试作冰刺"), "{text}");
        assert!(text.contains("已全部揭示"), "{text}");
        assert!(text.contains("LB 100/100（Lv.1）"), "{text}");
        assert_eq!(val["state"]["battle"]["phase"], "Additional");
        assert_eq!(val["state"]["battle"]["enemies"][0]["is_break"], true);
        assert_eq!(val["state"]["battle"]["enemies"][0]["bp"], 0);
        assert_eq!(
            val["state"]["battle"]["cmd"]["secondary_actions"][0]["name"],
            "试作冰刺"
        );
    }

    // 5. Talk scene: auto-selects ["ui", "field"], headline has speaker & waiting_key.
    {
        let bridge = TestDir::new();
        write_v1_fixture(bridge.path(), "talk.json").await;
        let module = module_with_data(bridge.path(), export.path());

        let (text, val) = call_data_ok(&module, "read_game_state", json!({})).await;
        assert_eq!(val["screen_kind"], "talk");
        assert_eq!(val["sections"], json!(["ui", "field"]));
        assert!(text.contains("对话中，说话人 灯塔守卫，等待按键"), "{text}");
        assert!(text.contains("欢迎来到微风港镇"), "{text}");
        assert_eq!(val["state"]["ui"]["talk"]["speaker"], "灯塔守卫");
        assert_eq!(val["state"]["ui"]["talk"]["waiting_key"], true);
        assert_eq!(val["state"]["field"]["can_control"], false);
    }

    // 6. Shop scene: auto-selects ["ui", "party"], resolves shop & item names & money.
    {
        let bridge = TestDir::new();
        write_v1_fixture(bridge.path(), "shop.json").await;
        let module = module_with_data(bridge.path(), export.path());

        let (text, val) = call_data_ok(&module, "read_game_state", json!({})).await;
        assert_eq!(val["screen_kind"], "shop");
        assert_eq!(val["sections"], json!(["ui", "party"]));
        assert!(
            text.contains("商店中，商店 1（港镇道具店）（购买，光标在「旅人帐篷」，金币 1250G）"),
            "{text}"
        );
        assert!(text.contains("清露草药"), "{text}");
        assert!(text.contains("旅人帐篷"), "{text}");
        assert_eq!(val["state"]["ui"]["shop"]["shop_name"], "港镇道具店");
        assert_eq!(val["state"]["ui"]["shop"]["tooltip_item_name"], "旅人帐篷");
        assert_eq!(val["state"]["party"]["money"], 1250);

        // Explicit `sections` overrides auto selection.
        let (_, custom) = call_data_ok(
            &module,
            "read_game_state",
            json!({ "sections": ["field", "battle", "ui", "party"] }),
        )
        .await;
        assert_eq!(
            custom["sections"],
            json!(["field", "battle", "ui", "party"])
        );
        assert_eq!(custom["state"]["battle"]["in_battle"], false);
    }
}

#[tokio::test]
async fn weakness_fairness_switch_hides_unrevealed_weaknesses_by_default_and_reveals_when_enabled()
{
    let export = TestDir::new();
    build_merged_fixture_export(export.path());
    let bridge = TestDir::new();
    write_v1_fixture(bridge.path(), "battle-cmd.json").await;

    // Default: fair mode (`reveal_hidden_weaknesses == false`).
    let fair_module = module_with_data(bridge.path(), export.path());
    assert!(!fair_module.reveal_hidden_weaknesses());
    let (fair_text, fair_json) = call_data_ok(&fair_module, "read_game_state", json!({})).await;
    assert_eq!(fair_json["reveal_hidden_weaknesses"], false);

    let enemies = fair_json["state"]["battle"]["enemies"]
        .as_array()
        .expect("enemies array");
    assert_eq!(enemies.len(), 2);

    // FrostWingAlpha (9101): Fire revealed, Thunder unrevealed; Ice Halved & Water Invalid unrevealed.
    assert_eq!(enemies[0]["weaknesses"]["revealed"], json!(["火"]));
    assert_eq!(enemies[0]["weaknesses"]["unrevealed_count"], 1);
    assert_eq!(enemies[0]["weaknesses"]["hidden_revealed"], false);
    assert!(enemies[0]["weaknesses"].get("unrevealed").is_none());
    assert!(enemies[0]["weaknesses"].get("all").is_none());
    assert_eq!(enemies[0]["resistances"]["revealed"], json!([]));
    assert_eq!(enemies[0]["resistances"]["unrevealed_count"], 2);
    assert!(enemies[0]["resistances"].get("unrevealed").is_none());

    // CoreGuardian (9201): Ice revealed, Water unrevealed; Fire Absorb & Earth Halved unrevealed.
    assert_eq!(enemies[1]["weaknesses"]["revealed"], json!(["冰"]));
    assert_eq!(enemies[1]["weaknesses"]["unrevealed_count"], 1);
    assert_eq!(enemies[1]["weaknesses"]["hidden_revealed"], false);
    assert!(enemies[1]["weaknesses"].get("unrevealed").is_none());
    assert_eq!(enemies[1]["resistances"]["revealed"], json!([]));
    assert_eq!(enemies[1]["resistances"]["unrevealed_count"], 2);

    // Neither enemy summary text nor JSON leaks unrevealed elements ("雷"/"Thunder", "水"/"Water", "土"/"Earth", "吸收"/"Absorb").
    let fair_enemy_section = fair_text
        .split_once("- 敌方：")
        .map(|(_, rest)| rest)
        .expect("enemy section in fair summary");
    let enemies_serialized =
        serde_json::to_string(&fair_json["state"]["battle"]["enemies"]).expect("serialize enemies");
    assert!(
        fair_enemy_section.contains(
            "已揭示弱点：火（数据表另有 1 个未揭示弱点，完整弱点可用 read_enemy(unit_id=9101) 查）"
        ),
        "{fair_enemy_section}"
    );
    assert!(
        fair_enemy_section.contains(
            "已揭示弱点：冰（数据表另有 1 个未揭示弱点，完整弱点可用 read_enemy(unit_id=9201) 查）"
        ),
        "{fair_enemy_section}"
    );
    for forbidden in [
        "雷", "Thunder", "水", "Water", "土", "Earth", "吸收", "Absorb",
    ] {
        assert!(
            !fair_enemy_section.contains(forbidden),
            "fair enemy summary leaked `{forbidden}`:\n{fair_enemy_section}"
        );
        assert!(
            !enemies_serialized.contains(forbidden),
            "fair JSON leaked `{forbidden}`:\n{enemies_serialized}"
        );
    }

    // Opt-in: `--reveal-hidden-weaknesses` enabled.
    let open_module =
        module_with_data(bridge.path(), export.path()).with_reveal_hidden_weaknesses(true);
    assert!(open_module.reveal_hidden_weaknesses());
    let (open_text, open_json) = call_data_ok(&open_module, "read_game_state", json!({})).await;
    assert_eq!(open_json["reveal_hidden_weaknesses"], true);

    let open_enemies = open_json["state"]["battle"]["enemies"]
        .as_array()
        .expect("open enemies");
    assert_eq!(open_enemies[0]["weaknesses"]["revealed"], json!(["火"]));
    assert_eq!(open_enemies[0]["weaknesses"]["unrevealed"], json!(["雷"]));
    assert_eq!(open_enemies[0]["weaknesses"]["all"], json!(["火", "雷"]));
    assert_eq!(open_enemies[1]["weaknesses"]["revealed"], json!(["冰"]));
    assert_eq!(open_enemies[1]["weaknesses"]["unrevealed"], json!(["水"]));
    assert_eq!(open_enemies[1]["weaknesses"]["all"], json!(["冰", "水"]));
    assert_eq!(
        open_enemies[1]["resistances"]["unrevealed"][0]["resistance_zh"],
        "吸收"
    );
    assert!(
        open_text.contains("未揭示弱点（已开启揭示开关）：雷"),
        "{open_text}"
    );
    assert!(
        open_text.contains("未揭示弱点（已开启揭示开关）：水"),
        "{open_text}"
    );
    assert!(open_text.contains("火(吸收)"), "{open_text}");

    // Works cleanly without static data directory as well (counts unrevealed from runtime state).
    let no_data_module = module(bridge.path());
    let (nodata_text, nodata_json) =
        call_data_ok(&no_data_module, "read_game_state", json!({})).await;
    assert_eq!(nodata_json["data_loaded"]["battle"], false);
    assert_eq!(
        nodata_json["state"]["battle"]["enemies"][1]["weaknesses"]["unrevealed_count"],
        1
    );
    let nodata_enemy_section = nodata_text
        .split_once("- 敌方：")
        .map(|(_, rest)| rest)
        .expect("enemy section without data");
    assert!(
        !nodata_enemy_section.contains("水"),
        "{nodata_enemy_section}"
    );
}

#[tokio::test]
async fn read_game_state_reports_stale_and_missing_bridge_honestly_and_handles_v0_state() {
    // 1. Missing state.json.
    {
        let bridge = TestDir::new();
        let module = module(bridge.path());
        let (text, json_err) = call_status_err(&module, "read_game_state", json!({})).await;
        assert!(text.contains("文件不存在"), "{text}");
        assert_eq!(json_err.expect("json error")["stale"], false);
    }

    // 2. Stale state.json: refuses to return old game data and reports "状态停止更新".
    {
        let bridge = TestDir::new();
        write_v1_fixture(bridge.path(), "field.json").await;
        let module = module(bridge.path());

        let (_, first) = call_data_ok(&module, "read_game_state", json!({})).await;
        assert_eq!(first["ok"], true);
        tokio::time::sleep(Duration::from_millis(150)).await;

        let (text, json_err) = call_status_err(&module, "read_game_state", json!({})).await;
        assert!(text.contains("状态停止更新"), "{text}");
        assert!(
            !text.contains("微光洞窟"),
            "must not serve stale data: {text}"
        );
        let err_val = json_err.expect("stale error JSON");
        assert_eq!(err_val["ok"], false);
        assert_eq!(err_val["stale"], true);
        assert!(err_val.get("state").is_none());
    }

    // 3. v0 state.json (no v1 topics) degrades cleanly without error.
    {
        let bridge = TestDir::new();
        write_state(bridge.path(), "v0-session", 1).await;
        let module = module(bridge.path());
        let (text, val) = call_data_ok(&module, "read_game_state", json!({})).await;
        assert_eq!(val["bridge_version"], "0");
        assert!(text.contains("ExampleMap"), "{text}");
    }

    // 4. Invalid arguments fail cleanly.
    {
        let bridge = TestDir::new();
        let module = module(bridge.path());
        let (err1, _) =
            call_status_err(&module, "read_game_state", json!({ "sections": [] })).await;
        assert!(err1.contains("不能为空数组"), "{err1}");
        let (err2, _) =
            call_status_err(&module, "read_game_state", json!({ "sections": ["bogus"] })).await;
        assert!(err2.contains("不支持主题"), "{err2}");
    }
}

#[tokio::test]
async fn bridge_command_tools_forward_to_mod_and_translate_results() {
    let bridge = TestDir::new();
    let export = TestDir::new();
    build_merged_fixture_export(export.path());

    let handler: AckHandler = Arc::new(|op, args| match op {
        "save_slots" => (
            true,
            json!({
                "slot_count": 3,
                "last_saved_slot": 1,
                "slots": [
                    {
                        "slot_index": 0,
                        "is_auto_save": true,
                        "exists": true,
                        "load_status": 0,
                        "main_progress": 1020,
                        "chapter": 1,
                        "current_map_id": 3001,
                        "play_level": 5,
                        "play_time": 1829.5,
                        "date_time_ticks": "638990000000000000",
                        "difficulty": 1,
                        "is_cleared": false,
                        "party_unit_ids": [9001]
                    },
                    {
                        "slot_index": 1,
                        "is_auto_save": false,
                        "exists": true,
                        "load_status": 0,
                        "main_progress": 1020,
                        "chapter": 1,
                        "current_map_id": 2001,
                        "play_level": 5,
                        "play_time": 1800.0,
                        "date_time_ticks": "638989990000000000",
                        "difficulty": 1,
                        "is_cleared": false,
                        "party_unit_ids": [9001]
                    },
                    {
                        "slot_index": 2,
                        "is_auto_save": false,
                        "exists": false,
                        "load_status": 7,
                        "main_progress": null,
                        "chapter": null,
                        "current_map_id": null,
                        "play_level": null,
                        "play_time": null,
                        "date_time_ticks": null,
                        "difficulty": null,
                        "is_cleared": null,
                        "party_unit_ids": []
                    }
                ]
            }),
            None,
        ),
        "inventory" => (
            true,
            json!({
                "count": 2,
                "total_qty": 6,
                "hash": "aabbccdd",
                "items": [
                    { "item_id": 1, "num": 5 },
                    { "item_id": 10100, "num": 1 }
                ]
            }),
            None,
        ),
        "event_flags" => (
            true,
            json!({
                "total": 2048,
                "true_count": 3,
                "hash": "11223344",
                "true_ids": [14, 25, 105]
            }),
            None,
        ),
        "input_state" => (
            true,
            json!({
                "active_imcs": ["MapUnit", "UI_Field"],
                "swap_decide": false,
                "input_enabled": true,
                "input_blocked": ["WindowDeactivate"],
                "key_bindings": {
                    "UICommon_Decide": "Enter",
                    "MapUnit_Interact": "SpaceBar"
                }
            }),
            None,
        ),
        "query_nav_path" => {
            let goal_x = args["goal"]["x"].as_f64().unwrap_or(0.0);
            if goal_x > 900.0 {
                (
                    true,
                    json!({ "valid": false, "partial": false, "length": 0.0, "points": [] }),
                    None,
                )
            } else if goal_x > 500.0 {
                (false, Value::Null, Some("导航网格未构建".to_owned()))
            } else {
                (
                    true,
                    json!({
                        "valid": true,
                        "partial": false,
                        "length": 250.5,
                        "points": [
                            { "x": 0.0, "y": 0.0, "z": 0.0 },
                            { "x": 120.0, "y": 80.0, "z": 0.0 },
                            { "x": 200.0, "y": 150.0, "z": 0.0 }
                        ]
                    }),
                    None,
                )
            }
        }
        other => (false, Value::Null, Some(format!("未知命令 {other}"))),
    });

    let fake_mod = CommandFakeMod::start(bridge.path(), handler).await;
    let module = module_with_data(bridge.path(), export.path());

    // 1. read_save_slots
    let (slots_text, slots_json) = call_data_ok(&module, "read_save_slots", json!({})).await;
    assert!(
        slots_text.contains("共 3 个（已占用 2 个，空槽 1 个；最近存档：槽 1）"),
        "{slots_text}"
    );
    assert!(slots_text.contains("启程之风"), "{slots_text}");
    assert!(slots_text.contains("微光洞窟"), "{slots_text}");
    assert!(slots_text.contains("合成英雄"), "{slots_text}");
    assert!(slots_text.contains("空槽位：2"), "{slots_text}");
    assert_eq!(slots_json["slots"][0]["current_map_name"], "微光洞窟");
    assert_eq!(slots_json["slots"][0]["chapter_name"], "启程之风");
    assert_eq!(
        slots_json["slots"][0]["party_members"][0]["name"],
        "合成英雄"
    );

    // 2. read_inventory
    let (inv_text, inv_json) = call_data_ok(&module, "read_inventory", json!({})).await;
    assert!(
        inv_text.contains("清露草药（#1） [消耗品] ×5"),
        "{inv_text}"
    );
    assert!(
        inv_text.contains("玄铁大剑（#10100） [装备] ×1"),
        "{inv_text}"
    );
    assert_eq!(inv_json["count"], 2);
    assert_eq!(inv_json["items"][0]["name"], "清露草药");
    assert_eq!(inv_json["items"][1]["category_zh"], "装备");

    // 3. read_event_flags (full and filtered)
    let (flags_text, flags_json) = call_data_ok(&module, "read_event_flags", json!({})).await;
    assert!(
        flags_text.contains("共 2048 位，已置位 3 个"),
        "{flags_text}"
    );
    assert!(flags_text.contains("14、25、105"), "{flags_text}");
    assert_eq!(flags_json["true_ids"], json!([14, 25, 105]));

    let (checked_text, checked_json) =
        call_data_ok(&module, "read_event_flags", json!({ "flag_ids": [25, 99] })).await;
    assert!(checked_text.contains("25=开、99=关"), "{checked_text}");
    assert!(checked_text.contains("SwitchReady"), "{checked_text}");
    assert_eq!(
        checked_json["checked"],
        json!([
            { "flag_id": 25, "is_set": true },
            { "flag_id": 99, "is_set": false }
        ])
    );
    let related = checked_json["related_conditions"]
        .as_array()
        .expect("related_conditions array");
    assert!(
        related
            .iter()
            .any(|c| c["name"] == "SwitchReady" && c["satisfied"] == true)
    );
    assert!(
        related
            .iter()
            .any(|c| c["name"] == "EventCombo" && c["satisfied"] == true)
    );

    // 4. read_input_state
    let (input_text, input_json) = call_data_ok(
        &module,
        "read_input_state",
        json!({ "mapping_names": ["UICommon_Decide", "MapUnit_Interact"], "slot": 0 }),
    )
    .await;
    assert!(input_text.contains("窗口失焦"), "{input_text}");
    assert!(input_text.contains("MapUnit"), "{input_text}");
    assert!(input_text.contains("UICommon_Decide=Enter"), "{input_text}");
    assert_eq!(input_json["window_deactivated"], true);
    assert_eq!(input_json["key_bindings"]["MapUnit_Interact"], "SpaceBar");

    // 5. query_nav_path (reachable, unreachable, and mod error ack)
    let (nav_text, nav_json) = call_data_ok(
        &module,
        "query_nav_path",
        json!({
            "start": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "goal": { "x": 200.0, "y": 150.0, "z": 0.0 }
        }),
    )
    .await;
    assert!(nav_text.contains("导航路径可达"), "{nav_text}");
    assert!(nav_text.contains("250.5"), "{nav_text}");
    assert_eq!(nav_json["valid"], true);
    assert_eq!(nav_json["point_count"], 3);

    let (unreach_text, unreach_json) = call_data_ok(
        &module,
        "query_nav_path",
        json!({
            "start": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "goal": { "x": 999.0, "y": 0.0, "z": 0.0 }
        }),
    )
    .await;
    assert!(unreach_text.contains("导航路径不可达"), "{unreach_text}");
    assert_eq!(unreach_json["valid"], false);

    let (mod_err_text, mod_err_json) = call_status_err(
        &module,
        "query_nav_path",
        json!({
            "start": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "goal": { "x": 600.0, "y": 0.0, "z": 0.0 }
        }),
    )
    .await;
    assert!(mod_err_text.contains("导航网格未构建"), "{mod_err_text}");
    assert_eq!(mod_err_json.expect("err json")["ok"], false);

    // Invalid arguments fail immediately without sending a command.
    let (bad_args, _) = call_status_err(
        &module,
        "query_nav_path",
        json!({ "start": { "x": 0.0, "y": 0.0 } }),
    )
    .await;
    assert!(bad_args.contains("start.z"), "{bad_args}");

    let recorded = fake_mod.recorded().await;
    let ops: Vec<&str> = recorded.iter().map(|(op, _)| op.as_str()).collect();
    assert_eq!(
        ops,
        vec![
            "save_slots",
            "inventory",
            "event_flags",
            "event_flags",
            "input_state",
            "query_nav_path",
            "query_nav_path",
            "query_nav_path",
        ]
    );
    assert_eq!(recorded[4].1["slot"], 0);
    assert_eq!(
        recorded[4].1["mapping_names"],
        json!(["UICommon_Decide", "MapUnit_Interact"])
    );

    fake_mod.stop().await;

    // Without mod running, bridge command reports timeout as tool error in Chinese.
    let (timeout_text, timeout_json) = call_status_err(&module, "read_inventory", json!({})).await;
    assert!(
        timeout_text.contains("没有回复命令 inventory"),
        "{timeout_text}"
    );
    assert_eq!(timeout_json.expect("timeout json")["ok"], false);
}
