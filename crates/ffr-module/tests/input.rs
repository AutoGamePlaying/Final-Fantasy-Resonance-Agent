//! Tests for opt-in R4 input injection in `ffr-module`:
//! - `--enable-input` gating on tool listing & dispatch (off by default);
//! - `<bridge_dir>/input.enabled` switch-file creation & cleanup across session and CLI lifecycle;
//! - command forwarding for `press` and `move`, parameter validation, and mod error propagation.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use agp_core::RunId;
use agp_game::{
    CallContext, Content, GameError, GameModule, PROTOCOL_VERSION, SessionStart, ToolCategory,
};
use common::{TestDir, call_data_ok, module, write_json_atomic};
use ffr_module::INPUT_SWITCH_FILE;
use serde_json::{Value, json};
use tokio::fs;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;

type AckHandler = Arc<dyn Fn(&str, &Value) -> (bool, Value, Option<String>) + Send + Sync>;

struct InputFakeMod {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
    recorded: Arc<Mutex<Vec<(String, Value)>>>,
}

impl InputFakeMod {
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
                    recorded_clone.lock().await.push((op.clone(), args.clone()));

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
        self.recorded.lock().await.clone()
    }

    async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

#[tokio::test]
async fn input_tools_are_disabled_by_default_and_exposed_only_when_enabled() {
    let temp = TestDir::new();

    // Default module: input disabled, 17 read-only tools, press/move absent & rejected.
    let readonly = module(temp.path());
    assert!(!readonly.input_enabled());
    let default_tools = readonly.list_tools().await.expect("list default tools");
    assert_eq!(default_tools.len(), 17);
    assert!(
        !default_tools
            .iter()
            .any(|t| t.name == "press" || t.name == "move")
    );

    for tool_name in ["press", "move"] {
        let err = readonly
            .call_tool(tool_name, json!({}), &CallContext::new(RunId::new()))
            .await
            .expect_err("disabled input tool must return ToolNotFound");
        assert!(matches!(err, GameError::ToolNotFound { name } if name == tool_name));
    }

    // Opt-in module: exposes press and move as ToolCategory::Action with side_effects = true.
    let armed = module(temp.path()).with_enable_input(true);
    assert!(armed.input_enabled());
    let armed_tools = armed.list_tools().await.expect("list armed tools");
    assert_eq!(armed_tools.len(), 19);

    let press_tool = armed_tools
        .iter()
        .find(|t| t.name == "press")
        .expect("press descriptor");
    assert_eq!(press_tool.category, ToolCategory::Action);
    assert!(press_tool.side_effects);

    let move_tool = armed_tools
        .iter()
        .find(|t| t.name == "move")
        .expect("move descriptor");
    assert_eq!(move_tool.category, ToolCategory::Action);
    assert!(move_tool.side_effects);
}

#[tokio::test]
async fn session_lifecycle_manages_input_enabled_switch_file() {
    let temp = TestDir::new();
    let bridge_dir = temp.path().join("bridge");
    let switch_path = bridge_dir.join(INPUT_SWITCH_FILE);

    // 1. Enabled module creates input.enabled in start_session and removes it in end_session.
    let armed = module(&bridge_dir).with_enable_input(true);
    armed
        .start_session(SessionStart {
            run_id: RunId::new(),
            start_save: None,
        })
        .await
        .expect("start armed session");
    assert!(switch_path.is_file(), "input.enabled must exist when armed");

    armed.end_session().await.expect("end armed session");
    assert!(
        !switch_path.exists(),
        "input.enabled must be deleted on end_session"
    );

    // 2. Default (disabled) module cleans up any stale input.enabled on start_session.
    std::fs::write(&switch_path, b"stale\n").expect("plant stale switch");
    assert!(switch_path.is_file());

    let readonly = module(&bridge_dir);
    readonly
        .start_session(SessionStart {
            run_id: RunId::new(),
            start_save: None,
        })
        .await
        .expect("start readonly session");
    assert!(
        !switch_path.exists(),
        "readonly start_session must delete stale input.enabled"
    );
}

#[tokio::test]
async fn press_and_move_forward_to_bridge_validate_args_and_propagate_mod_errors() {
    let temp = TestDir::new();
    let switch_path = temp.path().join(INPUT_SWITCH_FILE);

    let switch_for_mod = switch_path.clone();
    let handler: AckHandler = Arc::new(move |op, args| {
        if op != "press" {
            return (false, Value::Null, Some(format!("未知命令 {op}")));
        }
        if !switch_for_mod.is_file() {
            return (
                false,
                Value::Null,
                Some("输入未开启：桥目录缺少 input.enabled 开关文件".to_owned()),
            );
        }
        let action = args["action"].as_str().unwrap_or("");
        if action == "move" {
            (
                true,
                json!({
                    "action": "move",
                    "times": 1,
                    "hold_ms": args["ms"],
                    "x": args["x"],
                    "y": args["y"],
                    "ms": args["ms"],
                    "injected": {
                        "method": "StartContinuousInputInjectionForAction",
                        "input_action": "IA_Map_UnitMove",
                    },
                    "before": { "screen_kind": "field", "cursor": null, "input_blocked": ["WindowDeactivate"] },
                    "after": { "screen_kind": "field", "cursor": null, "input_blocked": ["WindowDeactivate"] },
                    "window_deactivated": true,
                }),
                None,
            )
        } else {
            (
                true,
                json!({
                    "action": action,
                    "times": args["times"],
                    "hold_ms": args["hold_ms"],
                    "injected": {
                        "method": "InjectInputForAction",
                        "input_action": "IA_UI_Common_Direction",
                    },
                    "before": { "screen_kind": "system_menu", "cursor": 0, "input_blocked": [] },
                    "after": { "screen_kind": "system_menu", "cursor": 2, "input_blocked": [] },
                    "window_deactivated": false,
                }),
                None,
            )
        }
    });

    let fake_mod = InputFakeMod::start(temp.path(), handler).await;
    let armed = module(temp.path()).with_enable_input(true);

    // 1. Without input.enabled on disk, mod rejects with "输入未开启".
    let disabled_out = armed
        .call_tool(
            "press",
            json!({ "action": "confirm" }),
            &CallContext::new(RunId::new()),
        )
        .await
        .expect("tool output");
    assert!(disabled_out.is_error);
    let Content::Text { text: err_text } = &disabled_out.content[0] else {
        panic!("expected text error");
    };
    assert!(err_text.contains("输入未开启"), "{err_text}");

    // 2. Start session -> creates input.enabled, then press & move succeed.
    armed
        .start_session(SessionStart {
            run_id: RunId::new(),
            start_save: None,
        })
        .await
        .expect("start session");

    let (press_text, press_json) =
        call_data_ok(&armed, "press", json!({ "action": "down", "times": 2 })).await;
    assert!(press_text.contains("`down`"), "{press_text}");
    assert!(press_text.contains("2 次"), "{press_text}");
    assert!(
        press_text.contains("system_menu（光标=0） → system_menu（光标=2）"),
        "{press_text}"
    );
    assert_eq!(press_json["ok"], true);
    assert_eq!(press_json["after"]["cursor"], 2);
    assert_eq!(press_json["window_deactivated"], false);

    let (move_text, move_json) = call_data_ok(
        &armed,
        "move",
        json!({ "direction": "up_right", "ms": 400 }),
    )
    .await;
    assert!(move_text.contains("右上"), "{move_text}");
    assert!(move_text.contains("400 ms"), "{move_text}");
    assert!(move_text.contains("WindowDeactivate"), "{move_text}");
    assert_eq!(move_json["ok"], true);
    assert_eq!(move_json["direction"], "up_right");
    assert_eq!(move_json["ms"], 400);
    assert_eq!(move_json["window_deactivated"], true);

    // 3. Invalid arguments are rejected locally without sending command.json.
    for (tool, bad_args, needle) in [
        ("press", json!({ "action": "move" }), "不支持动作"),
        ("press", json!({ "action": "cheat" }), "不支持动作"),
        (
            "press",
            json!({ "action": "confirm", "times": 0 }),
            "1..=20",
        ),
        (
            "press",
            json!({ "action": "confirm", "extra": 1 }),
            "未知参数",
        ),
        (
            "move",
            json!({ "direction": "jump", "ms": 200 }),
            "不支持方向",
        ),
        ("move", json!({ "direction": "up", "ms": 0 }), "1..=5000"),
        ("move", json!({ "direction": "up", "ms": 6000 }), "1..=5000"),
    ] {
        let out = armed
            .call_tool(tool, bad_args, &CallContext::new(RunId::new()))
            .await
            .expect("arg validation returns ToolOutput");
        assert!(out.is_error, "{tool} should reject invalid args");
        let Content::Text { text } = &out.content[0] else {
            panic!("expected text error");
        };
        assert!(text.contains(needle), "expected `{needle}` in `{text}`");
    }

    // Verify exact commands forwarded to the bridge (only the 3 valid calls).
    let recorded = fake_mod.recorded().await;
    assert_eq!(recorded.len(), 3);
    assert_eq!(
        recorded[0],
        (
            "press".to_owned(),
            json!({ "action": "confirm", "times": 1, "hold_ms": 0 })
        )
    );
    assert_eq!(
        recorded[1],
        (
            "press".to_owned(),
            json!({ "action": "down", "times": 2, "hold_ms": 0 })
        )
    );
    assert_eq!(
        recorded[2],
        (
            "press".to_owned(),
            json!({
                "action": "move",
                "x": std::f64::consts::FRAC_1_SQRT_2,
                "y": std::f64::consts::FRAC_1_SQRT_2,
                "ms": 400
            })
        )
    );

    armed.end_session().await.expect("end session");
    fake_mod.stop().await;
}

#[test]
fn cli_enable_input_flag_gates_tools_and_manages_switch_file_lifecycle() {
    let temp = TestDir::new();
    let bridge_dir = temp.path().join("bridge");
    std::fs::create_dir_all(&bridge_dir).expect("create bridge dir");
    let switch_path = bridge_dir.join(INPUT_SWITCH_FILE);

    // Helper to run one interactive request against a spawned child and inspect disk state while running.
    let send_one = |stdin: &mut std::process::ChildStdin,
                    reader: &mut BufReader<std::process::ChildStdout>,
                    req: Value|
     -> Value {
        writeln!(stdin, "{req}").expect("write req");
        stdin.flush().expect("flush req");
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).expect("read resp");
        assert_ne!(bytes, 0, "ffr-module closed stdout unexpectedly");
        serde_json::from_str(&line).expect("parse resp")
    };

    // 1. Default CLI (without --enable-input): deletes pre-existing stale input.enabled, lists 17 tools.
    std::fs::write(&switch_path, b"stale\n").expect("plant stale switch");
    let mut child = Command::new(env!("CARGO_BIN_EXE_ffr-module"))
        .args(["--bridge-dir", bridge_dir.to_str().expect("utf8 path")])
        .env_remove("FFR_GAME_DIR")
        .env_remove("FFR_DEPS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn readonly ffr-module");

    {
        let mut stdin = child.stdin.take().expect("stdin");
        let mut reader = BufReader::new(child.stdout.take().expect("stdout"));
        let _ = send_one(
            &mut stdin,
            &mut reader,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocol_version": PROTOCOL_VERSION,
                    "client_name": "input-cli-test",
                    "client_version": "1",
                }
            }),
        );
        assert!(
            !switch_path.exists(),
            "startup without --enable-input must remove stale input.enabled"
        );
        let tools_resp = send_one(
            &mut stdin,
            &mut reader,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
        );
        let tools = tools_resp["result"]["tools"]
            .as_array()
            .expect("tools array");
        assert_eq!(tools.len(), 17);
        assert!(
            !tools
                .iter()
                .any(|t| t["name"] == "press" || t["name"] == "move")
        );
    }
    let status = child.wait().expect("wait readonly child");
    assert!(status.success());

    // 2. Opt-in CLI (--enable-input): creates input.enabled while running, exposes press/move, cleans up on exit.
    let mut armed_child = Command::new(env!("CARGO_BIN_EXE_ffr-module"))
        .args([
            "--bridge-dir",
            bridge_dir.to_str().expect("utf8 path"),
            "--enable-input",
        ])
        .env_remove("FFR_GAME_DIR")
        .env_remove("FFR_DEPS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn armed ffr-module");

    {
        let mut stdin = armed_child.stdin.take().expect("stdin");
        let mut reader = BufReader::new(armed_child.stdout.take().expect("stdout"));
        let _ = send_one(
            &mut stdin,
            &mut reader,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocol_version": PROTOCOL_VERSION,
                    "client_name": "input-cli-test",
                    "client_version": "1",
                }
            }),
        );
        assert!(
            switch_path.is_file(),
            "startup with --enable-input must create input.enabled"
        );
        let tools_resp = send_one(
            &mut stdin,
            &mut reader,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
        );
        let tools = tools_resp["result"]["tools"]
            .as_array()
            .expect("tools array");
        assert_eq!(tools.len(), 19);
        let press = tools
            .iter()
            .find(|t| t["name"] == "press")
            .expect("press tool listed");
        assert_eq!(press["category"], "action");
        assert_eq!(press["side_effects"], true);
        let mv = tools
            .iter()
            .find(|t| t["name"] == "move")
            .expect("move tool listed");
        assert_eq!(mv["category"], "action");
        assert_eq!(mv["side_effects"], true);
    }
    let armed_status = armed_child.wait().expect("wait armed child");
    assert!(armed_status.success());
    assert!(
        !switch_path.exists(),
        "exiting --enable-input process must clean up input.enabled"
    );
}
