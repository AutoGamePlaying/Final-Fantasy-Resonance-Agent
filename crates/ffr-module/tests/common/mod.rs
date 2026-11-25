use std::fs as std_fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use agp_core::RunId;
use agp_game::{CallContext, Content, GameModule, ToolOutput};
use agp_platform::FileChannelOptions;
use ffr_module::FfrModule;
use serde_json::{Map, Value, json};
use tokio::fs;
use tokio::sync::watch;
use tokio::task::JoinHandle;

static NEXT_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

pub struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub fn new() -> Self {
        let id = NEXT_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("ffr-module-tests/{}/{id}", std::process::id()));
        std_fs::create_dir_all(&path).expect("create test directory");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std_fs::remove_dir_all(&self.path);
    }
}

#[allow(dead_code)]
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/ffr-module sits two levels below repository root")
        .to_path_buf()
}

#[allow(dead_code)]
pub fn fixture_data_export() -> PathBuf {
    repo_root().join("test/fixtures/data/export")
}

#[allow(dead_code)]
pub fn fixture_battle_export() -> PathBuf {
    repo_root().join("test/fixtures/battle")
}

#[allow(dead_code)]
pub fn fixture_world_export() -> PathBuf {
    repo_root().join("test/fixtures/world/export")
}

#[allow(dead_code)]
pub fn fixture_gaps_export() -> PathBuf {
    repo_root().join("test/fixtures/gaps/export")
}

#[allow(dead_code)]
pub fn fixture_bridge_v1_dir() -> PathBuf {
    repo_root().join("test/fixtures/bridge-v1")
}

#[allow(dead_code)]
pub fn load_bridge_v1_fixture(name: &str) -> Value {
    let path = fixture_bridge_v1_dir().join(name);
    let raw = std_fs::read_to_string(&path).expect("read bridge v1 fixture");
    serde_json::from_str(&raw).expect("parse bridge v1 fixture")
}

#[allow(dead_code)]
pub fn build_merged_fixture_export(dst: &Path) {
    let sources = [
        fixture_world_export(),
        fixture_battle_export(),
        fixture_gaps_export(),
    ];
    for src in &sources {
        copy_dir_recursive(src, dst);
    }

    // Merge Localization JSON catalogs from world + battle + gaps fixtures so all namespaces coexist.
    for lang in ["zh-Hans", "en", "ja"] {
        let mut merged = Map::new();
        for src in &sources {
            let path = src.join(format!("Localization/{lang}.json"));
            if !path.is_file() {
                continue;
            }
            let raw = std_fs::read_to_string(&path).expect("read fixture localization");
            let parsed: Value = serde_json::from_str(&raw).expect("parse fixture localization");
            for (ns, entries) in parsed.as_object().expect("localization root object") {
                merged.insert(ns.clone(), entries.clone());
            }
        }
        let target = dst.join(format!("Localization/{lang}.json"));
        std_fs::write(
            target,
            serde_json::to_vec_pretty(&Value::Object(merged)).expect("encode merged localization"),
        )
        .expect("write merged localization");
    }
}

#[allow(dead_code)]
pub fn copy_dir_recursive(src: &Path, dst: &Path) {
    std_fs::create_dir_all(dst).expect("create destination dir");
    for entry in std_fs::read_dir(src).expect("read source dir") {
        let entry = entry.expect("dir entry");
        let target = dst.join(entry.file_name());
        if entry.file_type().expect("entry file type").is_dir() {
            copy_dir_recursive(&entry.path(), &target);
        } else {
            std_fs::copy(entry.path(), target).expect("copy fixture file");
        }
    }
}

pub fn options() -> FileChannelOptions {
    FileChannelOptions {
        stale_after: Duration::from_millis(100),
        read_retry_for: Duration::from_millis(200),
        retry_interval: Duration::from_millis(5),
        ack_timeout: Duration::from_millis(100),
    }
}

pub fn module(dir: &Path) -> FfrModule {
    FfrModule::with_channel_options(dir, "FFRS-Win64-Shipping.exe", options())
}

#[allow(dead_code)]
pub fn module_with_data(bridge_dir: &Path, data_dir: impl Into<PathBuf>) -> FfrModule {
    module(bridge_dir).with_export_dir(data_dir)
}

#[allow(dead_code)]
pub fn module_with_db(bridge_dir: &Path, db_path: impl Into<PathBuf>) -> FfrModule {
    module(bridge_dir).with_db(db_path)
}

#[allow(dead_code)]
pub fn build_fixture_db_from_export(export_dir: &Path, db_path: &Path) {
    let options = ffr_db::BuildOptions {
        inputs: ffr_db::BuildInputs {
            export: Some(export_dir.to_path_buf()),
            ..Default::default()
        },
        output: db_path.to_path_buf(),
        stages: Some(vec![
            ffr_db::Stage::Text,
            ffr_db::Stage::Battle,
            ffr_db::Stage::World,
        ]),
        game_build_id: None,
    };
    ffr_db::build(&options).expect("build fixture ffr-db sqlite");
}

#[allow(dead_code)]
pub async fn read_all_resource_text(module: &FfrModule, uri: &str) -> String {
    use agp_game::ResourceRange;

    let mut out = String::new();
    let mut offset = 0_u64;
    loop {
        let page = module
            .read_resource(
                uri,
                ResourceRange {
                    offset,
                    length: 64 * 1024,
                },
            )
            .await
            .expect("read resource page");
        let Content::Text { text } = page.content else {
            panic!("resource page content must be Text");
        };
        out.push_str(&text);
        match page.next_offset {
            Some(next) => offset = next,
            None => break,
        }
    }
    out
}

#[allow(dead_code)]
pub async fn call_json(module: &FfrModule, name: &str) -> (ToolOutput, Value) {
    let output = module
        .call_tool(name, json!({}), &CallContext::new(RunId::new()))
        .await
        .expect("tool call succeeds");
    let value = match output.content.as_slice() {
        [Content::Json { value }] => value.clone(),
        _ => panic!("tool must return one JSON content item"),
    };
    (output, value)
}

#[allow(dead_code)]
pub async fn call_data_ok(module: &FfrModule, name: &str, args: Value) -> (String, Value) {
    let output = module
        .call_tool(name, args, &CallContext::new(RunId::new()))
        .await
        .expect("tool call succeeds");
    assert!(
        !output.is_error,
        "expected {name} to succeed, got error: {:?}",
        output.content
    );
    match output.content.as_slice() {
        [Content::Text { text }, Content::Json { value }] => (text.clone(), value.clone()),
        other => panic!("data tool `{name}` must return [Text, Json], got {other:?}"),
    }
}

#[allow(dead_code)]
pub async fn call_data_err(module: &FfrModule, name: &str, args: Value) -> String {
    let output = module
        .call_tool(name, args, &CallContext::new(RunId::new()))
        .await
        .expect("tool error is returned as ToolOutput");
    assert!(
        output.is_error,
        "expected {name} to fail, got ok: {:?}",
        output.content
    );
    match output.content.as_slice() {
        [Content::Text { text }] => text.clone(),
        other => panic!("error output for `{name}` must be one Text item, got {other:?}"),
    }
}

#[allow(dead_code)]
pub async fn write_state(dir: &Path, session: &str, seq: u64) {
    write_json_atomic(
        &dir.join("state.json"),
        &json!({
            "v": 1,
            "session": session,
            "seq": seq,
            "clock_ms": seq * 500,
            "data": {
                "world": "ExampleWorld",
                "map": "ExampleMap",
                "player_controller_class": "ExampleController",
                "pawn_class": "ExamplePawn",
                "game_mode_class": null,
                "game_instance_class": null,
                "pawn_location": {"x": 1.0, "y": 2.0, "z": 3.0},
                "errors": [],
                "bridge_version": "0",
            },
        }),
    )
    .await;
}

#[allow(dead_code)]
pub async fn write_json_atomic(path: &Path, value: &Value) {
    let temporary_path = path.with_extension("tmp");
    let bytes = serde_json::to_vec(value).expect("serialize test JSON");
    fs::write(&temporary_path, bytes)
        .await
        .expect("write temporary JSON");
    match fs::remove_file(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("remove old JSON: {error}"),
    }
    fs::rename(temporary_path, path)
        .await
        .expect("rename temporary JSON");
}

#[allow(dead_code)]
pub struct FakeMod {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

#[allow(dead_code)]
impl FakeMod {
    pub async fn start(dir: &Path) -> Self {
        write_state(dir, "fake-session", 1).await;
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(fake_mod(dir.to_owned(), stopped));
        Self { stop, task }
    }

    pub async fn stop(self) {
        self.stop.send(true).expect("stop fake mod");
        self.task.await.expect("join fake mod");
    }
}

#[allow(dead_code)]
async fn fake_mod(dir: PathBuf, mut stopped: watch::Receiver<bool>) {
    let mut seq = 1;
    loop {
        seq += 1;
        write_state(&dir, "fake-session", seq).await;

        if let Ok(bytes) = fs::read(dir.join("command.json")).await
            && let Ok(command) = serde_json::from_slice::<Value>(&bytes)
        {
            let id = command["id"].clone();
            let op = command["op"].as_str().unwrap_or_default();
            let ack = if op == "ping" {
                json!({
                    "v": 1,
                    "id": id,
                    "ok": true,
                    "result": {"pong": true, "seq": seq},
                    "error": null,
                })
            } else {
                json!({
                    "v": 1,
                    "id": id,
                    "ok": false,
                    "result": null,
                    "error": format!("不认识的命令 {op}"),
                })
            };
            write_json_atomic(&dir.join("ack.json"), &ack).await;
            match fs::remove_file(dir.join("command.json")).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("fake mod removes command: {error}"),
            }
        }

        tokio::select! {
            result = stopped.changed() => {
                if result.is_err() || *stopped.borrow() {
                    return;
                }
            }
            () = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
    }
}
