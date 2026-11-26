//! Smoke tests against real unpacked FFR data (`$FFR_DEPS/ffr/export` or `.deps/ffr/export`).
//!
//! Skips cleanly when unpacked data or `zh-Hans.json` is unavailable.

mod common;

use std::path::PathBuf;

use agp_core::RunId;
use agp_game::{CallContext, Content, GameModule, ResourceRange};
use common::{
    TestDir, build_fixture_db_from_export, build_merged_fixture_export, call_data_err,
    call_data_ok, module_with_data, module_with_db, open_real_db_or_skip, read_all_resource_text,
    repo_root,
};
use ffr_data::{FFR_DEPS_ENV, datatable_dir, export_dir_from_deps, localization_file};
use ffr_db::default_db_path_from_deps;
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
    // If `.deps/ffr/db/ffr.sqlite` exists, validate it strictly (never skip or rebuild on old
    // schema version); only skip if the DB file does not exist at all.
    let Some(db) = open_real_db_or_skip(&candidate_db, "真实数据 DB 对齐测试") else {
        return;
    };
    assert!(
        db.has_battle_data().unwrap_or(false)
            && db.has_world_data().unwrap_or(false)
            && db.has_scenario_data().unwrap_or(false),
        "真实数据库 {} 缺少核心表（battle/world/scenario）；请运行 `ffr-db build` 重建完整数据库",
        candidate_db.display()
    );
    let db_has_levels = db.map_exits(3001).is_ok_and(|exits| !exits.is_empty());
    drop(db);
    let db_path = candidate_db;

    let db_mod = module_with_db(&bridge_dir, &db_path);
    let json_mod = module_with_data(&bridge_dir, &export_root);

    let mut cases = vec![
        ("read_enemy", json!({ "name": "钢蝠" })),
        ("read_enemy", json!({ "unit_id": 2 })),
        ("read_skill", json!({ "name": "火炎" })),
        ("read_skill", json!({ "skill_id": 400220 })),
        ("read_skill", json!({ "skill_id": 100220 })),
        ("read_item", json!({ "name": "恢复药" })),
        ("read_item", json!({ "name": "碎星锤" })),
        ("read_item", json!({ "item_id": 10699 })),
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

#[tokio::test]
async fn schema_v1_database_fails_with_clear_chinese_rebuild_instructions() {
    let temp = TestDir::new();
    let export_dir = temp.path().join("export");
    let db_path = temp.path().join("old-v1.sqlite");
    build_merged_fixture_export(&export_dir);
    build_fixture_db_from_export(&export_dir, &db_path);

    // Downgrade `meta.schema_version` to `'1'` to simulate an un-rebuilt v1 database.
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open temp db rw");
        conn.execute(
            "UPDATE meta SET value = '1' WHERE key = 'schema_version'",
            [],
        )
        .expect("set schema_version = 1");
    }

    // 1. Real-data test helper must panic with clear Chinese rebuild guidance (never auto-skip).
    let panic_res = std::panic::catch_unwind(|| open_real_db_or_skip(&db_path, "真实数据测试"));
    let panic_payload = panic_res.expect_err("schema v1 database must panic instead of skipping");
    let msg = panic_payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic_payload.downcast_ref::<&str>().copied())
        .expect("panic message is string");
    assert!(msg.contains("schema 版本为 v1"), "{msg}");
    assert!(
        msg.contains(&format!("要求 v{}", ffr_db::SCHEMA_VERSION)),
        "{msg}"
    );
    assert!(msg.contains("ffr-db build"), "{msg}");
    assert!(msg.contains("不会自动跳过"), "{msg}");
    assert!(msg.contains("不会自动重建"), "{msg}");

    // 2. Module data tools also return a clear Chinese version-mismatch error.
    let bridge_dir = temp.path().join("bridge");
    let db_mod = module_with_db(&bridge_dir, &db_path);
    let err = call_data_err(&db_mod, "read_item", json!({ "item_id": 1 })).await;
    assert!(err.contains("版本不兼容"), "{err}");
    assert!(err.contains("v1"), "{err}");
    assert!(err.contains("ffr-db build"), "{err}");
}
