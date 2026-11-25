//! Smoke tests against real unpacked FFR data (`$FFR_DEPS/ffr/export` or `.deps/ffr/export`).
//!
//! Skips cleanly when unpacked data or `zh-Hans.json` is unavailable.

mod common;

use std::path::PathBuf;

use agp_game::{Content, GameModule, ResourceRange};
use common::{TestDir, call_data_ok, module_with_data, repo_root};
use ffr_data::{FFR_DEPS_ENV, datatable_dir, export_dir_from_deps, localization_file};
use serde_json::json;

fn real_export_dir() -> PathBuf {
    if let Some(deps) = std::env::var_os(FFR_DEPS_ENV)
        && !deps.is_empty()
    {
        return export_dir_from_deps(PathBuf::from(deps));
    }
    export_dir_from_deps(repo_root().join(".deps"))
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
    assert_eq!(resources.len(), 4);
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
