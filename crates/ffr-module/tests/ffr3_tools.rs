//! End-to-end tests for `ffr-3` static game-data tools (`find_route`, `where_to_get`, `read_npc`,
//! `read_talk`, `whats_near`, `query_game_db`), enriched `read_map` & `read_enemy`,
//! `ffr://db/schema`, and real-DB spot checks.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{
    TestDir, call_data_err, call_data_ok, copy_dir_recursive, module_with_data, module_with_db,
    open_real_db_or_skip, read_all_resource_text, repo_root, write_json_atomic,
};
use ffr_data::FFR_DEPS_ENV;
use ffr_db::{BuildInputs, BuildOptions, Stage, default_db_path_from_deps};
use serde_json::{Value, json};

/// Build a synthetic multi-stage `ffr-db` SQLite database based on `test/fixtures/db/links`,
/// enriched with multi-hop map routes (conditional + event-only + savepoint teleport), duplicate
/// map/item/NPC names, chests/drops/steals/quest rewards, and spatial objects around `(0, 0, 0)`.
fn build_ffr3_fixture_db(work_dir: &Path) -> (PathBuf, PathBuf) {
    let links_src = repo_root().join("test/fixtures/db/links");
    let fixture_root = work_dir.join("fixture");
    copy_dir_recursive(&links_src, &fixture_root);

    let export_dir = fixture_root.join("export");
    let export_full_dir = fixture_root.join("export-full");
    let maps_dir = fixture_root.join("maps");

    // 1. Add zh-Hans localization entries for synthetic maps, items, quests, and enemy.
    {
        let loc_path = export_dir.join("Localization/zh-Hans.json");
        let mut loc: Value =
            serde_json::from_str(&fs::read_to_string(&loc_path).expect("read zh-Hans"))
                .expect("parse zh-Hans");
        let world_ns = loc["ST_MockWorld"]
            .as_object_mut()
            .expect("ST_MockWorld object");
        world_ns.insert("MAP_3002".to_owned(), json!("试作走廊"));
        world_ns.insert("MAP_3003".to_owned(), json!("试作城镇"));
        world_ns.insert("MAP_3004".to_owned(), json!("重名洞窟"));
        world_ns.insert("MAP_3005".to_owned(), json!("重名洞窟"));
        world_ns.insert("MAP_3099".to_owned(), json!("孤岛遗迹"));
        world_ns.insert("ITEM_901".to_owned(), json!("重名秘药"));
        world_ns.insert("ITEM_902".to_owned(), json!("重名秘药"));
        world_ns.insert("ITEM_903".to_owned(), json!("孤立圣物"));
        world_ns.insert("UNIT_462".to_owned(), json!("试作魔兽"));
        world_ns.insert("QUEST_10".to_owned(), json!("试作支线任务"));
        world_ns.insert("QB_1001".to_owned(), json!("试作委托板任务"));
        fs::write(
            &loc_path,
            serde_json::to_vec_pretty(&loc).expect("encode zh-Hans"),
        )
        .expect("write zh-Hans");
    }

    // 2. Add extra maps (3002, 3003, duplicate 3004 & 3005, isolated 3099) to CDT_MapData_Prod.
    {
        let map_path = export_dir.join("FFRS/Content/Datatable/Map/CDT_MapData_Prod.json");
        let mut doc: Value =
            serde_json::from_str(&fs::read_to_string(&map_path).expect("read map dt"))
                .expect("parse map dt");
        let rows = doc[0]["Rows"].as_object_mut().expect("map rows");
        let make_map = |id: i64, key: &str, level_name: &str, map_type: &str| {
            json!({
                "ID": id,
                "SortId": id,
                "GroupId": 1,
                "regionId": -1,
                "sameMapId": -1,
                "island": "eMapIslandType::Grandshelt",
                "Name": {
                    "TableId": "/Game/Datatable/Strings/ST_MockWorld.ST_MockWorld",
                    "Key": key,
                    "SourceString": key,
                    "LocalizedString": key
                },
                "DangerLevel": 2,
                "Level": {
                    "AssetPathName": format!("/Game/Map/Mock/{level_name}.{level_name}"),
                    "SubPathString": ""
                },
                "MapType": map_type,
                "dungeonType": "eMapDungeonType::Cave",
                "battleStage": -1,
                "doMovementEncount": false,
                "doEncountCondition": "",
                "encountGroupId": -1,
                "facilityDataList": [],
                "startupEventList": []
            })
        };
        rows.insert(
            "MockCorridor".to_owned(),
            make_map(3002, "MAP_3002", "MockCorridor_PL", "eMapType::Dungeon"),
        );
        rows.insert(
            "MockTown".to_owned(),
            make_map(3003, "MAP_3003", "MockTown2_PL", "eMapType::Town"),
        );
        rows.insert(
            "DupCaveA".to_owned(),
            make_map(3004, "MAP_3004", "DupCaveA_PL", "eMapType::Dungeon"),
        );
        rows.insert(
            "DupCaveB".to_owned(),
            make_map(3005, "MAP_3005", "DupCaveB_PL", "eMapType::Field"),
        );
        rows.insert(
            "IsolatedMap".to_owned(),
            make_map(3099, "MAP_3099", "Isolated_PL", "eMapType::Town"),
        );
        fs::write(
            &map_path,
            serde_json::to_vec_pretty(&doc).expect("encode map dt"),
        )
        .expect("write map dt");
    }

    // 3. Add duplicate-name items (901, 902) and source-less item (903) to DT_ConsumablesItemData.
    {
        let item_path = export_dir.join("FFRS/Content/Datatable/Item/DT_ConsumablesItemData.json");
        let mut doc: Value =
            serde_json::from_str(&fs::read_to_string(&item_path).expect("read item dt"))
                .expect("parse item dt");
        let rows = doc[0]["Rows"].as_object_mut().expect("item rows");
        for (row_key, id, loc_key) in [
            ("DupElixirA", 901, "ITEM_901"),
            ("DupElixirB", 902, "ITEM_902"),
            ("OrphanRelic", 903, "ITEM_903"),
        ] {
            rows.insert(
                row_key.to_owned(),
                json!({
                    "usableScene": "eItemUsableScene::Any",
                    "isConsumedAfterUse": true,
                    "SkillId": -1,
                    "throwSkillData": { "isThrow": false, "SkillId": -1 },
                    "ID": id,
                    "SortId": id,
                    "Name": {
                        "TableId": "/Game/Datatable/Strings/ST_MockWorld.ST_MockWorld",
                        "Key": loc_key,
                        "SourceString": loc_key,
                        "LocalizedString": loc_key
                    },
                    "Description": { "CultureInvariantString": null },
                    "ItemType": "eItemType::Consumables",
                    "buyingPrice": 200,
                    "sellingPrice": 100,
                    "isAvailableSale": true
                }),
            );
        }
        fs::write(
            &item_path,
            serde_json::to_vec_pretty(&doc).expect("encode item dt"),
        )
        .expect("write item dt");
    }

    // 4. Give enemy 462 its own zh-Hans name + drops & steals of item 1 and item 2.
    {
        let unit_path = export_dir.join("FFRS/Content/Datatable/Unit/DT_UnitParameter.json");
        let mut doc: Value =
            serde_json::from_str(&fs::read_to_string(&unit_path).expect("read unit dt"))
                .expect("parse unit dt");
        let enemy = doc[0]["Rows"]["テスト敵"]
            .as_object_mut()
            .expect("enemy 462 row");
        enemy.insert(
            "Name".to_owned(),
            json!({
                "TableId": "/Game/Datatable/Strings/ST_MockWorld.ST_MockWorld",
                "Key": "UNIT_462",
                "SourceString": "テスト敵",
                "LocalizedString": "テスト敵"
            }),
        );
        enemy.insert("DropNormalItem".to_owned(), json!(1));
        enemy.insert("DropNormalItemRate".to_owned(), json!(30));
        enemy.insert("DropRareItem".to_owned(), json!(2));
        enemy.insert("DropRareItemRate".to_owned(), json!(5));
        enemy.insert("StealNormalItem".to_owned(), json!(1));
        enemy.insert("StealRareItem".to_owned(), json!(2));
        enemy.insert("StealItemRate".to_owned(), json!(50));
        fs::write(
            &unit_path,
            serde_json::to_vec_pretty(&doc).expect("encode unit dt"),
        )
        .expect("write unit dt");
    }

    // 5. Add treasure boxes in CDT_MapGimmickTreasureBox (normal box #0 with item 1, trap box #1
    //    with battle_group 9001 so enemy 462 gets chapter attribution if needed).
    {
        let tb_path = export_dir
            .join("FFRS/Content/Datatable/Map/Gimmick/TreasureBox/CDT_MapGimmickTreasureBox.json");
        let mut doc: Value =
            serde_json::from_str(&fs::read_to_string(&tb_path).expect("read tb dt"))
                .expect("parse tb dt");
        let rows = doc[0]["Rows"].as_object_mut().expect("tb rows");
        rows.insert(
            "MockShrine_Box_0".to_owned(),
            json!({
                "ID": 0,
                "mapId": 3001,
                "ItemId": 1,
                "ItemNum": 2,
                "Money": -1,
                "IsTrap": false,
                "battleGroupId": -1,
                "battleFinishEventId": "None"
            }),
        );
        rows.insert(
            "MockShrine_Box_1".to_owned(),
            json!({
                "ID": 1,
                "mapId": 3001,
                "ItemId": -1,
                "ItemNum": 0,
                "Money": 500,
                "IsTrap": true,
                "battleGroupId": 9001,
                "battleFinishEventId": "None"
            }),
        );
        fs::write(
            &tb_path,
            serde_json::to_vec_pretty(&doc).expect("encode tb dt"),
        )
        .expect("write tb dt");
    }

    // 6. Add side quest prize & quest board reward granting item 1.
    {
        let req_path = export_dir.join("FFRS/Content/Datatable/Request/DT_RequestData.json");
        let mut doc: Value =
            serde_json::from_str(&fs::read_to_string(&req_path).expect("read request dt"))
                .expect("parse request dt");
        let rows = doc[0]["Rows"].as_object_mut().expect("request rows");
        rows.insert(
            "MockReq_10".to_owned(),
            json!({
                "ID": 10,
                "Name": {
                    "TableId": "/Game/Datatable/Strings/ST_MockWorld.ST_MockWorld",
                    "Key": "QUEST_10",
                    "SourceString": "試作依頼",
                    "LocalizedString": "試作依頼"
                },
                "Requester": { "CultureInvariantString": null },
                "RequesterId": { "mapId": 3001, "Data": { "Type": "eMapObjectType::NPC", "ID": 3 } },
                "SubRequesterIds": [],
                "ReportDestinationId": { "mapId": 3001, "Data": { "Type": "eMapObjectType::NPC", "ID": 3 } },
                "RequesterMapUnitId": 1060,
                "Category": "ERequestCategory::Grandshelt",
                "AcceptCondition": "{progress:0}>=1010",
                "Occurrence": { "Condition": "{progress:0}>=1010", "PredictionFlagId": -1, "DestinationList": [] },
                "ProgressList": [],
                "AchievementCondition": "",
                "CompletionCondition": "",
                "EndingSummaryText": { "CultureInvariantString": null },
                "Prize": [
                    { "ID": 1, "Num": 3, "Text": { "CultureInvariantString": null } }
                ]
            }),
        );
        fs::write(
            &req_path,
            serde_json::to_vec_pretty(&doc).expect("encode request dt"),
        )
        .expect("write request dt");
    }
    {
        let qb_path = export_dir.join("FFRS/Content/Datatable/Quest/CDT_QuestBoardData.json");
        let mut doc: Value =
            serde_json::from_str(&fs::read_to_string(&qb_path).expect("read qb dt"))
                .expect("parse qb dt");
        let rows = doc[0]["Rows"].as_object_mut().expect("qb rows");
        rows.insert(
            "MockBoard_1001".to_owned(),
            json!({
                "QuestType": "EQuest_EntryType::Request",
                "QuestBoardID": 1000,
                "QuestId": 1001,
                "QuestName": {
                    "TableId": "/Game/Datatable/Strings/ST_MockWorld.ST_MockWorld",
                    "Key": "QB_1001",
                    "SourceString": "試作掲示板",
                    "LocalizedString": "試作掲示板"
                },
                "Description": { "CultureInvariantString": null },
                "MapName": { "CultureInvariantString": null },
                "Condition": "",
                "ClearConditionList": [{ "ID": -1, "DeliveryQuantity": -1 }],
                "RewardList": [
                    { "Money": -1, "ItemId": 1, "ItemNum": 5 }
                ],
                "RequestData": { "EventList": [] }
            }),
        );
        fs::write(
            &qb_path,
            serde_json::to_vec_pretty(&doc).expect("encode qb dt"),
        )
        .expect("write qb dt");
    }

    // 7. Add two duplicate-name map_units (2001, 2002) for `read_npc(name=...)` ambiguity test.
    {
        let mu_path = export_dir.join("FFRS/Content/Datatable/Asset/Map/DT_MapUnitAsset.json");
        let mut doc: Value =
            serde_json::from_str(&fs::read_to_string(&mu_path).expect("read map unit dt"))
                .expect("parse map unit dt");
        let rows = doc[0]["Rows"].as_object_mut().expect("map unit rows");
        for (row_key, id) in [("重名旅人A", 2001), ("重名旅人B", 2002)] {
            rows.insert(
                row_key.to_owned(),
                json!({
                    "ID": id,
                    "animationAssetList": [],
                    "Footstep": "eMapUnitFootstepType::Default",
                    "shadowScale": 1.0,
                    "specularScale": 0.5
                }),
            );
        }
        fs::write(
            &mu_path,
            serde_json::to_vec_pretty(&doc).expect("encode map unit dt"),
        )
        .expect("write map unit dt");
    }

    // 8. Enrich `MockTown_EV.json` (map 3001) with chest, save point, exit, encounter zone, and far actor,
    //    and create level files for maps 3002 & 3003 plus updated `_graph.json` so `find_route`
    //    traverses conditional cross-map exit (3001->3002) + savepoint teleport (3002->3003) +
    //    event-only exit (3002->3004).
    {
        let level_3001_path = maps_dir.join("MockTown_EV.json");
        let mut lvl: Value =
            serde_json::from_str(&fs::read_to_string(&level_3001_path).expect("read MockTown_EV"))
                .expect("parse MockTown_EV");
        let actors = lvl["actors"].as_array_mut().expect("actors array");
        actors.extend([
            json!({
                "name": "BP_SavePoint_3001",
                "class": "/Game/Blueprint/Map/BP_MapGimmick_SavePoint.BP_MapGimmick_SavePoint_C",
                "isGameClass": true,
                "location": { "x": 100.0, "y": 0.0, "z": 0.0 }
            }),
            json!({
                "name": "BP_Chest_0",
                "class": "/Game/Blueprint/Map/BP_MapGimmick_TreasureBox.BP_MapGimmick_TreasureBox_C",
                "isGameClass": true,
                "location": { "x": 0.0, "y": -150.0, "z": 0.0 }
            }),
            json!({
                "name": "BP_Exit_To_3002",
                "class": "/Game/Blueprint/Map/BP_MapTransitionTrigger.BP_MapTransitionTrigger_C",
                "isGameClass": true,
                "location": { "x": -250.0, "y": 0.0, "z": 0.0 }
            }),
            json!({
                "name": "BP_Encount_3001",
                "class": "/Game/Blueprint/Map/BP_MapEncountBox.BP_MapEncountBox_C",
                "isGameClass": true,
                "location": { "x": -300.0, "y": -300.0, "z": 0.0 }
            }),
            json!({
                "name": "BP_Far_Chest",
                "class": "/Game/Blueprint/Map/BP_MapGimmick_TreasureBox.BP_MapGimmick_TreasureBox_C",
                "isGameClass": true,
                "location": { "x": 5000.0, "y": 0.0, "z": 0.0 }
            }),
        ]);
        let summary = lvl["summary"].as_object_mut().expect("summary object");
        summary.insert(
            "treasureBoxes".to_owned(),
            json!([
                {
                    "actorName": "BP_Chest_0",
                    "label": null,
                    "class": "/Game/Blueprint/Map/BP_MapGimmick_TreasureBox.BP_MapGimmick_TreasureBox_C",
                    "uniqueId": 23,
                    "chestId": 0,
                    "mapId": 3001,
                    "itemId": 1,
                    "itemNum": 2,
                    "money": null,
                    "isTrap": false,
                    "battleGroupId": null,
                    "battleFinishEventId": null,
                    "visibleCondition": "{flag:5}",
                    "location": { "x": 0.0, "y": -150.0, "z": 0.0 }
                },
                {
                    "actorName": "BP_Far_Chest",
                    "label": null,
                    "class": "/Game/Blueprint/Map/BP_MapGimmick_TreasureBox.BP_MapGimmick_TreasureBox_C",
                    "uniqueId": 24,
                    "chestId": 1,
                    "mapId": 3001,
                    "itemId": null,
                    "itemNum": null,
                    "money": 500,
                    "isTrap": true,
                    "battleGroupId": 9001,
                    "battleFinishEventId": null,
                    "visibleCondition": null,
                    "location": { "x": 5000.0, "y": 0.0, "z": 0.0 }
                }
            ]),
        );
        summary.insert(
            "savePoints".to_owned(),
            json!([
                {
                    "actorName": "BP_SavePoint_3001",
                    "label": null,
                    "class": "/Game/Blueprint/Map/BP_MapGimmick_SavePoint.BP_MapGimmick_SavePoint_C",
                    "uniqueId": 25,
                    "savePointType": "eMapType::Dungeon",
                    "isTeleport": false,
                    "transitionMapId": null,
                    "transitionPointId": null,
                    "parentMapId": null,
                    "parentUniqueId": null,
                    "targetMapId": null,
                    "targetPointId": null,
                    "visibleCondition": null,
                    "location": { "x": 100.0, "y": 0.0, "z": 0.0 }
                }
            ]),
        );
        summary.insert(
            "transitions".to_owned(),
            json!([
                {
                    "actorName": "BP_Exit_To_3002",
                    "label": "ExitTo3002",
                    "class": "/Game/Blueprint/Map/BP_MapTransitionTrigger.BP_MapTransitionTrigger_C",
                    "uniqueId": 60,
                    "isSameMap": false,
                    "isAutoTransition": true,
                    "isEventOnly": false,
                    "condition": "{flag:100}",
                    "targets": [
                        {
                            "condition": "{flag:100}",
                            "targetMapId": 3002,
                            "targetPointId": 0,
                            "isEventOnly": false,
                            "events": [],
                            "afterTransitionEvents": []
                        }
                    ],
                    "boxExtent": null,
                    "location": { "x": -250.0, "y": 0.0, "z": 0.0 }
                }
            ]),
        );
        summary.insert(
            "encounterZones".to_owned(),
            json!([
                {
                    "actorName": "BP_Encount_3001",
                    "label": null,
                    "class": "/Game/Blueprint/Map/BP_MapEncountBox.BP_MapEncountBox_C",
                    "kind": "Box",
                    "id": 1,
                    "priority": 10,
                    "terrainId": 1,
                    "battleStageId": 2,
                    "overwriteRatio": null,
                    "isResetDistance": false,
                    "visibleCondition": null,
                    "flagCondition": null,
                    "encountDatatable": "/Game/Datatable/Encount/DT_Mock_Encount",
                    "terrainDatatables": [],
                    "boxExtent": { "x": 50.0, "y": 50.0, "z": 50.0 },
                    "sphereRadius": null,
                    "location": { "x": -300.0, "y": -300.0, "z": 0.0 }
                }
            ]),
        );
        fs::write(
            &level_3001_path,
            serde_json::to_vec_pretty(&lvl).expect("encode MockTown_EV"),
        )
        .expect("write MockTown_EV");

        // Map 3002 level: has landing point 0, savepoint teleport to 3003, and event-only exit to 3004.
        let level_3002 = json!({
            "schemaVersion": 1,
            "level": {
                "name": "MockCorridor_GD",
                "outputFile": "MockCorridor_GD.json",
                "path": "FFRS/Content/Map/Mock/MockCorridor_GD.umap",
                "packagePath": "/Game/Map/Mock/MockCorridor_GD",
                "sublevelType": "GD",
                "persistentLevel": "MockCorridor_PL",
                "persistentLevels": ["MockCorridor_PL"],
                "mapId": 3002,
                "mapIds": [3002],
                "mapRowKey": "MockCorridor",
                "mapType": "eMapType::Dungeon",
                "streamingLevels": [],
                "actorCount": 3,
                "gameActorCount": 3,
                "engineActorCount": 0
            },
            "actors": [
                {
                    "name": "BP_TP_3002_0",
                    "class": "/Game/Blueprint/Map/BP_MapTransitionPoint.BP_MapTransitionPoint_C",
                    "isGameClass": true,
                    "location": { "x": 50.0, "y": 60.0, "z": 10.0 }
                },
                {
                    "name": "BP_SavePoint_3002",
                    "class": "/Game/Blueprint/Map/BP_MapGimmick_SavePoint.BP_MapGimmick_SavePoint_C",
                    "isGameClass": true,
                    "location": { "x": 500.0, "y": 0.0, "z": 0.0 }
                },
                {
                    "name": "BP_EventExit_3004",
                    "class": "/Game/Blueprint/Map/BP_MapTransitionTrigger.BP_MapTransitionTrigger_C",
                    "isGameClass": true,
                    "location": { "x": 800.0, "y": 100.0, "z": 0.0 }
                }
            ],
            "summary": {
                "mapUnits": [],
                "treasureBoxes": [],
                "eventTriggers": [],
                "transitions": [
                    {
                        "actorName": "BP_EventExit_3004",
                        "label": null,
                        "class": "/Game/Blueprint/Map/BP_MapTransitionTrigger.BP_MapTransitionTrigger_C",
                        "uniqueId": 70,
                        "isSameMap": false,
                        "isAutoTransition": false,
                        "isEventOnly": true,
                        "condition": "{progress:0}>=1030",
                        "targets": [
                            {
                                "condition": "{progress:0}>=1030",
                                "targetMapId": 3004,
                                "targetPointId": 0,
                                "isEventOnly": true,
                                "events": [],
                                "afterTransitionEvents": []
                            }
                        ],
                        "boxExtent": null,
                        "location": { "x": 800.0, "y": 100.0, "z": 0.0 }
                    }
                ],
                "transitionPoints": [
                    {
                        "actorName": "BP_TP_3002_0",
                        "label": "Start",
                        "class": "/Game/Blueprint/Map/BP_MapTransitionPoint.BP_MapTransitionPoint_C",
                        "pointId": 0,
                        "direction": "eMapDirectionType::East",
                        "pointName": "走廊入口",
                        "location": { "x": 50.0, "y": 60.0, "z": 10.0 }
                    }
                ],
                "savePoints": [
                    {
                        "actorName": "BP_SavePoint_3002",
                        "label": null,
                        "class": "/Game/Blueprint/Map/BP_MapGimmick_SavePoint.BP_MapGimmick_SavePoint_C",
                        "uniqueId": 55,
                        "savePointType": "eMapType::Dungeon",
                        "isTeleport": true,
                        "transitionMapId": 3003,
                        "transitionPointId": 0,
                        "parentMapId": null,
                        "parentUniqueId": null,
                        "targetMapId": 3003,
                        "targetPointId": 0,
                        "visibleCondition": null,
                        "location": { "x": 500.0, "y": 0.0, "z": 0.0 }
                    }
                ],
                "shops": [],
                "inns": [],
                "gimmicks": [],
                "encounterZones": []
            }
        });
        fs::write(
            maps_dir.join("MockCorridor_GD.json"),
            serde_json::to_vec_pretty(&level_3002).expect("encode 3002"),
        )
        .expect("write 3002");

        // Map 3003 & 3004 landing points.
        let level_3003 = json!({
            "schemaVersion": 1,
            "level": {
                "name": "MockTown2_GD",
                "outputFile": "MockTown2_GD.json",
                "path": "FFRS/Content/Map/Mock/MockTown2_GD.umap",
                "packagePath": "/Game/Map/Mock/MockTown2_GD",
                "sublevelType": "GD",
                "persistentLevel": "MockTown2_PL",
                "persistentLevels": ["MockTown2_PL"],
                "mapId": 3003,
                "mapIds": [3003],
                "mapRowKey": "MockTown",
                "mapType": "eMapType::Town",
                "streamingLevels": [],
                "actorCount": 1,
                "gameActorCount": 1,
                "engineActorCount": 0
            },
            "actors": [
                {
                    "name": "BP_TP_3003_0",
                    "class": "/Game/Blueprint/Map/BP_MapTransitionPoint.BP_MapTransitionPoint_C",
                    "isGameClass": true,
                    "location": { "x": 1000.0, "y": 2000.0, "z": 0.0 }
                }
            ],
            "summary": {
                "mapUnits": [],
                "treasureBoxes": [],
                "eventTriggers": [],
                "transitions": [],
                "transitionPoints": [
                    {
                        "actorName": "BP_TP_3003_0",
                        "label": "TownArrival",
                        "class": "/Game/Blueprint/Map/BP_MapTransitionPoint.BP_MapTransitionPoint_C",
                        "pointId": 0,
                        "direction": "eMapDirectionType::North",
                        "pointName": "城镇入口",
                        "location": { "x": 1000.0, "y": 2000.0, "z": 0.0 }
                    }
                ],
                "savePoints": [],
                "shops": [],
                "inns": [],
                "gimmicks": [],
                "encounterZones": []
            }
        });
        fs::write(
            maps_dir.join("MockTown2_GD.json"),
            serde_json::to_vec_pretty(&level_3003).expect("encode 3003"),
        )
        .expect("write 3003");

        let level_3004 = json!({
            "schemaVersion": 1,
            "level": {
                "name": "DupCaveA_GD",
                "outputFile": "DupCaveA_GD.json",
                "path": "FFRS/Content/Map/Mock/DupCaveA_GD.umap",
                "packagePath": "/Game/Map/Mock/DupCaveA_GD",
                "sublevelType": "GD",
                "persistentLevel": "DupCaveA_PL",
                "persistentLevels": ["DupCaveA_PL"],
                "mapId": 3004,
                "mapIds": [3004],
                "mapRowKey": "DupCaveA",
                "mapType": "eMapType::Dungeon",
                "streamingLevels": [],
                "actorCount": 1,
                "gameActorCount": 1,
                "engineActorCount": 0
            },
            "actors": [
                {
                    "name": "BP_TP_3004_0",
                    "class": "/Game/Blueprint/Map/BP_MapTransitionPoint.BP_MapTransitionPoint_C",
                    "isGameClass": true,
                    "location": { "x": -10.0, "y": -20.0, "z": 0.0 }
                }
            ],
            "summary": {
                "mapUnits": [],
                "treasureBoxes": [],
                "eventTriggers": [],
                "transitions": [],
                "transitionPoints": [
                    {
                        "actorName": "BP_TP_3004_0",
                        "label": null,
                        "class": "/Game/Blueprint/Map/BP_MapTransitionPoint.BP_MapTransitionPoint_C",
                        "pointId": 0,
                        "direction": "eMapDirectionType::South",
                        "pointName": null,
                        "location": { "x": -10.0, "y": -20.0, "z": 0.0 }
                    }
                ],
                "savePoints": [],
                "shops": [],
                "inns": [],
                "gimmicks": [],
                "encounterZones": []
            }
        });
        fs::write(
            maps_dir.join("DupCaveA_GD.json"),
            serde_json::to_vec_pretty(&level_3004).expect("encode 3004"),
        )
        .expect("write 3004");

        // Update `_graph.json` with edges:
        // 3001 --(conditional {flag:100})--> 3002 --(savepoint_teleport)--> 3003
        //                                    3002 --(event-only {progress:0}>=1030)--> 3004
        let graph = json!({
            "schemaVersion": 1,
            "generatedAt": "2026-11-25T00:00:00Z",
            "stats": {},
            "maps": {
                "3001": {
                    "mapId": 3001,
                    "mapRowKey": "MockShrine",
                    "mapType": "eMapType::Dungeon",
                    "persistentLevel": "MockTown_PL",
                    "sublevels": [
                        { "name": "MockTown_EV", "type": "EV", "outputFile": "MockTown_EV.json" }
                    ],
                    "counts": {},
                    "exits": [
                        {
                            "kind": "transition_trigger",
                            "sourceLevel": "MockTown_EV",
                            "actorName": "BP_Exit_To_3002",
                            "uniqueId": 60,
                            "targetMapId": 3002,
                            "targetPointId": 0,
                            "isSameMap": false,
                            "isAutoTransition": true,
                            "isEventOnly": false,
                            "condition": "{flag:100}",
                            "location": { "x": -250.0, "y": 0.0, "z": 0.0 }
                        }
                    ],
                    "transitionPoints": [],
                    "shops": [],
                    "savePoints": [],
                    "treasureBoxes": []
                },
                "3002": {
                    "mapId": 3002,
                    "mapRowKey": "MockCorridor",
                    "mapType": "eMapType::Dungeon",
                    "persistentLevel": "MockCorridor_PL",
                    "sublevels": [
                        { "name": "MockCorridor_GD", "type": "GD", "outputFile": "MockCorridor_GD.json" }
                    ],
                    "counts": {},
                    "exits": [
                        {
                            "kind": "savepoint_teleport",
                            "sourceLevel": "MockCorridor_GD",
                            "actorName": "BP_SavePoint_3002",
                            "uniqueId": 55,
                            "targetMapId": 3003,
                            "targetPointId": 0,
                            "isSameMap": false,
                            "isAutoTransition": false,
                            "isEventOnly": false,
                            "condition": null,
                            "location": { "x": 500.0, "y": 0.0, "z": 0.0 }
                        },
                        {
                            "kind": "transition_trigger",
                            "sourceLevel": "MockCorridor_GD",
                            "actorName": "BP_EventExit_3004",
                            "uniqueId": 70,
                            "targetMapId": 3004,
                            "targetPointId": 0,
                            "isSameMap": false,
                            "isAutoTransition": false,
                            "isEventOnly": true,
                            "condition": "{progress:0}>=1030",
                            "location": { "x": 800.0, "y": 100.0, "z": 0.0 }
                        }
                    ],
                    "transitionPoints": [],
                    "shops": [],
                    "savePoints": [],
                    "treasureBoxes": []
                },
                "3003": {
                    "mapId": 3003,
                    "mapRowKey": "MockTown",
                    "mapType": "eMapType::Town",
                    "persistentLevel": "MockTown2_PL",
                    "sublevels": [
                        { "name": "MockTown2_GD", "type": "GD", "outputFile": "MockTown2_GD.json" }
                    ],
                    "counts": {},
                    "exits": [],
                    "transitionPoints": [],
                    "shops": [],
                    "savePoints": [],
                    "treasureBoxes": []
                },
                "3004": {
                    "mapId": 3004,
                    "mapRowKey": "DupCaveA",
                    "mapType": "eMapType::Dungeon",
                    "persistentLevel": "DupCaveA_PL",
                    "sublevels": [
                        { "name": "DupCaveA_GD", "type": "GD", "outputFile": "DupCaveA_GD.json" }
                    ],
                    "counts": {},
                    "exits": [],
                    "transitionPoints": [],
                    "shops": [],
                    "savePoints": [],
                    "treasureBoxes": []
                }
            },
            "shopsByMap": {
                "3001": [2, 100]
            },
            "shopToMaps": {
                "2": [3001],
                "100": [3001]
            },
            "unmappedLevels": []
        });
        fs::write(
            maps_dir.join("_graph.json"),
            serde_json::to_vec_pretty(&graph).expect("encode _graph.json"),
        )
        .expect("write _graph.json");
    }

    let db_path = work_dir.join("ffr3-fixture.sqlite");
    let options = BuildOptions {
        inputs: BuildInputs {
            export: Some(export_dir.clone()),
            full: Some(export_full_dir),
            maps: Some(maps_dir),
            jmaps: vec![fixture_root.join("01-mock.jmap")],
            ..Default::default()
        },
        output: db_path.clone(),
        stages: Some(vec![
            Stage::Text,
            Stage::Datatable,
            Stage::Battle,
            Stage::World,
            Stage::Reflection,
            Stage::InputUi,
            Stage::Levels,
            Stage::Sequences,
            Stage::Links,
        ]),
        game_build_id: Some("mock-ffr3".to_owned()),
        threads: None,
    };
    ffr_db::build(&options).expect("build ffr3 fixture db");

    // Seed enemy_chapter row for unit 462 (chapter 1) so chapter reporting is covered in both
    // `where_to_get` and `read_enemy`.
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open db to seed enemy_chapter");
        conn.execute(
            "INSERT INTO enemy_chapter (unit_id, chapter_no, source, battle_group_id, source_ref) VALUES (462, 1, 'encounter', 9001, 'mock_pool')",
            [],
        )
        .expect("insert enemy_chapter");
    }

    (db_path, export_dir)
}

#[tokio::test]
async fn find_route_handles_normal_savepoint_event_duplicate_not_found_and_invalid_args() {
    let temp = TestDir::new();
    let bridge = temp.path().join("bridge");
    let (db_path, _) = build_ffr3_fixture_db(temp.path());
    let module = module_with_db(&bridge, &db_path);

    // 1. Multi-hop route by ID & Chinese name: 3001 -> 3002 (conditional) -> 3003 (savepoint teleport).
    let (text, val) = call_data_ok(
        &module,
        "find_route",
        json!({ "from": "试作神殿入口", "to": 3003 }),
    )
    .await;
    assert_eq!(val["reachable"], true);
    assert_eq!(val["hops"], 2);
    assert_eq!(val["uses_savepoint_teleport"], true);
    assert_eq!(val["has_conditional_steps"], true);
    assert_eq!(val["has_event_only_steps"], false);
    let steps = val["steps"].as_array().expect("steps array");
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0]["to_map_id"], 3002);
    assert_eq!(steps[0]["condition"], "{flag:100}");
    assert_eq!(steps[0]["exit_location"]["x"], -250.0);
    assert_eq!(steps[0]["target_location"]["x"], 50.0);
    assert_eq!(steps[1]["to_map_id"], 3003);
    assert_eq!(steps[1]["is_savepoint_teleport"], true);
    assert_eq!(steps[1]["target_location"]["x"], 1000.0);
    assert!(text.contains("存档点传送"), "{text}");
    assert!(text.contains("{flag:100}"), "{text}");

    // 2. Event-only step reported explicitly (3001 -> 3004).
    let (ev_text, ev_val) =
        call_data_ok(&module, "find_route", json!({ "from": 3001, "to": 3004 })).await;
    assert_eq!(ev_val["reachable"], true);
    assert_eq!(ev_val["has_event_only_steps"], true);
    assert_eq!(ev_val["steps"][1]["is_event_only"], true);
    assert!(ev_text.contains("仅事件触发"), "{ev_text}");

    // 3. Same start & destination -> 0 hops.
    let (same_text, same_val) =
        call_data_ok(&module, "find_route", json!({ "from": 3001, "to": 3001 })).await;
    assert_eq!(same_val["hops"], 0);
    assert!(same_text.contains("0 步"), "{same_text}");

    // 4. Unreachable map -> reachable: false.
    let (unreach_text, unreach_val) =
        call_data_ok(&module, "find_route", json!({ "from": 3001, "to": 3099 })).await;
    assert_eq!(unreach_val["reachable"], false);
    assert!(unreach_text.contains("未找到"), "{unreach_text}");

    // 5. Duplicate map name lists candidates without choosing.
    let (dup_text, dup_val) = call_data_ok(
        &module,
        "find_route",
        json!({ "from": 3001, "to": "重名洞窟" }),
    )
    .await;
    assert_eq!(dup_val["ambiguous"], true);
    assert_eq!(dup_val["parameter"], "to");
    assert_eq!(dup_val["candidates"].as_array().map(Vec::len), Some(2));
    assert!(
        dup_text.contains("3004") && dup_text.contains("3005"),
        "{dup_text}"
    );

    let (_, dup_both) = call_data_ok(
        &module,
        "find_route",
        json!({ "from": "重名洞窟", "to": "重名洞窟" }),
    )
    .await;
    assert_eq!(dup_both["ambiguous"], true);
    assert_eq!(
        dup_both["from_candidates"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(dup_both["to_candidates"].as_array().map(Vec::len), Some(2));

    // 6. Not found reports which parameter failed (`from` vs `to`).
    let err_from = call_data_err(&module, "find_route", json!({ "from": 99999, "to": 3001 })).await;
    assert!(err_from.contains("`from`"), "{err_from}");
    let err_to = call_data_err(
        &module,
        "find_route",
        json!({ "from": 3001, "to": "根本没有的地图" }),
    )
    .await;
    assert!(err_to.contains("`to`"), "{err_to}");

    // 7. Invalid arguments.
    let err_missing = call_data_err(&module, "find_route", json!({ "from": 3001 })).await;
    assert!(err_missing.contains("`to`"), "{err_missing}");
    let err_type = call_data_err(&module, "find_route", json!({ "from": true, "to": 3001 })).await;
    assert!(err_type.contains("`from`"), "{err_type}");
    let err_extra = call_data_err(
        &module,
        "find_route",
        json!({ "from": 3001, "to": 3002, "extra": 1 }),
    )
    .await;
    assert!(err_extra.contains("未知参数"), "{err_extra}");
}

#[tokio::test]
async fn where_to_get_covers_chests_shops_drops_steals_quests_duplicates_and_errors() {
    let temp = TestDir::new();
    let bridge = temp.path().join("bridge");
    let (db_path, _) = build_ffr3_fixture_db(temp.path());
    let module = module_with_db(&bridge, &db_path);

    // 1. Item 1「试作药水」has ALL four source categories: chest, shop NPC, enemy drop+steal, quests.
    let (text, val) = call_data_ok(&module, "where_to_get", json!({ "name": "试作药水" })).await;
    assert_eq!(val["item_id"], 1);
    assert_eq!(val["item_name"], "试作药水");
    assert_eq!(val["counts"]["treasure_boxes"], 1);
    assert!(val["counts"]["shops"].as_u64().is_some_and(|n| n >= 1));
    assert_eq!(val["counts"]["enemy_drops"], 2); // drop_normal + steal_normal
    assert_eq!(val["counts"]["quest_rewards"], 2); // side quest + quest board

    // Chest carries coordinates & visible condition.
    assert_eq!(val["treasure_boxes"][0]["map_id"], 3001);
    assert_eq!(val["treasure_boxes"][0]["location"]["y"], -150.0);
    assert_eq!(val["treasure_boxes"][0]["visible_condition"], "{flag:5}");

    // Shop carries NPC locations, price, and NPC visible condition.
    let shop0 = &val["shops"][0];
    assert_eq!(shop0["shop_id"], 2);
    assert!(
        shop0["locations"]
            .as_array()
            .expect("shop locations")
            .iter()
            .any(|l| l["map_id"] == 3001
                && l["unique_id"] == 3
                && l["visible_condition"] == "{progress:0}>=1020")
    );

    // Enemy drops & steals carry rates and chapters.
    let drops = val["enemy_drops"].as_array().expect("enemy_drops");
    assert!(
        drops.iter().any(|d| d["unit_id"] == 462
            && d["kind"] == "drop_normal"
            && d["rate"] == 30
            && d["chapters"] == json!([1])),
        "unexpected drops: {drops:?}"
    );
    assert!(
        drops.iter().any(|d| d["unit_id"] == 462
            && d["kind"] == "steal_normal"
            && d["rate"] == 50
            && d["chapters"] == json!([1])),
        "unexpected steals: {drops:?}"
    );

    // Quest rewards include both side quest & quest board.
    let quests = val["quest_rewards"].as_array().expect("quest_rewards");
    assert!(
        quests
            .iter()
            .any(|q| q["source"] == "quest" && q["quest_id"] == 10 && q["count"] == 3)
    );
    assert!(
        quests
            .iter()
            .any(|q| q["source"] == "quest_board" && q["quest_id"] == 1001 && q["count"] == 5)
    );
    assert!(
        text.contains("【宝箱】")
            && text.contains("【商店】")
            && text.contains("【敌人掉落与偷窃】")
            && text.contains("【任务奖励】"),
        "{text}"
    );

    // 2. Source-less item reports zero sources cleanly.
    let (empty_text, empty_val) =
        call_data_ok(&module, "where_to_get", json!({ "item_id": 903 })).await;
    assert_eq!(empty_val["counts"]["treasure_boxes"], 0);
    assert!(empty_text.contains("未在静态数据表中找到"), "{empty_text}");

    // 3. Duplicate item name lists candidates without choosing.
    let (dup_text, dup_val) =
        call_data_ok(&module, "where_to_get", json!({ "name": "重名秘药" })).await;
    assert_eq!(dup_val["ambiguous"], true);
    assert_eq!(dup_val["candidates"].as_array().map(Vec::len), Some(2));
    assert!(
        dup_text.contains("901") && dup_text.contains("902"),
        "{dup_text}"
    );

    // 4. Not found & invalid args name the failing parameter.
    let err_id = call_data_err(&module, "where_to_get", json!({ "item_id": 88888 })).await;
    assert!(err_id.contains("`item_id`"), "{err_id}");
    let err_name = call_data_err(&module, "where_to_get", json!({ "name": "无此道具" })).await;
    assert!(err_name.contains("`name`"), "{err_name}");
    let err_both = call_data_err(
        &module,
        "where_to_get",
        json!({ "item_id": 1, "name": "试作药水" }),
    )
    .await;
    assert!(err_both.contains("不能同时传入"), "{err_both}");
    let err_none = call_data_err(&module, "where_to_get", json!({})).await;
    assert!(err_none.contains("必须提供"), "{err_none}");
}

#[tokio::test]
async fn read_npc_and_read_talk_cover_all_lookup_modes_duplicates_and_errors() {
    let temp = TestDir::new();
    let bridge = temp.path().join("bridge");
    let (db_path, _) = build_ffr3_fixture_db(temp.path());
    let module = module_with_db(&bridge, &db_path);

    // 1. read_npc by (map_id, unique_id) — shop NPC.
    let (shop_text, shop_val) = call_data_ok(
        &module,
        "read_npc",
        json!({ "map_id": 3001, "unique_id": 3 }),
    )
    .await;
    assert_eq!(shop_val["map_id"], 3001);
    assert_eq!(shop_val["unique_id"], 3);
    assert_eq!(shop_val["map_unit_id"], 1060);
    assert_eq!(shop_val["visible_condition"], "{progress:0}>=1020");
    assert_eq!(shop_val["is_inn"], false);
    assert_eq!(shop_val["talk_events"][0]["event_id"], "EV_NPC_SHOP");
    assert_eq!(shop_val["shops"][0]["shop_id"], 2);
    assert_eq!(shop_val["shops"][0]["items"][0]["item_id"], 1);
    assert!(shop_text.contains("EV_NPC_SHOP"), "{shop_text}");
    assert!(shop_text.contains("试作药水"), "{shop_text}");

    // 2. read_npc by (map_id, unique_id) — inn NPC.
    let (inn_text, inn_val) = call_data_ok(
        &module,
        "read_npc",
        json!({ "map_id": 3001, "unique_id": 2 }),
    )
    .await;
    assert_eq!(inn_val["is_inn"], true);
    assert_eq!(inn_val["inn_price"], 100);
    assert_eq!(inn_val["talk_events"][0]["event_id"], "EV_NPC_INN");
    assert!(inn_text.contains("旅店：是"), "{inn_text}");
    assert!(inn_text.contains("100G"), "{inn_text}");

    // 3. read_npc by map_unit_id (returns both placements of map_unit 1060).
    let (mu_text, mu_val) = call_data_ok(&module, "read_npc", json!({ "map_unit_id": 1060 })).await;
    assert_eq!(mu_val["placement_count"], 2);
    assert!(
        mu_text.contains("unique_id=2") && mu_text.contains("unique_id=3"),
        "{mu_text}"
    );

    // 4. read_npc by unique zh-Hans name (resolves directly to single placement detail).
    let (_, by_zh) = call_data_ok(&module, "read_npc", json!({ "name": "测试勇者" })).await;
    assert_eq!(by_zh["unique_id"], 4);
    assert_eq!(by_zh["various_shop_id"], 100);
    assert_eq!(by_zh["shops"].as_array().map(Vec::len), Some(2));

    // 5. read_npc by ambiguous name lists candidates without choosing.
    let (dup_text, dup_val) =
        call_data_ok(&module, "read_npc", json!({ "name": "重名旅人" })).await;
    assert_eq!(dup_val["ambiguous"], true);
    assert_eq!(dup_val["candidates"].as_array().map(Vec::len), Some(2));
    assert!(
        dup_text.contains("2001") && dup_text.contains("2002"),
        "{dup_text}"
    );

    // 6. read_npc errors identify which parameter failed.
    let err_bad_map = call_data_err(
        &module,
        "read_npc",
        json!({ "map_id": 99999, "unique_id": 1 }),
    )
    .await;
    assert!(err_bad_map.contains("`map_id`"), "{err_bad_map}");
    let err_bad_uid = call_data_err(
        &module,
        "read_npc",
        json!({ "map_id": 3001, "unique_id": 99999 }),
    )
    .await;
    assert!(err_bad_uid.contains("`unique_id`"), "{err_bad_uid}");
    let err_bad_mu = call_data_err(&module, "read_npc", json!({ "map_unit_id": 88888 })).await;
    assert!(err_bad_mu.contains("`map_unit_id`"), "{err_bad_mu}");
    let err_bad_name = call_data_err(&module, "read_npc", json!({ "name": "不存在NPC" })).await;
    assert!(err_bad_name.contains("`name`"), "{err_bad_name}");
    let err_uid_only = call_data_err(&module, "read_npc", json!({ "unique_id": 3 })).await;
    assert!(err_uid_only.contains("`map_id`"), "{err_uid_only}");
    let err_conflict = call_data_err(
        &module,
        "read_npc",
        json!({ "map_unit_id": 10, "name": "测试勇者" }),
    )
    .await;
    assert!(err_conflict.contains("不能同时传入"), "{err_conflict}");

    // 7. read_talk with lines, continuous/choice branches, and linked cutscene sequence.
    let (talk_text, talk_val) =
        call_data_ok(&module, "read_talk", json!({ "event_id": "EV_STORY_01" })).await;
    assert_eq!(talk_val["event_id"], "EV_STORY_01");
    assert_eq!(talk_val["line_count"], 3);
    assert_eq!(talk_val["lines"][0]["text"], "第一章开场旁白第一句。");
    assert_eq!(talk_val["lines"][1]["text"], "勇者准备出发。");
    assert_eq!(talk_val["lines"][2]["text"], Value::Null);
    assert_eq!(talk_val["continuous_events"][0]["event_id"], "EV_STORY_02");
    assert_eq!(
        talk_val["choice_branches"].as_array().map(Vec::len),
        Some(2)
    );
    assert!(talk_val["sequence"]["duration_secs"].as_f64().is_some());
    assert_eq!(talk_val["sequence"]["can_skip"], true);
    assert!(talk_text.contains("EV_STORY_02"), "{talk_text}");
    assert!(talk_text.contains("选项分支"), "{talk_text}");
    assert!(talk_text.contains("关联过场"), "{talk_text}");
    assert!(talk_text.contains("无中文译文"), "{talk_text}");

    // 8. read_talk with triggered battle group.
    let (bg_text, bg_val) =
        call_data_ok(&module, "read_talk", json!({ "event_id": "EV_STORY_02" })).await;
    assert_eq!(bg_val["encount_group_id"], 9001);
    assert_eq!(bg_val["battle_group"]["group_id"], 9001);
    assert_eq!(bg_val["battle_group"]["members"][0]["unit_id"], 462);
    assert!(bg_text.contains("触发战斗组：#9001"), "{bg_text}");

    // 9. read_talk not found & invalid args.
    let err_talk = call_data_err(&module, "read_talk", json!({ "event_id": "NO_SUCH_EV" })).await;
    assert!(err_talk.contains("`event_id`"), "{err_talk}");
    let err_empty = call_data_err(&module, "read_talk", json!({})).await;
    assert!(err_empty.contains("缺少必需"), "{err_empty}");
}

#[tokio::test]
async fn whats_near_supports_fake_bridge_unavailable_bridge_and_explicit_coordinates() {
    let temp = TestDir::new();
    let bridge_dir = temp.path().join("bridge");
    fs::create_dir_all(&bridge_dir).expect("create bridge dir");
    let (db_path, _) = build_ffr3_fixture_db(temp.path());
    let module = module_with_db(&bridge_dir, &db_path);

    // 1. Bridge missing -> explains reason and prompts explicit params.
    let err_missing = call_data_err(&module, "whats_near", json!({})).await;
    assert!(err_missing.contains("文件不存在"), "{err_missing}");
    assert!(err_missing.contains("map_id"), "{err_missing}");

    // 2. Bridge present but without map/coords (e.g. title screen) -> explains reason & prompts explicit.
    write_json_atomic(
        &bridge_dir.join("state.json"),
        &json!({
            "v": 1,
            "session": "test-title",
            "seq": 1,
            "clock_ms": 100,
            "data": {
                "bridge_version": "1",
                "screen_kind": "title",
                "field": { "map_id": -1, "player": { "location": null } }
            }
        }),
    )
    .await;
    let err_no_map = call_data_err(&module, "whats_near", json!({})).await;
    assert!(err_no_map.contains("未提供有效地图编号"), "{err_no_map}");
    assert!(err_no_map.contains("map_id"), "{err_no_map}");

    // 3. Fresh fake bridge state in map 3001 at (0, 0, 0) -> finds all 6 kinds sorted by distance + compass directions.
    write_json_atomic(
        &bridge_dir.join("state.json"),
        &json!({
            "v": 1,
            "session": "test-field",
            "seq": 2,
            "clock_ms": 200,
            "data": {
                "bridge_version": "1",
                "screen_kind": "field",
                "field": {
                    "map_id": 3001,
                    "player": { "location": { "x": 0.0, "y": 0.0, "z": 0.0 } }
                }
            }
        }),
    )
    .await;
    let (near_text, near_val) = call_data_ok(&module, "whats_near", json!({ "radius": 600 })).await;
    assert_eq!(near_val["position_source"], "bridge");
    assert_eq!(near_val["map_id"], 3001);
    let items = near_val["items"].as_array().expect("items array");
    // Should include event(50, East), save_point(100, North), chest(150, West),
    // shop NPC(223.6, NorthEast), inn NPC(223.6, NorthWest), exit(250, South),
    // various NPC(300, North), dangling NPC(400, North), encounter_zone(424.3, SouthWest).
    // Far chest at (5000, 0, 0) must be excluded.
    assert!(items.len() >= 8, "expected >=8 nearby items: {near_text}");
    assert!(
        !items.iter().any(|i| i["unique_id"] == 24),
        "far chest must be excluded"
    );

    // Verify strictly non-decreasing distance sort.
    let dists: Vec<f64> = items
        .iter()
        .map(|i| i["distance"].as_f64().expect("distance"))
        .collect();
    assert!(
        dists.windows(2).all(|w| w[0] <= w[1]),
        "not sorted: {dists:?}"
    );

    // Verify compass directions (+X=北, +Y=东, -X=南, -Y=西) at distance >= 50.
    assert_eq!(items[0]["kind"], "event");
    assert_eq!(items[0]["direction"], "East");
    assert_eq!(items[0]["direction_zh"], "东");
    assert_eq!(items[1]["kind"], "save_point");
    assert_eq!(items[1]["direction_zh"], "北");
    assert_eq!(items[2]["kind"], "chest");
    assert_eq!(items[2]["direction_zh"], "西");
    let exit_item = items.iter().find(|i| i["kind"] == "exit").expect("exit");
    assert_eq!(exit_item["direction_zh"], "南");
    let enc_item = items
        .iter()
        .find(|i| i["kind"] == "encounter_zone")
        .expect("encounter_zone");
    assert_eq!(enc_item["direction_zh"], "西南");
    assert!(near_text.contains("来自文件桥实时位置"), "{near_text}");
    assert!(
        near_text.contains("方向按 +X 为北推算，未经真机核对"),
        "{near_text}"
    );

    // 3b. "就在这里" threshold (< 50): querying 0.3 units and 49 units from chest #0 at (0, -150, 0)
    //     omits direction and sets JSON `direction: null`; at exact distance 50 gives West.
    let (here_03_text, here_03_val) = call_data_ok(
        &module,
        "whats_near",
        json!({
            "map_id": 3001,
            "x": 0.3,
            "y": -150.0,
            "z": 0.0,
            "radius": 100,
            "kinds": ["chest"]
        }),
    )
    .await;
    assert_eq!(here_03_val["items"][0]["direction"], Value::Null);
    assert_eq!(here_03_val["items"][0]["direction_zh"], Value::Null);
    assert_eq!(here_03_val["items"][0]["bearing_deg"], Value::Null);
    assert!(
        here_03_text.contains("- [宝箱] 就在这里：宝箱 #0"),
        "{here_03_text}"
    );
    assert!(
        !here_03_text.contains("距离 0"),
        "should not show 距离 0: {here_03_text}"
    );

    let (here_49_text, here_49_val) = call_data_ok(
        &module,
        "whats_near",
        json!({
            "map_id": 3001,
            "x": 0.0,
            "y": -101.0,
            "z": 0.0,
            "radius": 100,
            "kinds": ["chest"]
        }),
    )
    .await;
    assert_eq!(here_49_val["items"][0]["direction"], Value::Null);
    assert!(
        here_49_text.contains("- [宝箱] 就在这里："),
        "{here_49_text}"
    );

    let (at_50_text, at_50_val) = call_data_ok(
        &module,
        "whats_near",
        json!({
            "map_id": 3001,
            "x": 0.0,
            "y": -100.0,
            "z": 0.0,
            "radius": 100,
            "kinds": ["chest"]
        }),
    )
    .await;
    assert_eq!(at_50_val["items"][0]["direction"], "West");
    assert_eq!(at_50_val["items"][0]["direction_zh"], "西");
    assert!(at_50_text.contains("距离 50（西）"), "{at_50_text}");

    // 4. Stale bridge state -> fails with staleness reason + explicit-param hint.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let err_stale = call_data_err(&module, "whats_near", json!({})).await;
    assert!(err_stale.contains("状态停止更新"), "{err_stale}");
    assert!(err_stale.contains("map_id"), "{err_stale}");

    // 5. Explicit parameters work offline even when bridge is stale, and `kinds` filters cleanly.
    let (_, explicit_val) = call_data_ok(
        &module,
        "whats_near",
        json!({
            "map_id": 3001,
            "x": 0.0,
            "y": 0.0,
            "radius": 600,
            "kinds": ["chest", "save_point"]
        }),
    )
    .await;
    assert_eq!(explicit_val["position_source"], "explicit");
    let filt = explicit_val["items"].as_array().expect("filtered items");
    assert_eq!(filt.len(), 2);
    assert!(
        filt.iter()
            .all(|i| matches!(i["kind"].as_str(), Some("chest" | "save_point")))
    );

    // 6. Invalid arguments.
    let err_partial =
        call_data_err(&module, "whats_near", json!({ "map_id": 3001, "x": 1.0 })).await;
    assert!(err_partial.contains("`x` 和 `y`"), "{err_partial}");
    let err_rad = call_data_err(
        &module,
        "whats_near",
        json!({ "map_id": 3001, "x": 0.0, "y": 0.0, "radius": 0 }),
    )
    .await;
    assert!(err_rad.contains("必须大于 0"), "{err_rad}");
    let err_kind = call_data_err(
        &module,
        "whats_near",
        json!({ "map_id": 3001, "x": 0.0, "y": 0.0, "kinds": ["bogus"] }),
    )
    .await;
    assert!(err_kind.contains("不支持类别"), "{err_kind}");
    let err_map = call_data_err(
        &module,
        "whats_near",
        json!({ "map_id": 99999, "x": 0.0, "y": 0.0 }),
    )
    .await;
    assert!(err_map.contains("`map_id`"), "{err_map}");
}

#[tokio::test]
async fn query_game_db_enforces_row_cap_size_truncation_timeout_and_readonly_safety() {
    let temp = TestDir::new();
    let bridge = temp.path().join("bridge");
    let (db_path, _) = build_ffr3_fixture_db(temp.path());
    let module = module_with_db(&bridge, &db_path);

    // 1. Normal SELECT query.
    let (ok_text, ok_val) = call_data_ok(
        &module,
        "query_game_db",
        json!({ "sql": "SELECT id, map_type FROM map ORDER BY id LIMIT 3" }),
    )
    .await;
    assert_eq!(ok_val["returned_rows"], 3);
    assert_eq!(ok_val["truncated_by_row_limit"], false);
    assert_eq!(ok_val["truncated_by_size"], false);
    assert!(ok_text.contains("返回 3 行"), "{ok_text}");

    // 2. Exceeding 200-row cap -> capped at 200 and flagged.
    let (cap_text, cap_val) = call_data_ok(
        &module,
        "query_game_db",
        json!({
            "sql": "WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 260) SELECT n FROM c"
        }),
    )
    .await;
    assert_eq!(cap_val["returned_rows"], 200);
    assert_eq!(cap_val["truncated_by_row_limit"], true);
    assert!(cap_text.contains("超过行数上限 200 行"), "{cap_text}");

    // 3. Output exceeding ~20 KB -> truncated with exact omission report.
    let (big_text, big_val) = call_data_ok(
        &module,
        "query_game_db",
        json!({
            "sql": "WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 60) SELECT n, printf('%01000d', n) AS pad FROM c"
        }),
    )
    .await;
    assert_eq!(big_val["truncated_by_size"], true);
    assert!(big_val["omitted_rows"].as_u64().is_some_and(|n| n > 0));
    assert!(big_val["omitted_bytes"].as_u64().is_some_and(|n| n > 0));
    let serialized = serde_json::to_string(&big_val).expect("serialize big_val");
    assert!(
        serialized.len() <= 22 * 1024,
        "truncated JSON should stay near ~20 KB, got {} bytes",
        serialized.len()
    );
    assert!(big_text.contains("20 KB"), "{big_text}");
    assert!(big_text.contains("略去其余"), "{big_text}");

    // 4. Write statements and multi-statements are rejected.
    for bad_sql in [
        "DELETE FROM map",
        "UPDATE map SET danger_level = 0",
        "INSERT INTO meta (key, value) VALUES ('a', 'b')",
        "DROP TABLE map",
        "CREATE TABLE evil (x INT)",
        "PRAGMA journal_mode = WAL",
        "SELECT 1; SELECT 2",
    ] {
        let err = call_data_err(&module, "query_game_db", json!({ "sql": bad_sql })).await;
        assert!(
            err.contains("拒绝执行"),
            "expected reject for `{bad_sql}`: {err}"
        );
    }

    // 5. Invalid `limit` rejected.
    let err_lim = call_data_err(
        &module,
        "query_game_db",
        json!({ "sql": "SELECT 1", "limit": 201 }),
    )
    .await;
    assert!(err_lim.contains("1..=200"), "{err_lim}");

    // 6. Infinite CTE hits 2s SQLite progress handler timeout cleanly, and subsequent queries still succeed.
    let start = Instant::now();
    let err_timeout = call_data_err(
        &module,
        "query_game_db",
        json!({ "sql": "WITH RECURSIVE spin(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM spin) SELECT COUNT(*) FROM spin" }),
    )
    .await;
    let elapsed = start.elapsed();
    assert!(err_timeout.contains("超时"), "{err_timeout}");
    assert!(err_timeout.contains("ffr://db/schema"), "{err_timeout}");
    assert!(
        elapsed >= Duration::from_millis(1500) && elapsed <= Duration::from_secs(6),
        "timeout elapsed out of expected ~2s window: {elapsed:?}"
    );

    let (_, after_val) =
        call_data_ok(&module, "query_game_db", json!({ "sql": "SELECT 42 AS v" })).await;
    assert_eq!(after_val["rows"][0]["v"], 42);

    // 7. `ffr://db/schema` resource describes all tables with Chinese descriptions & columns.
    let schema_raw = read_all_resource_text(&module, "ffr://db/schema").await;
    let schema: Value = serde_json::from_str(&schema_raw).expect("valid schema JSON");
    assert_eq!(schema["schema_version"], ffr_db::SCHEMA_VERSION);
    assert!(schema["table_count"].as_u64().is_some_and(|n| n >= 89));
    let tables = schema["tables"].as_array().expect("tables array");
    for required in [
        "map",
        "item",
        "unit",
        "level_object",
        "map_edge",
        "talk_event",
        "text_untranslated",
        "item_untranslated",
        "skill_untranslated",
        "talk_line_untranslated",
        "map_unit_untranslated",
    ] {
        let entry = tables
            .iter()
            .find(|t| t["name"] == required)
            .unwrap_or_else(|| panic!("missing `{required}` in ffr://db/schema"));
        assert!(
            entry["description"].as_str().is_some_and(|s| !s.is_empty()),
            "missing description for {required}"
        );
        assert!(
            entry["columns"].as_array().is_some_and(|c| !c.is_empty()),
            "missing columns for {required}"
        );
    }
}

#[tokio::test]
async fn untranslated_entities_in_ffr3_tools_render_honest_text_and_json() {
    let temp = TestDir::new();
    let bridge = temp.path().join("bridge");
    let links_src = repo_root().join("test/fixtures/db/links");
    let fixture_root = temp.path().join("fixture");
    copy_dir_recursive(&links_src, &fixture_root);

    let export_dir = fixture_root.join("export");
    let export_full_dir = fixture_root.join("export-full");
    let maps_dir = fixture_root.join("maps");

    let ja_item_name = "テスト未訳アイテム";
    let ja_item_desc = "テスト未訳アイテムの説明";
    let ja_skill_name = "テスト未訳スキル";
    let ja_skill_desc = "テスト未訳スキルの説明";
    let ja_enemy_name = "テスト未訳モンスター";
    let ja_npc_name = "テスト未訳勇者";
    let ja_talk_line = "未翻訳テスト台詞。";

    // 1. Mark item 1, skill 100010, enemy 462, playable/map_unit 10 (unit 1001), and talk L3_UNTRANS
    //    as officially untranslated by storing identical kana Japanese strings in `ja.json` and `zh-Hans.json`.
    for lang in ["ja", "zh-Hans"] {
        let loc_path = export_dir.join(format!("Localization/{lang}.json"));
        let mut loc: Value =
            serde_json::from_str(&fs::read_to_string(&loc_path).expect("read loc"))
                .expect("parse loc");
        let world_ns = loc["ST_MockWorld"].as_object_mut().expect("ST_MockWorld");
        world_ns.insert("ITEM_1".to_owned(), json!(ja_item_name));
        world_ns.insert("ITEM_1_DESC".to_owned(), json!(ja_item_desc));
        world_ns.insert("SKILL_100010".to_owned(), json!(ja_skill_name));
        world_ns.insert("SKILL_100010_DESC".to_owned(), json!(ja_skill_desc));
        world_ns.insert("UNIT_462".to_owned(), json!(ja_enemy_name));
        world_ns.insert("UNIT_1001".to_owned(), json!(ja_npc_name));
        let story_ns = loc["ST_MockStory"].as_object_mut().expect("ST_MockStory");
        story_ns.insert("L3_UNTRANS".to_owned(), json!(ja_talk_line));
        fs::write(
            &loc_path,
            serde_json::to_vec_pretty(&loc).expect("encode loc"),
        )
        .expect("write loc");
    }

    // Attach untranslated Description to item 1 and skill 100010, and untranslated Name + drop to enemy 462.
    {
        let item_path = export_dir.join("FFRS/Content/Datatable/Item/DT_ConsumablesItemData.json");
        let mut doc: Value =
            serde_json::from_str(&fs::read_to_string(&item_path).expect("read item dt"))
                .expect("parse item dt");
        let rows = doc[0]["Rows"].as_object_mut().expect("item rows");
        for row in rows.values_mut() {
            if row["ID"] == 1 {
                row["Description"] = json!({
                    "TableId": "/Game/Datatable/Strings/ST_MockWorld.ST_MockWorld",
                    "Key": "ITEM_1_DESC",
                    "SourceString": ja_item_desc,
                    "LocalizedString": ja_item_desc
                });
            }
        }
        fs::write(
            &item_path,
            serde_json::to_vec_pretty(&doc).expect("encode item dt"),
        )
        .expect("write item dt");
    }
    {
        let skill_path = export_dir.join("FFRS/Content/Datatable/Skill/DT_SkillData.json");
        let mut doc: Value =
            serde_json::from_str(&fs::read_to_string(&skill_path).expect("read skill dt"))
                .expect("parse skill dt");
        doc[0]["Rows"]["MockSlash"]["Description"] = json!({
            "TableId": "/Game/Datatable/Strings/ST_MockWorld.ST_MockWorld",
            "Key": "SKILL_100010_DESC",
            "SourceString": ja_skill_desc,
            "LocalizedString": ja_skill_desc
        });
        fs::write(
            &skill_path,
            serde_json::to_vec_pretty(&doc).expect("encode skill dt"),
        )
        .expect("write skill dt");
    }
    {
        let unit_path = export_dir.join("FFRS/Content/Datatable/Unit/DT_UnitParameter.json");
        let mut doc: Value =
            serde_json::from_str(&fs::read_to_string(&unit_path).expect("read unit dt"))
                .expect("parse unit dt");
        let enemy = doc[0]["Rows"]["テスト敵"]
            .as_object_mut()
            .expect("enemy 462 row");
        enemy.insert(
            "Name".to_owned(),
            json!({
                "TableId": "/Game/Datatable/Strings/ST_MockWorld.ST_MockWorld",
                "Key": "UNIT_462",
                "SourceString": ja_enemy_name,
                "LocalizedString": ja_enemy_name
            }),
        );
        enemy.insert("DropNormalItem".to_owned(), json!(1));
        enemy.insert("DropNormalItemRate".to_owned(), json!(25));
        fs::write(
            &unit_path,
            serde_json::to_vec_pretty(&doc).expect("encode unit dt"),
        )
        .expect("write unit dt");
    }

    let db_path = temp.path().join("ffr3-untranslated.sqlite");
    let options = BuildOptions {
        inputs: BuildInputs {
            export: Some(export_dir),
            full: Some(export_full_dir),
            maps: Some(maps_dir),
            jmaps: vec![fixture_root.join("01-mock.jmap")],
            ..Default::default()
        },
        output: db_path.clone(),
        stages: Some(vec![
            Stage::Text,
            Stage::Datatable,
            Stage::Battle,
            Stage::World,
            Stage::Reflection,
            Stage::InputUi,
            Stage::Levels,
            Stage::Sequences,
            Stage::Links,
        ]),
        game_build_id: None,
        threads: None,
    };
    ffr_db::build(&options).expect("build untranslated ffr3 fixture db");

    let module = module_with_db(&bridge, &db_path);

    // 1. `read_item`
    let (item_text, item_val) = call_data_ok(&module, "read_item", json!({ "item_id": 1 })).await;
    assert!(
        item_text.contains(&format!("（游戏中文版未翻译，日文原文：{ja_item_name}）")),
        "{item_text}"
    );
    assert!(
        item_text.contains(&format!("（游戏中文版未翻译，日文原文：{ja_item_desc}）")),
        "{item_text}"
    );
    assert_eq!(item_val["name"], Value::Null);
    assert_eq!(item_val["name_untranslated"], true);
    assert_eq!(item_val["name_ja"], ja_item_name);
    assert_eq!(item_val["description"], Value::Null);
    assert_eq!(item_val["description_untranslated"], true);
    assert_eq!(item_val["description_ja"], ja_item_desc);

    // 2. `read_skill`
    let (skill_text, skill_val) =
        call_data_ok(&module, "read_skill", json!({ "skill_id": 100010 })).await;
    assert!(
        skill_text.contains(&format!("（游戏中文版未翻译，日文原文：{ja_skill_name}）")),
        "{skill_text}"
    );
    assert!(
        skill_text.contains(&format!("（游戏中文版未翻译，日文原文：{ja_skill_desc}）")),
        "{skill_text}"
    );
    assert_eq!(skill_val["name"], Value::Null);
    assert_eq!(skill_val["name_untranslated"], true);
    assert_eq!(skill_val["name_ja"], ja_skill_name);
    assert_eq!(skill_val["description"], Value::Null);
    assert_eq!(skill_val["description_untranslated"], true);
    assert_eq!(skill_val["description_ja"], ja_skill_desc);

    // 3. `read_enemy` (untranslated enemy name + untranslated dropped item)
    let (enemy_text, enemy_val) =
        call_data_ok(&module, "read_enemy", json!({ "unit_id": 462 })).await;
    assert!(
        enemy_text.contains(&format!("（游戏中文版未翻译，日文原文：{ja_enemy_name}）")),
        "{enemy_text}"
    );
    assert!(
        enemy_text.contains(&format!("（游戏中文版未翻译，日文原文：{ja_item_name}）")),
        "{enemy_text}"
    );
    assert_eq!(enemy_val["name"], Value::Null);
    assert_eq!(enemy_val["name_untranslated"], true);
    assert_eq!(enemy_val["name_ja"], ja_enemy_name);
    assert_eq!(enemy_val["drops"]["normal"]["item_name"], Value::Null);
    assert_eq!(enemy_val["drops"]["normal"]["item_name_untranslated"], true);
    assert_eq!(enemy_val["drops"]["normal"]["item_name_ja"], ja_item_name);

    // 4. `where_to_get` (untranslated item + untranslated enemy drop source)
    let (wtg_text, wtg_val) = call_data_ok(&module, "where_to_get", json!({ "item_id": 1 })).await;
    assert!(
        wtg_text.contains(&format!("（游戏中文版未翻译，日文原文：{ja_item_name}）")),
        "{wtg_text}"
    );
    assert!(
        wtg_text.contains(&format!("（游戏中文版未翻译，日文原文：{ja_enemy_name}）")),
        "{wtg_text}"
    );
    assert_eq!(wtg_val["item_name"], Value::Null);
    assert_eq!(wtg_val["item_name_untranslated"], true);
    assert_eq!(wtg_val["item_name_ja"], ja_item_name);
    assert_eq!(wtg_val["enemy_drops"][0]["unit_name"], Value::Null);
    assert_eq!(wtg_val["enemy_drops"][0]["unit_name_untranslated"], true);
    assert_eq!(wtg_val["enemy_drops"][0]["unit_name_ja"], ja_enemy_name);

    // 5. `read_npc` (by map_unit_id and by Japanese name)
    let (npc_text, npc_val) = call_data_ok(&module, "read_npc", json!({ "map_unit_id": 10 })).await;
    assert!(
        npc_text.contains(&format!("（游戏中文版未翻译，日文原文：{ja_npc_name}）")),
        "{npc_text}"
    );
    assert_eq!(npc_val["name_zh"], Value::Null);
    assert_eq!(npc_val["name_untranslated"], true);
    assert_eq!(npc_val["name_ja"], ja_npc_name);

    let (npc_by_name_text, npc_by_name_val) =
        call_data_ok(&module, "read_npc", json!({ "name": ja_npc_name })).await;
    assert!(
        npc_by_name_text.contains(&format!("（游戏中文版未翻译，日文原文：{ja_npc_name}）")),
        "{npc_by_name_text}"
    );
    assert_eq!(npc_by_name_val["name_zh"], Value::Null);
    assert_eq!(npc_by_name_val["name_untranslated"], true);
    assert_eq!(npc_by_name_val["name_ja"], ja_npc_name);

    // 6. `read_talk` (line L3_UNTRANS is officially untranslated)
    let (talk_text, talk_val) =
        call_data_ok(&module, "read_talk", json!({ "event_id": "EV_STORY_01" })).await;
    assert!(
        talk_text.contains(&format!("（游戏中文版未翻译，日文原文：{ja_talk_line}）")),
        "{talk_text}"
    );
    let untrans_line = talk_val["lines"]
        .as_array()
        .expect("talk lines")
        .iter()
        .find(|l| l["row_key"] == "Line_M")
        .expect("Line_M present");
    assert_eq!(untrans_line["text"], Value::Null);
    assert_eq!(untrans_line["text_untranslated"], true);
    assert_eq!(untrans_line["text_ja"], ja_talk_line);

    // 7. `search_data` finds untranslated item, skill, and enemy
    let (search_text, search_val) =
        call_data_ok(&module, "search_data", json!({ "query": "テスト未訳" })).await;
    assert!(
        search_text.contains(&format!("（游戏中文版未翻译，日文原文：{ja_item_name}）")),
        "{search_text}"
    );
    assert!(
        search_text.contains(&format!("（游戏中文版未翻译，日文原文：{ja_skill_name}）")),
        "{search_text}"
    );
    assert!(
        search_text.contains(&format!("（游戏中文版未翻译，日文原文：{ja_enemy_name}）")),
        "{search_text}"
    );
    assert_eq!(search_val["total_matches"], 3, "{search_val}");
}

#[tokio::test]
async fn enriched_read_map_and_read_enemy_and_legacy_json_fallback_errors() {
    let temp = TestDir::new();
    let bridge = temp.path().join("bridge");
    let (db_path, export_dir) = build_ffr3_fixture_db(temp.path());
    let db_mod = module_with_db(&bridge, &db_path);

    // Enriched `read_map` includes exits, npcs, shops, treasure boxes with coords, save points, encounter zones.
    let (map_text, map_val) = call_data_ok(&db_mod, "read_map", json!({ "map_id": 3001 })).await;
    assert!(map_val["exits"].as_array().is_some_and(|a| !a.is_empty()));
    assert!(map_val["npcs"].as_array().is_some_and(|a| a.len() >= 3));
    assert!(map_val["shops"].as_array().is_some_and(|a| !a.is_empty()));
    assert!(
        map_val["save_points"]
            .as_array()
            .is_some_and(|a| a.len() == 1)
    );
    assert!(
        map_val["encounter_zones"]
            .as_array()
            .is_some_and(|a| a.len() == 1)
    );
    assert_eq!(map_val["treasure_boxes"][0]["location"]["y"], -150.0);
    assert!(map_text.contains("出口："), "{map_text}");
    assert!(map_text.contains("存档点位置："), "{map_text}");
    assert!(map_text.contains("遇敌区域：1 处"), "{map_text}");

    // Enriched `read_enemy` includes drops, steals, chapters, and battle_groups.
    let (enemy_text, enemy_val) =
        call_data_ok(&db_mod, "read_enemy", json!({ "unit_id": 462 })).await;
    assert_eq!(enemy_val["drops"]["normal"]["item_id"], 1);
    assert_eq!(enemy_val["drops"]["normal"]["rate"], 30);
    assert_eq!(enemy_val["drops"]["rare"]["item_id"], 2);
    assert_eq!(enemy_val["steals"]["normal"]["item_id"], 1);
    assert_eq!(enemy_val["steals"]["rare"]["item_id"], 2);
    assert_eq!(enemy_val["chapters"], json!([1]));
    assert_eq!(enemy_val["battle_groups"][0]["group_id"], 9001);
    assert!(enemy_text.contains("掉落："), "{enemy_text}");
    assert!(enemy_text.contains("偷窃："), "{enemy_text}");
    assert!(enemy_text.contains("所在战斗组：#9001"), "{enemy_text}");

    // Legacy `--data-dir` JSON backend rejects the 6 DB-only tools cleanly in Chinese.
    let json_mod = module_with_data(&bridge, &export_dir);
    for (tool, args) in [
        ("find_route", json!({ "from": 3001, "to": 3002 })),
        ("where_to_get", json!({ "item_id": 1 })),
        ("read_npc", json!({ "map_unit_id": 1060 })),
        ("read_talk", json!({ "event_id": "EV_STORY_01" })),
        ("whats_near", json!({ "map_id": 3001, "x": 0.0, "y": 0.0 })),
        ("query_game_db", json!({ "sql": "SELECT 1" })),
    ] {
        let err = call_data_err(&json_mod, tool, args).await;
        assert!(
            err.contains("需要本地 SQLite 游戏数据库"),
            "tool `{tool}`: {err}"
        );
    }
}

fn real_db_path() -> PathBuf {
    if let Some(deps) = std::env::var_os(FFR_DEPS_ENV)
        && !deps.is_empty()
    {
        return default_db_path_from_deps(PathBuf::from(deps));
    }
    default_db_path_from_deps(repo_root().join(".deps"))
}

#[tokio::test]
async fn real_db_six_ffr3_spot_checks() {
    let db_path = real_db_path();
    let Some(db) = open_real_db_or_skip(&db_path, "真实数据库 ffr-3 抽查") else {
        return;
    };
    let has_full = db.has_battle_data().unwrap_or(false)
        && db.has_world_data().unwrap_or(false)
        && db.map_exits(3001).is_ok_and(|e| !e.is_empty());
    assert!(
        has_full,
        "真实数据库 {} 缺少关卡或核心表；请运行 `ffr-db build` 重建完整数据库",
        db_path.display()
    );
    drop(db);

    let temp = TestDir::new();
    let bridge = temp.path().join("bridge");
    let module = module_with_db(&bridge, &db_path);

    // Spot-check 1: route from map 3001 to 1000.
    let (_, route_val) =
        call_data_ok(&module, "find_route", json!({ "from": 3001, "to": 1000 })).await;
    assert_eq!(route_val["reachable"], true);
    assert!(route_val["hops"].as_u64().is_some_and(|h| h >= 1));
    assert_eq!(
        route_val["steps"]
            .as_array()
            .and_then(|a| a.last())
            .and_then(|s| s["to_map_id"].as_i64()),
        Some(1000)
    );

    // Spot-check 2: where to get 恢复药.
    let (_, potion_val) = call_data_ok(&module, "where_to_get", json!({ "name": "恢复药" })).await;
    assert_eq!(potion_val["item_id"], 1);
    assert!(
        potion_val["counts"]["treasure_boxes"]
            .as_u64()
            .is_some_and(|n| n > 0)
    );
    assert!(
        potion_val["counts"]["shops"]
            .as_u64()
            .is_some_and(|n| n > 0)
    );
    assert!(
        potion_val["counts"]["enemy_drops"]
            .as_u64()
            .is_some_and(|n| n > 0)
    );

    // Spot-check 3: map 3001 shop NPC (unique_id=3).
    let (_, npc_val) = call_data_ok(
        &module,
        "read_npc",
        json!({ "map_id": 3001, "unique_id": 3 }),
    )
    .await;
    assert_eq!(npc_val["map_id"], 3001);
    assert_eq!(npc_val["unique_id"], 3);
    assert_eq!(npc_val["is_inn"], false);
    assert!(npc_val["shops"].as_array().is_some_and(|shops| {
        !shops.is_empty()
            && shops[0]["items"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
    }));

    // Spot-check 4: Chapter 1 first talk (C01_001_01) — verify metadata only, never log lines.
    let (_, talk_val) =
        call_data_ok(&module, "read_talk", json!({ "event_id": "C01_001_01" })).await;
    assert_eq!(talk_val["event_id"], "C01_001_01");
    assert!(talk_val["line_count"].as_u64().is_some_and(|n| n > 0));
    assert!(talk_val["sequence"].is_object());

    // Spot-check 5: objects within 2000 of map 3001 save point (-3861.933, -550.627, -200.0).
    let (near_text, near_val) = call_data_ok(
        &module,
        "whats_near",
        json!({
            "map_id": 3001,
            "x": -3861.933,
            "y": -550.627,
            "z": -200.0,
            "radius": 2000
        }),
    )
    .await;
    assert_eq!(near_val["position_source"], "explicit");
    let near_items = near_val["items"].as_array().expect("near items");
    assert!(
        near_items.len() >= 5,
        "expected >=5 items near 3001 save point"
    );
    assert_eq!(near_items[0]["kind"], "save_point");
    assert_eq!(near_items[0]["direction"], Value::Null);
    assert!(near_text.contains("- [存档点] 就在这里："), "{near_text}");
    assert!(
        near_text.contains("方向按 +X 为北推算，未经真机核对"),
        "{near_text}"
    );
    assert!(near_items.iter().any(|i| i["kind"] == "npc"));
    assert!(near_items.iter().any(|i| i["kind"] == "chest"));
    assert!(near_items.iter().any(|i| i["kind"] == "exit"));

    // Spot-check 6: one `query_game_db` read-only SQL query.
    let (_, sql_val) = call_data_ok(
        &module,
        "query_game_db",
        json!({ "sql": "SELECT chapter_no, row_key FROM chapter ORDER BY chapter_no" }),
    )
    .await;
    assert!(sql_val["returned_rows"].as_u64().is_some_and(|n| n >= 1));
    assert_eq!(sql_val["truncated_by_row_limit"], false);
    assert_eq!(sql_val["truncated_by_size"], false);

    // Ensure `ffr://db/schema` is also readable through the real-DB module.
    let schema_raw = read_all_resource_text(&module, "ffr://db/schema").await;
    let schema: Value = serde_json::from_str(&schema_raw).expect("valid schema JSON");
    assert!(schema["table_count"].as_u64().is_some_and(|n| n >= 70));
}
