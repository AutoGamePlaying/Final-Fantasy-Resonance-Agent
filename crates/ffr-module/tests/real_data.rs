//! Smoke tests against real unpacked FFR data (`$FFR_DEPS/ffr/export` or `.deps/ffr/export`).
//!
//! Skips cleanly when unpacked data or `zh-Hans.json` is unavailable.

mod common;

use std::path::PathBuf;

use agp_core::RunId;
use agp_game::{CallContext, Content, GameModule, ResourceRange};
use common::{
    TestDir, build_fixture_db_from_export, call_data_ok, module_with_data, module_with_db,
    read_all_resource_text, repo_root,
};
use ffr_data::{FFR_DEPS_ENV, datatable_dir, export_dir_from_deps, localization_file};
use ffr_db::{GameDb, default_db_path_from_deps};
use serde_json::json;

fn real_export_dir() -> PathBuf {
    if let Some(deps) = std::env::var_os(FFR_DEPS_ENV)
        && !deps.is_empty()
    {
        return export_dir_from_deps(PathBuf::from(deps));
    }
    export_dir_from_deps(repo_root().join(".deps"))
}

fn real_db_candidate() -> PathBuf {
    if let Some(deps) = std::env::var_os(FFR_DEPS_ENV)
        && !deps.is_empty()
    {
        return default_db_path_from_deps(PathBuf::from(deps));
    }
    default_db_path_from_deps(repo_root().join(".deps"))
}

fn has_populated_core_tables(db_path: &std::path::Path) -> bool {
    let Ok(db) = GameDb::open(db_path) else {
        return false;
    };
    db.has_battle_data().unwrap_or(false)
        && db.has_world_data().unwrap_or(false)
        && db.has_scenario_data().unwrap_or(false)
}

#[tokio::test]
async fn real_data_steel_bat_weak_to_ice_and_spot_checks() {
    let export_root = real_export_dir();
    let unit_file = datatable_dir(&export_root).join("Unit/DT_UnitParameter.json");
    let map_file = datatable_dir(&export_root).join("Map/CDT_MapData_Prod.json");
    let has_data = unit_file.is_file()
        && map_file.is_file()
        && localization_file(&export_root, "zh-Hans").is_file();
    if !has_data {
        eprintln!(
            "跳过真实导出数据测试：未找到完整导出目录 {}",
            export_root.display()
        );
        return;
    }

    let bridge = TestDir::new();
    let module = module_with_data(bridge.path(), &export_root);

    // Spot-check 钢蝠: weak to Ice.
    let (bat_text, bat_json) = call_data_ok(&module, "read_enemy", json!({ "name": "钢蝠" })).await;
    assert!(bat_text.contains("钢蝠"), "{bat_text}");
    assert!(bat_text.contains("冰"), "{bat_text}");
    assert_eq!(bat_json["unit_id"], 2);
    assert_eq!(bat_json["is_boss"], false);
    assert!(
        bat_json["weaknesses"]
            .as_array()
            .expect("weaknesses array")
            .iter()
            .any(|v| v == "冰"),
        "钢蝠弱点应含冰：{bat_json}"
    );
    assert!(
        bat_json["weakness_elements"]
            .as_array()
            .expect("weakness_elements array")
            .iter()
            .any(|v| v == "Ice"),
        "钢蝠弱点属性代码应含 Ice：{bat_json}"
    );
    assert_eq!(
        bat_json["chapters"],
        json!([1]),
        "钢蝠应归属第 1 章：{bat_json}"
    );

    // Spot-check search_data, read_chapter, read_map, and resources/read.
    let (_, search_json) = call_data_ok(&module, "search_data", json!({ "query": "钢蝠" })).await;
    assert!(
        search_json["results"]
            .as_array()
            .expect("results")
            .iter()
            .any(|hit| hit["kind"] == "enemy" && hit["id"] == 2)
    );

    let (_, ch1_json) = call_data_ok(&module, "read_chapter", json!({ "chapter": 1 })).await;
    assert_eq!(ch1_json["chapter_no"], 1);
    assert_eq!(ch1_json["main_quest_id"], 1001);
    assert!(
        ch1_json["main_scenario"]
            .as_array()
            .is_some_and(|list| list.len() == 19)
    );

    let (_, map_json) = call_data_ok(
        &module,
        "read_map",
        json!({ "level_name": "Dng_01Gra_43_PL" }),
    )
    .await;
    assert_eq!(map_json["map_id"], 3001);

    let resources = module.list_resources().await.expect("list_resources");
    assert_eq!(resources.len(), 5);
    assert!(resources.iter().all(|r| r.size_hint.is_some_and(|n| n > 0)));

    let first_page = module
        .read_resource(
            "ffr://enemies",
            ResourceRange {
                offset: 0,
                length: 1024,
            },
        )
        .await
        .expect("read first page of ffr://enemies");
    let Content::Text { text } = first_page.content else {
        panic!("expected text content");
    };
    assert!(text.starts_with(r#"{"enemies":["#));
    assert!(first_page.next_offset.is_some());

    // Spot-check `ffr://enemies?chapter=1` includes 钢蝠 (unit_id = 2) and chapter=2 does not.
    let ch1_page = module
        .read_resource(
            "ffr://enemies?chapter=1",
            ResourceRange {
                offset: 0,
                length: 1024 * 1024,
            },
        )
        .await
        .expect("read ffr://enemies?chapter=1");
    let Content::Text { text: ch1_body } = ch1_page.content else {
        panic!("expected text content");
    };
    let ch1_parsed: serde_json::Value =
        serde_json::from_str(&ch1_body).expect("valid chapter 1 enemies JSON");
    assert_eq!(ch1_parsed["chapter"], 1);
    assert!(
        ch1_parsed["enemies"]
            .as_array()
            .expect("ch1 enemies array")
            .iter()
            .any(|e| e["unit_id"] == 2 && e["name"] == "钢蝠"),
        "第 1 章敌人列表应包含钢蝠"
    );

    let ch2_page = module
        .read_resource(
            "ffr://enemies?chapter=2",
            ResourceRange {
                offset: 0,
                length: 1024 * 1024,
            },
        )
        .await
        .expect("read ffr://enemies?chapter=2");
    let Content::Text { text: ch2_body } = ch2_page.content else {
        panic!("expected text content");
    };
    let ch2_parsed: serde_json::Value =
        serde_json::from_str(&ch2_body).expect("valid chapter 2 enemies JSON");
    assert!(
        !ch2_parsed["enemies"]
            .as_array()
            .expect("ch2 enemies array")
            .iter()
            .any(|e| e["unit_id"] == 2),
        "第 2 章敌人列表不应包含钢蝠"
    );
}

#[tokio::test]
async fn real_data_db_and_json_outputs_align_verbatim() {
    let export_root = real_export_dir();
    let unit_file = datatable_dir(&export_root).join("Unit/DT_UnitParameter.json");
    let map_file = datatable_dir(&export_root).join("Map/CDT_MapData_Prod.json");
    let has_data = unit_file.is_file()
        && map_file.is_file()
        && localization_file(&export_root, "zh-Hans").is_file();
    if !has_data {
        eprintln!(
            "跳过真实数据 DB 对齐测试：未找到完整导出目录 {}",
            export_root.display()
        );
        return;
    }

    let temp = TestDir::new();
    let bridge_dir = temp.path().join("bridge");
    let candidate_db = real_db_candidate();
    let db_path = if has_populated_core_tables(&candidate_db) {
        candidate_db
    } else {
        let built = temp.path().join("real-core.sqlite");
        build_fixture_db_from_export(&export_root, &built);
        built
    };

    let db_has_levels = GameDb::open(&db_path)
        .ok()
        .and_then(|db| db.map_exits(3001).ok())
        .is_some_and(|exits| !exits.is_empty());

    let db_mod = module_with_db(&bridge_dir, &db_path);
    let json_mod = module_with_data(&bridge_dir, &export_root);

    let mut cases = vec![
        ("read_enemy", json!({ "name": "钢蝠" })),
        ("read_enemy", json!({ "unit_id": 2 })),
        ("read_skill", json!({ "name": "火炎" })),
        ("read_item", json!({ "name": "恢复药" })),
        ("read_item", json!({ "name": "碎星锤" })),
        ("read_chapter", json!({})),
        ("read_chapter", json!({ "chapter": 1 })),
        ("read_shop", json!({ "shop_id": 2 })),
        ("read_shop", json!({ "item_id": 1 })),
        ("read_battle_group", json!({ "group_id": 1 })),
        ("read_quest", json!({ "chapter": 1 })),
        ("search_data", json!({ "query": "钢蝠" })),
        ("search_data", json!({ "query": "火炎" })),
        ("search_data", json!({ "query": "恢复药" })),
    ];
    if !db_has_levels {
        cases.push(("read_map", json!({ "map_id": 3001 })));
        cases.push(("read_map", json!({ "level_name": "Dng_01Gra_43_PL" })));
    }

    for (tool, args) in cases {
        let db_out = db_mod
            .call_tool(tool, args.clone(), &CallContext::new(RunId::new()))
            .await
            .expect("db real call");
        let json_out = json_mod
            .call_tool(tool, args.clone(), &CallContext::new(RunId::new()))
            .await
            .expect("json real call");
        assert_eq!(
            (db_out.is_error, &db_out.content),
            (json_out.is_error, &json_out.content),
            "真实数据 DB 与 JSON 输出不一致：{tool} {args}"
        );
    }

    if db_has_levels {
        // Full DB enriches `read_map` with level data (exits, NPCs, shops, save points, encounter zones)
        // that legacy export-only JSON cannot provide; verify core fields align and enriched fields populate.
        for args in [
            json!({ "map_id": 3001 }),
            json!({ "level_name": "Dng_01Gra_43_PL" }),
        ] {
            let (db_text, db_val) = call_data_ok(&db_mod, "read_map", args.clone()).await;
            let (_, json_val) = call_data_ok(&json_mod, "read_map", args).await;
            for key in [
                "map_id",
                "name",
                "level_name",
                "map_type",
                "island",
                "danger_level",
                "teleportable",
                "save_point",
                "shop_facilities",
                "treasure_count",
            ] {
                assert_eq!(db_val[key], json_val[key], "read_map core field `{key}`");
            }
            assert!(
                db_val["exits"].as_array().is_some_and(|a| !a.is_empty()),
                "full DB read_map(3001) should have exits: {db_text}"
            );
            assert!(
                db_val["npcs"].as_array().is_some_and(|a| !a.is_empty()),
                "full DB read_map(3001) should have npcs: {db_text}"
            );
            assert!(
                db_val["shops"].as_array().is_some_and(|a| !a.is_empty()),
                "full DB read_map(3001) should have shops: {db_text}"
            );
            assert!(
                db_val["save_points"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty()),
                "full DB read_map(3001) should have save_points: {db_text}"
            );
        }
        // Map 3000 has dungeon encounter zones (map 3001 is the safe entrance room).
        let (dng_text, dng_val) =
            call_data_ok(&db_mod, "read_map", json!({ "map_id": 3000 })).await;
        assert!(
            dng_val["encounter_zones"]
                .as_array()
                .is_some_and(|a| !a.is_empty()),
            "full DB read_map(3000) should have encounter_zones: {dng_text}"
        );
    }

    // Also verify one concrete skill_id from `火炎` candidates aligns verbatim.
    let (_, fire_json) = call_data_ok(&db_mod, "read_skill", json!({ "name": "火炎" })).await;
    if let Some(first_id) = fire_json["candidates"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|c| c["skill_id"].as_i64())
    {
        let db_out = db_mod
            .call_tool(
                "read_skill",
                json!({ "skill_id": first_id }),
                &CallContext::new(RunId::new()),
            )
            .await
            .expect("db skill by id");
        let json_out = json_mod
            .call_tool(
                "read_skill",
                json!({ "skill_id": first_id }),
                &CallContext::new(RunId::new()),
            )
            .await
            .expect("json skill by id");
        assert_eq!(
            (db_out.is_error, &db_out.content),
            (json_out.is_error, &json_out.content),
            "真实数据 read_skill(skill_id={first_id}) 不一致"
        );
    }

    // Verify all catalog resources and descriptors match byte-for-byte on real data.
    let db_res = db_mod.list_resources().await.expect("db list_resources");
    let json_res = json_mod
        .list_resources()
        .await
        .expect("json list_resources");
    assert_eq!(db_res, json_res, "真实数据 list_resources 不一致");

    for uri in [
        "ffr://chapters",
        "ffr://maps",
        "ffr://enemies",
        "ffr://enemies?chapter=1",
        "ffr://skills",
        "ffr://db/schema",
    ] {
        let db_body = read_all_resource_text(&db_mod, uri).await;
        let json_body = read_all_resource_text(&json_mod, uri).await;
        assert_eq!(db_body, json_body, "真实数据资源 `{uri}` 不一致");
    }
}
