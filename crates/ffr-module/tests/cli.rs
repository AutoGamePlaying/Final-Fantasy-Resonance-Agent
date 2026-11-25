mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

use agp_core::RunId;
use agp_game::PROTOCOL_VERSION;
use common::{TestDir, copy_dir_recursive, fixture_data_export};
use serde_json::{Value, json};

fn exchange(mut child: Child, requests: &[Value]) -> Vec<Value> {
    let mut stdin = child.stdin.take().expect("child stdin");
    let stdout = child.stdout.take().expect("child stdout");
    let mut reader = BufReader::new(stdout);
    let mut responses = Vec::with_capacity(requests.len());

    for req in requests {
        writeln!(stdin, "{req}").expect("write request");
        stdin.flush().expect("flush request");

        let mut line = String::new();
        let bytes = reader.read_line(&mut line).expect("read response");
        assert_ne!(bytes, 0, "ffr-module closed stdout before responding");
        responses.push(serde_json::from_str(&line).expect("response line is JSON"));
    }

    drop(stdin);
    let output = child.wait_with_output().expect("wait for ffr-module");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    responses
}

#[test]
fn startup_fails_in_chinese_when_no_bridge_directory_can_be_determined() {
    let output = Command::new(env!("CARGO_BIN_EXE_ffr-module"))
        .env_remove("FFR_GAME_DIR")
        .env_remove("FFR_DEPS")
        .output()
        .expect("run ffr-module without arguments");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "stdout is protocol-only");
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("没有指定桥目录"), "{stderr}");
    assert!(stderr.contains("--bridge-dir"), "{stderr}");
}

#[test]
fn startup_succeeds_when_data_dir_is_missing_and_status_tools_still_work() {
    let temp = TestDir::new();
    let missing_data = temp.path().join("does-not-exist");

    let child = Command::new(env!("CARGO_BIN_EXE_ffr-module"))
        .args([
            "--bridge-dir",
            temp.path().to_str().expect("utf8 path"),
            "--data-dir",
            missing_data.to_str().expect("utf8 path"),
        ])
        .env_remove("FFR_GAME_DIR")
        .env_remove("FFR_DEPS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ffr-module with missing data dir");

    let responses = exchange(
        child,
        &[
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocol_version": PROTOCOL_VERSION,
                    "client_name": "cli-test",
                    "client_version": "1",
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "game_status",
                    "args": {},
                    "run_id": RunId::new(),
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "read_chapter",
                    "args": {},
                    "run_id": RunId::new(),
                }
            }),
        ],
    );

    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["result"]["info"]["toolset_version"], "ffr-2");
    assert_eq!(responses[1]["result"]["output"]["is_error"], false);
    assert_eq!(responses[2]["result"]["output"]["is_error"], true);
    let err_text = responses[2]["result"]["output"]["content"][0]["text"]
        .as_str()
        .expect("error text");
    assert!(err_text.contains("不存在"), "{err_text}");
}

#[test]
fn data_dir_defaults_to_ffr_deps_export_directory() {
    let temp = TestDir::new();
    let bridge_dir = temp.path().join("bridge");
    let deps_dir = temp.path().join("deps");
    let export_dir = deps_dir.join("ffr/export");
    copy_dir_recursive(&fixture_data_export(), &export_dir);

    let child = Command::new(env!("CARGO_BIN_EXE_ffr-module"))
        .args(["--bridge-dir", bridge_dir.to_str().expect("utf8 path")])
        .env_remove("FFR_GAME_DIR")
        .env("FFR_DEPS", &deps_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ffr-module using FFR_DEPS");

    let responses = exchange(
        child,
        &[
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocol_version": PROTOCOL_VERSION,
                    "client_name": "cli-test",
                    "client_version": "1",
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "read_chapter",
                    "args": { "chapter": 1 },
                    "run_id": RunId::new(),
                }
            }),
        ],
    );

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[1]["result"]["output"]["is_error"], false);
    assert_eq!(
        responses[1]["result"]["output"]["content"][1]["value"]["name"],
        "起始之森"
    );
}

#[test]
fn cli_flag_controls_reveal_hidden_weaknesses_in_read_game_state() {
    let temp = TestDir::new();
    let bridge_dir = temp.path().join("bridge");
    let export_dir = temp.path().join("export");
    std::fs::create_dir_all(&bridge_dir).expect("create bridge dir");
    common::build_merged_fixture_export(&export_dir);

    let fixture = common::load_bridge_v1_fixture("battle-cmd.json");
    std::fs::write(
        bridge_dir.join("state.json"),
        serde_json::to_vec(&fixture).expect("encode battle fixture"),
    )
    .expect("write state.json");

    let run_once = |extra_args: &[&str]| {
        let mut args = vec![
            "--bridge-dir",
            bridge_dir.to_str().expect("utf8 bridge path"),
            "--data-dir",
            export_dir.to_str().expect("utf8 export path"),
        ];
        args.extend_from_slice(extra_args);
        let child = Command::new(env!("CARGO_BIN_EXE_ffr-module"))
            .args(args)
            .env_remove("FFR_GAME_DIR")
            .env_remove("FFR_DEPS")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn ffr-module");
        exchange(
            child,
            &[
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocol_version": PROTOCOL_VERSION,
                        "client_name": "cli-test",
                        "client_version": "1",
                    }
                }),
                json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/call",
                    "params": {
                        "name": "read_game_state",
                        "args": {},
                        "run_id": RunId::new(),
                    }
                }),
            ],
        )
    };

    // Default (fair mode): hidden weaknesses are withheld.
    let fair = run_once(&[]);
    let fair_out = &fair[1]["result"]["output"];
    assert_eq!(fair_out["is_error"], false);
    assert_eq!(
        fair_out["content"][1]["value"]["reveal_hidden_weaknesses"],
        false
    );
    let fair_text = fair_out["content"][0]["text"].as_str().expect("text");
    assert!(fair_text.contains("光标在「招式」"), "{fair_text}");
    assert!(
        fair_text.contains("数据表另有 1 个未揭示弱点"),
        "{fair_text}"
    );
    let fair_enemy_section = fair_text
        .split_once("- 敌方：")
        .map(|(_, rest)| rest)
        .expect("enemy section in summary");
    assert!(!fair_enemy_section.contains("水"), "{fair_enemy_section}");
    assert!(!fair_enemy_section.contains("雷"), "{fair_enemy_section}");

    // Opt-in flag: hidden weaknesses are revealed.
    let revealed = run_once(&["--reveal-hidden-weaknesses"]);
    let rev_out = &revealed[1]["result"]["output"];
    assert_eq!(rev_out["is_error"], false);
    assert_eq!(
        rev_out["content"][1]["value"]["reveal_hidden_weaknesses"],
        true
    );
    let rev_text = rev_out["content"][0]["text"].as_str().expect("text");
    let rev_enemy_section = rev_text
        .split_once("- 敌方：")
        .map(|(_, rest)| rest)
        .expect("enemy section in revealed summary");
    assert!(rev_enemy_section.contains("水"), "{rev_enemy_section}");
    assert!(rev_enemy_section.contains("雷"), "{rev_enemy_section}");
}
