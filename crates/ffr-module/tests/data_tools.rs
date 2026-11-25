mod common;

use std::fs;
use std::path::Path;

use agp_game::{Content, GameModule, ResourceRange};
use common::{
    FakeMod, TestDir, build_merged_fixture_export, call_data_err, call_data_ok, call_json,
    fixture_battle_export, fixture_data_export, fixture_world_export, module, module_with_data,
};
use serde_json::{Value, json};

#[tokio::test]
async fn scenario_fixture_supports_read_chapter_without_other_tables() {
    let bridge = TestDir::new();
    let module = module_with_data(bridge.path(), fixture_data_export());

    let (all_text, all_json) = call_data_ok(&module, "read_chapter", json!({})).await;
    assert!(all_text.contains("起始之森"));
    assert!(all_text.contains("微澜港湾"));
    assert!(all_text.contains("无中文译名"));
    assert_eq!(all_json["chapters"].as_array().map(Vec::len), Some(3));

    let (ch1_text, ch1_json) = call_data_ok(&module, "read_chapter", json!({ "chapter": 1 })).await;
    assert!(ch1_text.contains("起始之森"));
    assert!(ch1_text.contains("与村口向导交谈"));
    assert!(ch1_text.contains("2001"));
    assert_eq!(ch1_json["chapter_no"], 1);
    assert_eq!(ch1_json["name"], "起始之森");
    assert_eq!(ch1_json["main_scenario"][0]["min_progress"], 1010);
    assert_eq!(
        ch1_json["main_scenario"][0]["destinations"][0]["map_id"],
        2001
    );

    let err = call_data_err(&module, "read_chapter", json!({ "chapter": 99 })).await;
    assert!(err.contains("找不到第 99 章"), "{err}");
}

#[tokio::test]
async fn battle_fixture_covers_enemy_group_skill_and_duplicate_name_candidates() {
    let bridge = TestDir::new();
    let module = module_with_data(bridge.path(), fixture_battle_export());

    // read_enemy by unit_id (Boss with high-risk skill).
    let (boss_text, boss_json) =
        call_data_ok(&module, "read_enemy", json!({ "unit_id": 9201 })).await;
    assert!(boss_text.contains("熔核守护者"));
    assert!(boss_text.contains("Boss"));
    assert!(boss_text.contains("4200"));
    assert!(boss_text.contains("500"));
    assert!(boss_text.contains("冰、水"));
    assert!(boss_text.contains("炽核爆裂"));
    assert_eq!(boss_json["unit_id"], 9201);
    assert_eq!(boss_json["is_boss"], true);
    assert_eq!(boss_json["max_hp"], 4200);
    assert_eq!(boss_json["max_break_point"], 500);
    assert_eq!(boss_json["weaknesses"], json!(["冰", "水"]));
    assert_eq!(boss_json["resistances"]["absorb"], json!(["火"]));
    assert_eq!(boss_json["high_risk_skills"][0]["skill_id"], 95003);
    assert_eq!(boss_json["high_risk_skills"][0]["name"], "炽核爆裂");

    // read_enemy by unique name.
    let (_, boss_by_name) =
        call_data_ok(&module, "read_enemy", json!({ "name": "熔核守护者" })).await;
    assert_eq!(boss_by_name["unit_id"], 9201);

    // read_enemy by ambiguous name lists all candidates without picking one.
    let (dup_text, dup_json) =
        call_data_ok(&module, "read_enemy", json!({ "name": "霜翼试作兽" })).await;
    assert!(dup_text.contains("9101"));
    assert!(dup_text.contains("9202"));
    assert!(dup_text.contains("候选"));
    assert_eq!(dup_json["ambiguous"], true);
    let ids: Vec<i64> = dup_json["candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .map(|c| c["unit_id"].as_i64().expect("unit_id"))
        .collect();
    assert_eq!(ids, vec![9101, 9202]);

    // read_battle_group (normal, boss, and broken member reference).
    let (norm_text, norm_json) =
        call_data_ok(&module, "read_battle_group", json!({ "group_id": 9801 })).await;
    assert!(norm_text.contains("普通遭遇"));
    assert!(norm_text.contains("可逃跑"));
    assert_eq!(norm_json["is_boss"], false);
    assert_eq!(norm_json["members"].as_array().map(Vec::len), Some(2));

    let (bg_boss_text, bg_boss_json) =
        call_data_ok(&module, "read_battle_group", json!({ "group_id": 9802 })).await;
    assert!(bg_boss_text.contains("Boss 战"));
    assert!(bg_boss_text.contains("不可逃跑"));
    assert_eq!(bg_boss_json["is_boss"], true);
    assert_eq!(bg_boss_json["is_bounty"], true);
    assert_eq!(bg_boss_json["members"][0]["unit_id"], 9201);

    let broken_err = call_data_err(&module, "read_battle_group", json!({ "group_id": 9803 })).await;
    assert!(broken_err.contains("99999"), "{broken_err}");

    // read_skill by ID and by name (including HP-ratio cost override).
    let (ice_text, ice_json) =
        call_data_ok(&module, "read_skill", json!({ "skill_id": 95001 })).await;
    assert!(ice_text.contains("试作冰刺"));
    assert!(ice_text.contains("冰"));
    assert!(ice_text.contains("威力：24"));
    assert!(ice_text.contains("破防削减：30"));
    assert!(ice_text.contains("8 MP"));
    assert!(ice_text.contains("敌方单体"));
    assert_eq!(ice_json["skill_id"], 95001);
    assert_eq!(ice_json["element"], "冰");
    assert_eq!(ice_json["power"], 24);
    assert_eq!(ice_json["break_damage"], 30);
    assert_eq!(ice_json["cost"], 8);

    let (hp_text, hp_json) =
        call_data_ok(&module, "read_skill", json!({ "name": "背水一击" })).await;
    assert!(hp_text.contains("10% HP"));
    assert_eq!(hp_json["skill_id"], 95004);
    assert_eq!(hp_json["cost_type"], "HpRatio");
}

#[tokio::test]
async fn world_fixture_covers_map_item_quest_and_shop_tools() {
    let bridge = TestDir::new();
    let module = module_with_data(bridge.path(), fixture_world_export());

    // read_map by map_id and by level_name / UE path.
    let (map_text, map_json) = call_data_ok(&module, "read_map", json!({ "map_id": 3001 })).await;
    assert!(map_text.contains("微光洞窟"));
    assert!(map_text.contains("Dng_01Fake_43_PL"));
    assert!(map_text.contains("世界地图可传送"));
    assert!(map_text.contains("存档/帐篷/迷宫内传送"));
    assert!(map_text.contains("宝箱明细"));
    assert_eq!(map_json["map_id"], 3001);
    assert_eq!(map_json["teleportable"], true);
    assert_eq!(map_json["save_point"]["can_use_tent"], true);
    assert_eq!(map_json["treasure_boxes"].as_array().map(Vec::len), Some(2));
    assert_eq!(map_json["treasure_boxes"][1]["is_trap"], true);
    assert_eq!(map_json["treasure_boxes"][1]["battle_group_id"], 99);

    let (town_text, town_json) = call_data_ok(
        &module,
        "read_map",
        json!({ "level_name": "Town_01Fake_01_PL:PersistentLevel" }),
    )
    .await;
    assert!(town_text.contains("微风港镇"));
    assert!(town_text.contains("商店设施"));
    assert_eq!(town_json["map_id"], 2001);
    assert!(
        town_json["shop_facilities"]
            .as_array()
            .is_some_and(|list| !list.is_empty())
    );

    // read_item by ID and by name, plus untranslated item staying honest.
    let (blade_text, blade_json) =
        call_data_ok(&module, "read_item", json!({ "item_id": 10100 })).await;
    assert!(blade_text.contains("玄铁大剑"));
    assert!(blade_text.contains("物攻+32"));
    assert!(blade_text.contains("#2001"));
    assert_eq!(blade_json["item_id"], 10100);
    assert_eq!(blade_json["sold_in_shop_ids"], json!([2001]));

    let (_, herb_json) = call_data_ok(&module, "read_item", json!({ "name": "清露草药" })).await;
    assert_eq!(herb_json["item_id"], 1);

    let (seed_text, seed_json) =
        call_data_ok(&module, "read_item", json!({ "item_id": 9500 })).await;
    assert!(seed_text.contains("无中文译名"));
    assert!(!seed_text.contains("未翻訳"));
    assert_eq!(seed_json["name"], Value::Null);

    // read_quest by ID, by name, and by chapter.
    let (q_text, q_json) = call_data_ok(&module, "read_quest", json!({ "quest_id": 10 })).await;
    assert!(q_text.contains("点亮港湾灯火"));
    assert!(q_text.contains("灯塔守卫"));
    assert_eq!(q_json["quest_id"], 10);
    assert_eq!(q_json["chapter_no"], 1);
    assert_eq!(q_json["objectives"].as_array().map(Vec::len), Some(2));

    let (_, q_by_name) =
        call_data_ok(&module, "read_quest", json!({ "name": "点亮港湾灯火" })).await;
    assert_eq!(q_by_name["quest_id"], 10);

    let (q_ch_text, q_ch_json) = call_data_ok(&module, "read_quest", json!({ "chapter": 1 })).await;
    assert!(q_ch_text.contains("点亮港湾灯火"));
    assert_eq!(q_ch_json["quests"].as_array().map(Vec::len), Some(1));

    let (q_empty_text, q_empty_json) =
        call_data_ok(&module, "read_quest", json!({ "chapter": 8 })).await;
    assert!(q_empty_text.contains("没有"));
    assert_eq!(q_empty_json["quests"], json!([]));

    // read_shop by ID, by name, and by item_id.
    let (shop_text, shop_json) = call_data_ok(&module, "read_shop", json!({ "shop_id": 1 })).await;
    assert!(shop_text.contains("港镇道具店"));
    assert!(shop_text.contains("清露草药"));
    assert_eq!(shop_json["shop_id"], 1);
    assert_eq!(shop_json["items"].as_array().map(Vec::len), Some(2));

    let (_, shop_by_name) =
        call_data_ok(&module, "read_shop", json!({ "name": "港镇道具店" })).await;
    assert_eq!(shop_by_name["shop_id"], 1);

    let (by_item_text, by_item_json) =
        call_data_ok(&module, "read_shop", json!({ "item_id": 500 })).await;
    assert!(by_item_text.contains("旅人帐篷"));
    assert!(by_item_text.contains("港镇道具店"));
    assert_eq!(by_item_json["shops"][0]["shop_id"], 1);
}

#[tokio::test]
async fn merged_fixture_covers_cross_kind_search_truncation_candidates_and_pagination() {
    let bridge = TestDir::new();
    let export = TestDir::new();
    build_merged_fixture_export(export.path());
    let module = module_with_data(bridge.path(), export.path());

    // Cross-kind search_data across map, item, quest, shop, enemy, skill, chapter.
    let (search_text, search_json) =
        call_data_ok(&module, "search_data", json!({ "query": "微光" })).await;
    assert!(search_text.contains("微光洞窟"));
    assert!(search_text.contains("微光幻兵"));
    assert_eq!(search_json["truncated"], false);
    let kinds: Vec<&str> = search_json["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|r| r["kind"].as_str().expect("kind"))
        .collect();
    assert!(kinds.contains(&"map"));
    assert!(kinds.contains(&"item"));

    // Filtered kinds.
    let (_, filtered_json) = call_data_ok(
        &module,
        "search_data",
        json!({ "query": "试作", "kinds": ["enemy", "skill"] }),
    )
    .await;
    let filtered_results = filtered_json["results"].as_array().expect("results");
    assert_eq!(filtered_results.len(), 4);
    assert!(
        filtered_results
            .iter()
            .all(|r| matches!(r["kind"].as_str(), Some("enemy" | "skill")))
    );

    // Resource listing and small-page UTF-8-safe pagination for all 5 catalogs.
    let descriptors = module.list_resources().await.expect("list_resources");
    assert_eq!(descriptors.len(), 5);
    for desc in &descriptors {
        assert!(desc.size_hint.is_some_and(|n| n > 0), "{desc:?}");
        let parsed = read_all_pages(&module, &desc.uri, 37).await;
        match desc.uri.as_str() {
            "ffr://chapters" => assert_eq!(parsed["chapters"].as_array().map(Vec::len), Some(1)),
            "ffr://maps" => assert_eq!(parsed["maps"].as_array().map(Vec::len), Some(5)),
            "ffr://enemies" => assert_eq!(parsed["enemies"].as_array().map(Vec::len), Some(3)),
            "ffr://skills" => assert_eq!(parsed["skills"].as_array().map(Vec::len), Some(4)),
            "ffr://db/schema" => assert!(
                parsed["table_count"].as_u64().is_some_and(|n| n >= 70),
                "{parsed}"
            ),
            other => panic!("unexpected resource uri: {other}"),
        }
    }

    // Chapter-filtered enemies URI (`ffr://enemies?chapter=N`) paginates and filters cleanly.
    let ch1_enemies = read_all_pages(&module, "ffr://enemies?chapter=1", 48).await;
    assert_eq!(ch1_enemies["chapter"], 1);
    let ch1_ids: Vec<i64> = ch1_enemies["enemies"]
        .as_array()
        .expect("chapter 1 enemies array")
        .iter()
        .map(|e| e["unit_id"].as_i64().expect("unit_id"))
        .collect();
    assert_eq!(ch1_ids, vec![9101, 9201]);
    assert_eq!(ch1_enemies["enemies"][0]["chapters"], json!([1]));

    let ch8_enemies = read_all_pages(&module, "ffr://enemies?chapter=8", 37).await;
    assert_eq!(ch8_enemies["chapter"], 8);
    assert_eq!(ch8_enemies["enemies"], json!([]));

    let bad_ch_zero = module
        .read_resource(
            "ffr://enemies?chapter=0",
            ResourceRange {
                offset: 0,
                length: 64,
            },
        )
        .await
        .expect_err("chapter=0 fails");
    assert!(bad_ch_zero.to_string().contains("大于等于 1"));

    let bad_ch_query = module
        .read_resource(
            "ffr://enemies?foo=1",
            ResourceRange {
                offset: 0,
                length: 64,
            },
        )
        .await
        .expect_err("unknown query param fails");
    assert!(bad_ch_query.to_string().contains("不支持的敌人筛选查询"));

    // `read_chapter` exposes `main_quest_id` & `main_quest_title`, and `read_enemy` exposes `chapters`.
    let (_, ch1_tool) = call_data_ok(&module, "read_chapter", json!({ "chapter": 1 })).await;
    assert_eq!(ch1_tool["main_quest_id"], 1001);
    assert_eq!(ch1_tool["main_quest_title"], "第1章启程之风");

    let (enemy_text, enemy_json) =
        call_data_ok(&module, "read_enemy", json!({ "unit_id": 9101 })).await;
    assert!(enemy_text.contains("第 1 章"), "{enemy_text}");
    assert_eq!(enemy_json["chapters"], json!([1]));

    let (_, unmapped_enemy) = call_data_ok(&module, "read_enemy", json!({ "unit_id": 9202 })).await;
    assert_eq!(unmapped_enemy["chapters"], json!([]));

    // Pagination boundary validation: zero length, mid-character offset, too-small length, past EOF.
    let zero_err = module
        .read_resource(
            "ffr://chapters",
            ResourceRange {
                offset: 0,
                length: 0,
            },
        )
        .await
        .expect_err("zero length fails");
    assert!(zero_err.to_string().contains("length 必须大于 0"));

    // Find a multi-byte Chinese character inside ffr://chapters to test UTF-8 boundary checks.
    let full_page = module
        .read_resource(
            "ffr://chapters",
            ResourceRange {
                offset: 0,
                length: 64 * 1024,
            },
        )
        .await
        .expect("read full chapters");
    let Content::Text { text: full_body } = full_page.content else {
        panic!("resource page is text");
    };
    let zh_byte_idx = full_body
        .char_indices()
        .find_map(|(idx, ch)| (ch.len_utf8() > 1).then_some(idx))
        .expect("chapters JSON contains Chinese characters");
    let mid_err = module
        .read_resource(
            "ffr://chapters",
            ResourceRange {
                offset: (zh_byte_idx + 1) as u64,
                length: 16,
            },
        )
        .await
        .expect_err("mid-character offset fails");
    assert!(mid_err.to_string().contains("多字节 UTF-8"));

    let tiny_err = module
        .read_resource(
            "ffr://chapters",
            ResourceRange {
                offset: zh_byte_idx as u64,
                length: 1,
            },
        )
        .await
        .expect_err("1 byte cannot fit a 3-byte Chinese character");
    assert!(tiny_err.to_string().contains("不足以容纳"));

    let eof_page = module
        .read_resource(
            "ffr://chapters",
            ResourceRange {
                offset: (full_body.len() + 10) as u64,
                length: 32,
            },
        )
        .await
        .expect("past EOF returns empty page");
    assert_eq!(eof_page.content, Content::text(""));
    assert_eq!(eof_page.next_offset, None);
}

#[tokio::test]
async fn search_data_caps_results_at_20_and_duplicate_names_list_candidates() {
    let bridge = TestDir::new();
    let export = TestDir::new();
    build_merged_fixture_export(export.path());
    inject_duplicate_and_bulk_items(export.path());

    let module = module_with_data(bridge.path(), export.path());

    // 25 synthetic items containing "测试药剂" -> search_data caps at 20.
    let (text, json) = call_data_ok(
        &module,
        "search_data",
        json!({ "query": "测试药剂", "kinds": ["item"] }),
    )
    .await;
    assert!(text.contains("共命中 25 条"));
    assert!(text.contains("仅展示前 20 条"));
    assert_eq!(json["total_matches"], 25);
    assert_eq!(json["truncated"], true);
    assert_eq!(json["results"].as_array().map(Vec::len), Some(20));

    // Duplicate item name "重名秘药" lists both candidates without choosing.
    let (dup_text, dup_json) =
        call_data_ok(&module, "read_item", json!({ "name": "重名秘药" })).await;
    assert!(dup_text.contains("50001"));
    assert!(dup_text.contains("50002"));
    assert_eq!(dup_json["ambiguous"], true);
    assert_eq!(dup_json["candidates"].as_array().map(Vec::len), Some(2));
}

#[tokio::test]
async fn parameter_validation_and_missing_data_dir_return_chinese_tool_errors() {
    let bridge = TestDir::new();
    let export = TestDir::new();
    build_merged_fixture_export(export.path());
    let module_ok = module_with_data(bridge.path(), export.path());

    // Parameter errors.
    let cases = [
        ("read_chapter", json!({ "chapter": 0 }), "大于等于 1"),
        ("read_chapter", json!({ "extra": 1 }), "未知参数"),
        ("read_map", json!({}), "必须提供"),
        (
            "read_map",
            json!({ "map_id": 1, "level_name": "X" }),
            "不能同时传入",
        ),
        ("read_enemy", json!({ "name": "   " }), "不能为空"),
        ("read_enemy", json!({ "unit_id": "abc" }), "必须是整数"),
        ("read_battle_group", json!({}), "缺少必需"),
        (
            "read_skill",
            json!({ "skill_id": 1, "name": "X" }),
            "不能同时传入",
        ),
        ("read_item", json!([]), "必须是 JSON 对象"),
        (
            "read_quest",
            json!({ "quest_id": 1, "chapter": 1 }),
            "恰好提供一个",
        ),
        ("read_shop", json!({}), "恰好提供一个"),
        ("search_data", json!({ "query": "" }), "不能为空"),
        (
            "search_data",
            json!({ "query": "冰", "kinds": [] }),
            "不能为空数组",
        ),
        (
            "search_data",
            json!({ "query": "冰", "kinds": ["bogus"] }),
            "不支持类别",
        ),
    ];

    for (tool, args, expected) in cases {
        let err = call_data_err(&module_ok, tool, args).await;
        assert!(
            err.contains(expected),
            "tool `{tool}` expected `{expected}` in `{err}`"
        );
    }

    // Missing data dir: all 9 data tools fail cleanly, while status tools remain unaffected.
    let fake_mod = FakeMod::start(bridge.path()).await;
    let missing_dir = bridge.path().join("nonexistent-export");
    let module_missing = module(bridge.path()).with_export_dir(&missing_dir);

    let (status_out, status_val) = call_json(&module_missing, "game_status").await;
    assert!(!status_out.is_error);
    assert_eq!(status_val["bridge"]["ok"], true);

    let (ping_out, ping_val) = call_json(&module_missing, "bridge_ping").await;
    assert!(!ping_out.is_error);
    assert_eq!(ping_val["ok"], true);

    let data_calls = [
        ("read_chapter", json!({})),
        ("read_map", json!({ "map_id": 2001 })),
        ("read_enemy", json!({ "unit_id": 9101 })),
        ("read_battle_group", json!({ "group_id": 9801 })),
        ("read_skill", json!({ "skill_id": 95001 })),
        ("read_item", json!({ "item_id": 1 })),
        ("read_quest", json!({ "quest_id": 10 })),
        ("read_shop", json!({ "shop_id": 1 })),
        ("search_data", json!({ "query": "微光" })),
    ];
    for (tool, args) in data_calls {
        let err = call_data_err(&module_missing, tool, args).await;
        assert!(
            err.contains("不存在"),
            "missing dir error for `{tool}`: {err}"
        );
    }

    fake_mod.stop().await;
}

async fn read_all_pages(module: &impl GameModule, uri: &str, page_len: u64) -> Value {
    let mut assembled = String::new();
    let mut offset = 0_u64;
    let mut pages = 0_usize;
    loop {
        let page = module
            .read_resource(
                uri,
                ResourceRange {
                    offset,
                    length: page_len,
                },
            )
            .await
            .expect("read resource page");
        assert_eq!(page.uri, uri);
        assert_eq!(page.offset, offset);
        let Content::Text { text } = page.content else {
            panic!("resource page content must be Text");
        };
        assert!(text.len() as u64 <= page_len);
        assembled.push_str(&text);
        pages += 1;
        match page.next_offset {
            Some(next) => {
                assert!(next > offset, "pagination must make forward progress");
                offset = next;
            }
            None => {
                assert_eq!(page.total_size, Some(assembled.len() as u64));
                break;
            }
        }
    }
    assert!(pages >= 2, "expected multiple pages for {uri}");
    serde_json::from_str(&assembled).expect("reassembled compact JSON is valid")
}

fn inject_duplicate_and_bulk_items(export_root: &Path) {
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
        .entry("BulkTest")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("BulkTest namespace");

    let mut add_item = |id: i64, key: &str, zh_name: &str| {
        ns.insert(key.to_owned(), json!(zh_name));
        rows.insert(
            format!("Bulk_{id}"),
            json!({
                "ID": id,
                "SortId": id,
                "ItemType": "eItemType::Consumables",
                "Name": { "Namespace": "BulkTest", "Key": key },
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
