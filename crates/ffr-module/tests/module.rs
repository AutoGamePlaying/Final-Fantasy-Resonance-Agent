mod common;

use agp_core::RunId;
use agp_game::{CallContext, Content, GameModule, ResourceRange, SessionStart};
use common::{FakeMod, TestDir, call_json, module, write_state};
use serde_json::json;

#[tokio::test]
async fn game_status_returns_fresh_bridge_data_unchanged() {
    let temp = TestDir::new();
    let fake_mod = FakeMod::start(temp.path()).await;
    let module = module(temp.path());

    let (output, status) = call_json(&module, "game_status").await;

    assert!(!output.is_error);
    assert_eq!(status["bridge"]["ok"], true);
    assert_eq!(status["bridge"]["session"], "fake-session");
    assert!(status["bridge"]["seq"].as_u64().is_some());
    assert!(status["bridge"]["file_age_ms"].as_u64().is_some());
    assert_eq!(status["bridge"]["data"]["bridge_version"], "0");
    assert_eq!(status["bridge"]["data"]["pawn_location"]["x"], 1.0);
    assert!(status.get("process").is_some());
    assert!(status.get("window").is_some());

    fake_mod.stop().await;
}

#[tokio::test]
async fn game_status_reports_an_empty_bridge_without_failing_the_tool() {
    let temp = TestDir::new();
    let module = module(temp.path());

    let (output, status) = call_json(&module, "game_status").await;

    assert!(!output.is_error);
    assert_eq!(status["bridge"]["ok"], false);
    assert!(
        status["bridge"]["error"]
            .as_str()
            .expect("bridge error is text")
            .contains("文件不存在")
    );
    assert!(status.get("process").is_some());
    assert!(status.get("window").is_some());
}

#[tokio::test]
async fn game_status_reports_stale_state_without_failing_the_tool() {
    let temp = TestDir::new();
    write_state(temp.path(), "stale-session", 1).await;
    let module = module(temp.path());

    let (_, first) = call_json(&module, "game_status").await;
    assert_eq!(first["bridge"]["ok"], true);
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let (output, second) = call_json(&module, "game_status").await;
    assert!(!output.is_error);
    assert_eq!(second["bridge"]["ok"], false);
    assert!(
        second["bridge"]["error"]
            .as_str()
            .expect("bridge error is text")
            .contains("状态停止更新")
    );
}

#[tokio::test]
async fn bridge_ping_returns_round_trip_time() {
    let temp = TestDir::new();
    let fake_mod = FakeMod::start(temp.path()).await;
    let module = module(temp.path());

    let (output, result) = call_json(&module, "bridge_ping").await;

    assert!(!output.is_error);
    assert_eq!(result["ok"], true);
    assert!(result["round_trip_ms"].as_u64().is_some());

    fake_mod.stop().await;
}

#[tokio::test]
async fn bridge_ping_timeout_is_a_tool_error_with_a_chinese_reason() {
    let temp = TestDir::new();
    let module = module(temp.path());

    let (output, result) = call_json(&module, "bridge_ping").await;

    assert!(output.is_error);
    assert_eq!(result["ok"], false);
    let error = result["error"].as_str().expect("ping error is text");
    assert!(error.contains("桥 ping 失败"));
    assert!(error.contains("没有回复命令 ping"));
}

#[tokio::test]
async fn only_invalid_arguments_make_game_status_an_error() {
    let temp = TestDir::new();
    let module = module(temp.path());

    let output = module
        .call_tool(
            "game_status",
            json!({"unexpected": true}),
            &CallContext::new(RunId::new()),
        )
        .await
        .expect("invalid arguments return tool output");

    assert!(output.is_error);
    let Content::Text { text } = &output.content[0] else {
        panic!("argument error must be text");
    };
    assert!(text.contains("不接受参数"));
}

#[tokio::test]
async fn session_creates_the_bridge_directory_and_missing_data_is_reported() {
    let temp = TestDir::new();
    let bridge_dir = temp.path().join("nested/bridge");
    let module = module(&bridge_dir);

    module
        .start_session(SessionStart {
            run_id: RunId::new(),
            start_save: None,
        })
        .await
        .expect("session creates bridge directory");
    assert!(bridge_dir.is_dir());
    module.end_session().await.expect("end session succeeds");

    let resources = module
        .list_resources()
        .await
        .expect("resources list succeeds");
    assert_eq!(resources.len(), 4);
    assert!(resources.iter().all(|res| res.size_hint.is_none()));

    let missing_error = module
        .read_resource(
            "ffr://missing",
            ResourceRange {
                offset: 0,
                length: 1,
            },
        )
        .await
        .expect_err("unknown resource fails");
    assert!(missing_error.to_string().contains("没有这个资源"));

    let unconfigured_error = module
        .read_resource(
            "ffr://chapters",
            ResourceRange {
                offset: 0,
                length: 64,
            },
        )
        .await
        .expect_err("unconfigured data dir fails cleanly");
    assert!(
        unconfigured_error
            .to_string()
            .contains("未配置游戏数据目录")
    );
}
