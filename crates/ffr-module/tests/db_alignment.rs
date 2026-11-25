//! Byte-for-byte alignment tests verifying that `ffr-db` SQLite backend and legacy JSON
//! (`GameDataCache`) backend produce strictly identical outputs (both Chinese summary text and
//! structured JSON) across all 9 data tools, all `ffr://` resources, live-state enrichment
//! (`read_game_state` with `--reveal-hidden-weaknesses` both off and on), and on-demand bridge
//! command enrichment.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agp_core::RunId;
use agp_game::{CallContext, Content, GameModule, ToolOutput};
use common::{
    TestDir, build_fixture_db_from_export, build_merged_fixture_export, fixture_battle_export,
    load_bridge_v1_fixture, module_with_data, module_with_db, read_all_resource_text,
    write_json_atomic,
};
use ffr_module::FfrModule;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::task::JoinHandle;

fn inject_extra_alignment_items(export_root: &Path) {
    let table_path = export_root.join("FFRS/Content/Datatable/Item/DT_ConsumablesItemData.json");
    let raw = fs::read_to_string(&table_path).expect("read consumables fixture");
    let mut doc: Value = serde_json::from_str(&raw).expect("parse consumables fixture");
    let rows = doc[0]["Rows"].as_object_mut().expect("Rows object");

    let loc_path = export_root.join("Localization/zh-Hans.json");
    let loc_raw = fs::read_to_string(&loc_path).expect("read zh-Hans loc");
    let mut loc: Value = serde_json::from_str(&loc_raw).expect("parse zh-Hans loc");
    let ns = loc
        .as_object_mut()
        .expect("loc object")
        .entry("AlignTest")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("AlignTest namespace");

    let mut add_item = |id: i64, key: &str, zh_name: &str| {
        ns.insert(key.to_owned(), json!(zh_name));
        rows.insert(
            format!("Align_{id}"),
            json!({
                "ID": id,
                "SortId": id,
                "ItemType": "eItemType::Consumables",
                "Name": { "Namespace": "AlignTest", "Key": key },
                "Description": { "CultureInvariantString": null },
                "buyingPrice": 50,
                "sellingPrice": 25,
                "isAvailableSale": true,
                "usableScene": "eItemUsableScene::Any",
                "isConsumedAfterUse": true,
                "SkillId": -1,
                "throwSkillData": { "isThrow": false, "SkillId": -1 }
            }),
        );
    };

    for i in 1..=25 {
        add_item(40000 + i, &format!("Potion_{i}"), &format!("测试药剂{i}"));
    }
    add_item(50001, "DupElixir_1", "重名秘药");
    add_item(50002, "DupElixir_2", "重名秘药");
    // Trailing full-width space edge case mirroring real `ST_EquipmentName` entries.
    add_item(50003, "TrailingSpace", "碎星锤\u{3000}");

    fs::write(
        table_path,
        serde_json::to_vec_pretty(&doc).expect("serialize consumables"),
    )
    .expect("write consumables");
    fs::write(
        loc_path,
        serde_json::to_vec_pretty(&loc).expect("serialize zh-Hans loc"),
    )
    .expect("write zh-Hans loc");
}

fn normalize_tool_output(output: &ToolOutput, strip_file_age: bool) -> (bool, Vec<Content>) {
    let content = output
        .content
        .iter()
        .map(|item| match item {
            Content::Json { value } if strip_file_age => {
                let mut cloned = value.clone();
                if let Some(obj) = cloned.as_object_mut()
                    && obj.contains_key("file_age_ms")
                {
                    obj.insert("file_age_ms".to_owned(), json!(0));
                }
                Content::Json { value: cloned }
            }
            other => other.clone(),
        })
        .collect();
    (output.is_error, content)
}

async fn assert_tool_aligned(
    db_mod: &FfrModule,
    json_mod: &FfrModule,
    tool: &str,
    args: Value,
    strip_file_age: bool,
) {
    let db_out = db_mod
        .call_tool(tool, args.clone(), &CallContext::new(RunId::new()))
        .await
        .expect("db tool call returns ToolOutput");
    let json_out = json_mod
        .call_tool(tool, args.clone(), &CallContext::new(RunId::new()))
        .await
        .expect("json tool call returns ToolOutput");

    let db_norm = normalize_tool_output(&db_out, strip_file_age);
    let json_norm = normalize_tool_output(&json_out, strip_file_age);
    assert_eq!(
        db_norm, json_norm,
        "DB vs JSON mismatch for tool `{tool}` with args {args}"
    );
}

#[tokio::test]
async fn all_data_tools_and_resources_align_verbatim_between_db_and_json() {
    let temp = TestDir::new();
    let bridge_dir = temp.path().join("bridge");
    let export_dir = temp.path().join("export");
    let db_path = temp.path().join("ffr.sqlite");
    build_merged_fixture_export(&export_dir);
    inject_extra_alignment_items(&export_dir);
    build_fixture_db_from_export(&export_dir, &db_path);

    let db_mod = module_with_db(&bridge_dir, &db_path);
    let json_mod = module_with_data(&bridge_dir, &export_dir);

    let cases: Vec<(&str, Value)> = vec![
        // read_chapter
        ("read_chapter", json!({})),
        ("read_chapter", json!({ "chapter": 1 })),
        ("read_chapter", json!({ "chapter": 99 })),
        ("read_chapter", json!({ "chapter": 0 })),
        // read_map
        ("read_map", json!({ "map_id": 3001 })),
        ("read_map", json!({ "map_id": 2001 })),
        (
            "read_map",
            json!({ "level_name": "Town_01Fake_01_PL:PersistentLevel" }),
        ),
        ("read_map", json!({ "map_id": 99999 })),
        ("read_map", json!({ "level_name": "NoSuchLevel_PL" })),
        ("read_map", json!({})),
        // read_enemy
        ("read_enemy", json!({ "unit_id": 9201 })),
        ("read_enemy", json!({ "unit_id": 9101 })),
        ("read_enemy", json!({ "unit_id": 9202 })),
        ("read_enemy", json!({ "unit_id": 9001 })),
        ("read_enemy", json!({ "name": "熔核守护者" })),
        ("read_enemy", json!({ "name": "霜翼试作兽" })),
        ("read_enemy", json!({ "name": "霜翼" })),
        ("read_enemy", json!({ "name": "完全不存在" })),
        ("read_enemy", json!({ "unit_id": 99999 })),
        // read_battle_group
        ("read_battle_group", json!({ "group_id": 9801 })),
        ("read_battle_group", json!({ "group_id": 9802 })),
        ("read_battle_group", json!({ "group_id": 9803 })),
        ("read_battle_group", json!({ "group_id": 99999 })),
        // read_skill
        ("read_skill", json!({ "skill_id": 95001 })),
        ("read_skill", json!({ "skill_id": 95004 })),
        ("read_skill", json!({ "name": "背水一击" })),
        ("read_skill", json!({ "name": "冰刺" })),
        ("read_skill", json!({ "name": "完全不存在" })),
        ("read_skill", json!({ "skill_id": 99999 })),
        // read_item
        ("read_item", json!({ "item_id": 10100 })),
        ("read_item", json!({ "item_id": 1 })),
        ("read_item", json!({ "item_id": 9500 })),
        ("read_item", json!({ "name": "清露草药" })),
        ("read_item", json!({ "name": "重名秘药" })),
        ("read_item", json!({ "name": "碎星锤" })),
        ("read_item", json!({ "name": "药剂" })),
        ("read_item", json!({ "name": "完全不存在" })),
        ("read_item", json!({ "item_id": 99999 })),
        // read_quest
        ("read_quest", json!({ "quest_id": 10 })),
        ("read_quest", json!({ "name": "点亮港湾灯火" })),
        ("read_quest", json!({ "name": "港湾" })),
        ("read_quest", json!({ "chapter": 1 })),
        ("read_quest", json!({ "chapter": 8 })),
        ("read_quest", json!({ "quest_id": 99999 })),
        // read_shop
        ("read_shop", json!({ "shop_id": 1 })),
        ("read_shop", json!({ "shop_id": 2001 })),
        ("read_shop", json!({ "name": "港镇道具店" })),
        ("read_shop", json!({ "name": "道具" })),
        ("read_shop", json!({ "item_id": 500 })),
        ("read_shop", json!({ "item_id": 99999 })),
        ("read_shop", json!({ "shop_id": 99999 })),
        // search_data
        ("search_data", json!({ "query": "微光" })),
        (
            "search_data",
            json!({ "query": "试作", "kinds": ["enemy", "skill"] }),
        ),
        (
            "search_data",
            json!({ "query": "测试药剂", "kinds": ["item"] }),
        ),
        ("search_data", json!({ "query": "启程" })),
        ("search_data", json!({ "query": "完全不存在" })),
    ];

    for (tool, args) in cases {
        assert_tool_aligned(&db_mod, &json_mod, tool, args, false).await;
    }

    // Resource descriptors (including exact `size_hint` byte length) must match verbatim.
    let db_res = db_mod.list_resources().await.expect("db list_resources");
    let json_res = json_mod
        .list_resources()
        .await
        .expect("json list_resources");
    assert_eq!(db_res, json_res, "list_resources mismatch");

    // Resource bodies must match byte-for-byte across all catalog URIs.
    for uri in [
        "ffr://chapters",
        "ffr://maps",
        "ffr://enemies",
        "ffr://enemies?chapter=1",
        "ffr://enemies?chapter=8",
        "ffr://skills",
        "ffr://db/schema",
    ] {
        let db_body = read_all_resource_text(&db_mod, uri).await;
        let json_body = read_all_resource_text(&json_mod, uri).await;
        assert_eq!(db_body, json_body, "resource `{uri}` mismatch");
    }
}

#[tokio::test]
async fn partial_battle_only_fixture_aligns_null_enemy_chapters_and_resource_hints() {
    let temp = TestDir::new();
    let bridge_dir = temp.path().join("bridge");
    let db_path = temp.path().join("battle-only.sqlite");
    build_fixture_db_from_export(&fixture_battle_export(), &db_path);

    let db_mod = module_with_db(&bridge_dir, &db_path);
    let json_mod = module_with_data(&bridge_dir, fixture_battle_export());

    assert_tool_aligned(
        &db_mod,
        &json_mod,
        "read_enemy",
        json!({ "unit_id": 9201 }),
        false,
    )
    .await;
    assert_tool_aligned(
        &db_mod,
        &json_mod,
        "read_enemy",
        json!({ "unit_id": 9101 }),
        false,
    )
    .await;

    let db_res = db_mod.list_resources().await.expect("db list_resources");
    let json_res = json_mod
        .list_resources()
        .await
        .expect("json list_resources");
    assert_eq!(db_res, json_res, "partial fixture list_resources mismatch");

    for uri in ["ffr://enemies", "ffr://skills"] {
        let db_body = read_all_resource_text(&db_mod, uri).await;
        let json_body = read_all_resource_text(&json_mod, uri).await;
        assert_eq!(db_body, json_body, "partial resource `{uri}` mismatch");
    }
}

#[tokio::test]
async fn live_state_enrichment_aligns_verbatim_with_fairness_switch_off_and_on() {
    let temp = TestDir::new();
    let export_dir = temp.path().join("export");
    let db_path = temp.path().join("ffr.sqlite");
    build_merged_fixture_export(&export_dir);
    build_fixture_db_from_export(&export_dir, &db_path);

    let fixtures = [
        "field.json",
        "battle-cmd.json",
        "battle-target.json",
        "battle-break.json",
        "talk.json",
        "shop.json",
    ];

    for fixture_name in fixtures {
        let bridge_dir = TestDir::new();
        let value = load_bridge_v1_fixture(fixture_name);
        write_json_atomic(&bridge_dir.path().join("state.json"), &value).await;

        for reveal_hidden in [false, true] {
            let db_mod = module_with_db(bridge_dir.path(), &db_path)
                .with_reveal_hidden_weaknesses(reveal_hidden);
            let json_mod = module_with_data(bridge_dir.path(), &export_dir)
                .with_reveal_hidden_weaknesses(reveal_hidden);

            assert_tool_aligned(&db_mod, &json_mod, "read_game_state", json!({}), true).await;
            assert_tool_aligned(
                &db_mod,
                &json_mod,
                "read_game_state",
                json!({ "sections": ["field", "battle", "ui", "party"] }),
                true,
            )
            .await;
        }
    }
}

type AckHandler = Arc<dyn Fn(&str, &Value) -> (bool, Value, Option<String>) + Send + Sync>;

struct AlignmentFakeMod {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl AlignmentFakeMod {
    async fn start(dir: &Path, handler: AckHandler) -> Self {
        let (stop, mut stopped) = watch::channel(false);
        let dir_owned: PathBuf = dir.to_owned();
        let task = tokio::spawn(async move {
            loop {
                if let Ok(bytes) = tokio::fs::read(dir_owned.join("command.json")).await
                    && let Ok(cmd) = serde_json::from_slice::<Value>(&bytes)
                {
                    let id = cmd["id"].clone();
                    let op = cmd["op"].as_str().unwrap_or_default().to_owned();
                    let args = cmd["args"].clone();
                    let (ok, result, error) = handler(&op, &args);
                    let ack = json!({
                        "v": 1,
                        "id": id,
                        "ok": ok,
                        "result": if ok { result } else { Value::Null },
                        "error": error,
                    });
                    write_json_atomic(&dir_owned.join("ack.json"), &ack).await;
                    let _ = tokio::fs::remove_file(dir_owned.join("command.json")).await;
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
        Self { stop, task }
    }

    async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

#[tokio::test]
async fn bridge_command_enrichment_aligns_verbatim_between_db_and_json() {
    let temp = TestDir::new();
    let bridge_dir = temp.path().join("bridge");
    fs::create_dir_all(&bridge_dir).expect("create bridge dir");
    let export_dir = temp.path().join("export");
    let db_path = temp.path().join("ffr.sqlite");
    build_merged_fixture_export(&export_dir);
    build_fixture_db_from_export(&export_dir, &db_path);

    let handler: AckHandler = Arc::new(|op, _args| match op {
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
                        "load_status": 1,
                        "main_progress": 1020,
                        "chapter": 1,
                        "current_map_id": 3001,
                        "play_level": 8,
                        "play_time": 3725.0,
                        "date_time_ticks": "638900000000000000",
                        "difficulty": 1,
                        "is_cleared": false,
                        "party_unit_ids": [9001]
                    },
                    {
                        "slot_index": 1,
                        "is_auto_save": false,
                        "exists": true,
                        "load_status": 1,
                        "main_progress": 1020,
                        "chapter": 1,
                        "current_map_id": 2001,
                        "play_level": 8,
                        "play_time": 3600.0,
                        "date_time_ticks": "638900000000000001",
                        "difficulty": 0,
                        "is_cleared": false,
                        "party_unit_ids": [9001]
                    },
                    {
                        "slot_index": 2,
                        "is_auto_save": false,
                        "exists": false
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
        other => (false, Value::Null, Some(format!("未知命令 {other}"))),
    });

    let fake_mod = AlignmentFakeMod::start(&bridge_dir, handler).await;
    let db_mod = module_with_db(&bridge_dir, &db_path);
    let json_mod = module_with_data(&bridge_dir, &export_dir);

    assert_tool_aligned(&db_mod, &json_mod, "read_save_slots", json!({}), false).await;
    assert_tool_aligned(&db_mod, &json_mod, "read_inventory", json!({}), false).await;
    assert_tool_aligned(&db_mod, &json_mod, "read_event_flags", json!({}), false).await;
    assert_tool_aligned(
        &db_mod,
        &json_mod,
        "read_event_flags",
        json!({ "flag_ids": [25, 99] }),
        false,
    )
    .await;

    fake_mod.stop().await;
}
