//! FINAL FANTASY RESONANCE game module for AGP.
//!
//! Exposes read-only process, window, and FfrBridge v1 live-state/command tools (`game_status`,
//! `bridge_ping`, `read_game_state`, `read_save_slots`, `read_inventory`, `read_event_flags`,
//! `read_input_state`, `query_nav_path`) alongside static game-data lookup tools (`read_*`,
//! `search_data`, `find_route`, `where_to_get`, `whats_near`, `query_game_db`) and paginated
//! read-only catalog/schema resources (`ffr://*`).

mod bridge_commands;
mod data_cache;
mod data_tools;
mod data_tools_db;
mod data_tools_ffr3;
mod input_tools;
mod resources;
mod state_tools;
mod text_format;

use std::path::{Path, PathBuf};
use std::time::Duration;

use agp_core::GameId;
use agp_game::{
    CallContext, Content, GameError, GameModule, ModuleInfo, ResourceDescriptor, ResourcePage,
    ResourceRange, Result, SessionReport, SessionStart, ToolCategory, ToolDescriptor, ToolOutput,
};
use agp_platform::{FileChannel, FileChannelOptions, PlatformError, process, window};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::data_cache::GameDataSource;
pub use crate::input_tools::{INPUT_SWITCH_FILE, remove_input_switch_blocking};

/// Stable toolset version identifier reported during `initialize`.
pub const TOOLSET_VERSION: &str = "ffr-3";

/// FFR AGP game module backed by the UE4SS file bridge and static game data (`ffr-db` SQLite
/// database by default, or legacy exported JSON directory). Read-only by default; opt-in input
/// injection tools (`press`, `move`) are exposed only when [`FfrModule::with_enable_input`] is
/// enabled.
#[derive(Debug)]
pub struct FfrModule {
    bridge_dir: PathBuf,
    channel: FileChannel,
    exe_name: String,
    data: GameDataSource,
    reveal_hidden_weaknesses: bool,
    enable_input: bool,
}

impl FfrModule {
    pub fn new(bridge_dir: impl Into<PathBuf>, exe_name: impl Into<String>) -> Self {
        Self::with_channel_options(bridge_dir, exe_name, FileChannelOptions::default())
    }

    pub fn with_channel_options(
        bridge_dir: impl Into<PathBuf>,
        exe_name: impl Into<String>,
        options: FileChannelOptions,
    ) -> Self {
        let bridge_dir = bridge_dir.into();
        Self {
            channel: FileChannel::new(bridge_dir.clone(), options),
            bridge_dir,
            exe_name: exe_name.into(),
            data: GameDataSource::db(None),
            reveal_hidden_weaknesses: false,
            enable_input: false,
        }
    }

    /// Attach an optional `ffr-db` SQLite database path for data tools, paginated resources, and
    /// live-state enrichment. Missing or incompatible databases never prevent module startup.
    pub fn with_db_path(mut self, db_path: Option<PathBuf>) -> Self {
        self.data = GameDataSource::db(db_path);
        self
    }

    /// Attach an explicit `ffr-db` SQLite database file path.
    pub fn with_db(self, db_path: impl Into<PathBuf>) -> Self {
        self.with_db_path(Some(db_path.into()))
    }

    /// Attach an optional CUE4Parse export directory (`<deps>/ffr/export`) using the legacy JSON
    /// reference backend. Missing directories never prevent module startup.
    pub fn with_data_dir(mut self, data_dir: Option<PathBuf>) -> Self {
        self.data = GameDataSource::json(data_dir);
        self
    }

    /// Attach an explicit CUE4Parse export directory (legacy JSON reference backend).
    pub fn with_export_dir(self, data_dir: impl Into<PathBuf>) -> Self {
        self.with_data_dir(Some(data_dir.into()))
    }

    /// Configure whether `read_game_state` may reveal enemy weaknesses/resistances that have not
    /// yet been analyzed in-game. Defaults to `false` for fair live play (see `docs/fairness.md`).
    pub fn with_reveal_hidden_weaknesses(mut self, reveal: bool) -> Self {
        self.reveal_hidden_weaknesses = reveal;
        self
    }

    /// Returns whether hidden battle weaknesses are currently disclosed in `read_game_state`.
    pub fn reveal_hidden_weaknesses(&self) -> bool {
        self.reveal_hidden_weaknesses
    }

    /// Opt in to exposing input action tools (`press`, `move`) and managing `<bridge_dir>/input.enabled`.
    /// Defaults to `false`.
    pub fn with_enable_input(mut self, enable: bool) -> Self {
        self.enable_input = enable;
        self
    }

    /// Returns whether simulated input tools are currently enabled.
    pub fn input_enabled(&self) -> bool {
        self.enable_input
    }

    /// Returns the bridge directory path used for state/command files and `input.enabled`.
    pub fn bridge_dir(&self) -> &Path {
        &self.bridge_dir
    }

    /// Create or remove `<bridge_dir>/input.enabled` to match `self.enable_input`.
    pub async fn sync_input_switch(&self) -> std::io::Result<()> {
        input_tools::sync_input_switch(&self.bridge_dir, self.enable_input).await
    }

    /// Remove `<bridge_dir>/input.enabled` if present.
    pub async fn remove_input_switch(&self) -> std::io::Result<()> {
        input_tools::remove_input_switch(&self.bridge_dir).await
    }

    async fn game_status(&self) -> ToolOutput {
        let (process, window) = query_platform(&self.exe_name);
        let bridge = match self.channel.read_fresh().await {
            Ok(snapshot) => json!({
                "ok": true,
                "session": snapshot.session,
                "seq": snapshot.seq,
                "file_age_ms": snapshot.file_age.map(duration_millis),
                "data": snapshot.data,
            }),
            Err(error) => json!({
                "ok": false,
                "error": error.to_string(),
            }),
        };

        ToolOutput::new(vec![Content::json(json!({
            "process": process,
            "window": window,
            "bridge": bridge,
        }))])
    }

    async fn bridge_ping(&self, ctx: &CallContext) -> ToolOutput {
        let started = tokio::time::Instant::now();
        match self.channel.ping(&ctx.cancellation).await {
            Ok(()) => ToolOutput::new(vec![Content::json(json!({
                "ok": true,
                "round_trip_ms": duration_millis(started.elapsed()),
            }))]),
            Err(error) => {
                let mut output = ToolOutput::new(vec![Content::json(json!({
                    "ok": false,
                    "error": format!("桥 ping 失败：{error}"),
                }))]);
                output.is_error = true;
                output
            }
        }
    }
}

#[async_trait]
impl GameModule for FfrModule {
    fn info(&self) -> ModuleInfo {
        ModuleInfo {
            // "ffr" is a static, non-empty identifier.
            game_id: GameId::new("ffr").expect("ffr is a valid game id"),
            module_version: env!("CARGO_PKG_VERSION").to_owned(),
            toolset_version: TOOLSET_VERSION.to_owned(),
        }
    }

    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>> {
        let empty_params = json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false,
        });
        let mut tools = vec![
            ToolDescriptor {
                name: "game_status".to_owned(),
                description: "读取游戏进程、主窗口和文件桥的最新状态；桥状态中的 data 原样返回。"
                    .to_owned(),
                parameters: empty_params.clone(),
                category: ToolCategory::Status,
                side_effects: false,
                timeout_ms: None,
            },
            ToolDescriptor {
                name: "bridge_ping".to_owned(),
                description: "向游戏内文件桥发送一次 ping，检查模组是否在线并返回往返毫秒数。"
                    .to_owned(),
                parameters: empty_params,
                category: ToolCategory::Status,
                side_effects: false,
                timeout_ms: None,
            },
            state_tools::read_game_state_descriptor(),
        ];
        tools.extend(bridge_commands::bridge_command_tool_descriptors());
        tools.extend(data_tools::data_tool_descriptors());
        if self.enable_input {
            tools.extend(input_tools::input_tool_descriptors());
        }
        Ok(tools)
    }

    async fn call_tool(&self, name: &str, args: Value, ctx: &CallContext) -> Result<ToolOutput> {
        match name {
            "game_status" | "bridge_ping" => {
                if !args.as_object().is_some_and(|args| args.is_empty()) {
                    let mut output = ToolOutput::new(vec![Content::text(format!(
                        "工具 `{name}` 不接受参数，请传入空对象 {{}}"
                    ))]);
                    output.is_error = true;
                    return Ok(output);
                }

                if name == "game_status" {
                    Ok(self.game_status().await)
                } else {
                    Ok(self.bridge_ping(ctx).await)
                }
            }
            state_tools::READ_GAME_STATE_TOOL => Ok(state_tools::handle_read_game_state(
                &self.channel,
                &self.data,
                self.reveal_hidden_weaknesses,
                args,
            )
            .await),
            other if bridge_commands::is_bridge_command_tool(other) => Ok(
                bridge_commands::call_bridge_command(&self.channel, &self.data, other, args, ctx)
                    .await,
            ),
            other if data_tools::is_data_tool(other) => {
                Ok(data_tools::call_data_tool(&self.channel, &self.data, other, args).await)
            }
            other if self.enable_input && input_tools::is_input_tool(other) => {
                Ok(input_tools::call_input_tool(&self.channel, other, args, ctx).await)
            }
            _ => Err(GameError::ToolNotFound {
                name: name.to_owned(),
            }),
        }
    }

    async fn list_resources(&self) -> Result<Vec<ResourceDescriptor>> {
        Ok(resources::list_resources(&self.data))
    }

    async fn read_resource(&self, uri: &str, range: ResourceRange) -> Result<ResourcePage> {
        resources::read_resource(&self.data, uri, range)
    }

    async fn start_session(&self, _session: SessionStart) -> Result<()> {
        self.channel
            .ensure_dir()
            .await
            .map_err(|error| GameError::InvalidMessage {
                operation: "创建桥目录".to_owned(),
                message: error.to_string(),
            })?;
        self.sync_input_switch()
            .await
            .map_err(|error| GameError::InvalidMessage {
                operation: "同步输入开关文件".to_owned(),
                message: format!("操作 {} 失败：{error}", INPUT_SWITCH_FILE),
            })
    }

    /// Always remove `input.enabled` when a session ends so simulated input never stays armed.
    async fn end_session(&self) -> Result<Option<SessionReport>> {
        self.remove_input_switch()
            .await
            .map_err(|error| GameError::InvalidMessage {
                operation: "清理输入开关文件".to_owned(),
                message: format!("删除 {} 失败：{error}", INPUT_SWITCH_FILE),
            })?;
        Ok(None)
    }
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn query_platform(exe_name: &str) -> (Value, Value) {
    let processes = match process::find_processes(exe_name) {
        Ok(processes) => processes,
        Err(PlatformError::Unsupported) => {
            return (
                json!({"running": null, "pid": null, "error": "仅 Windows 可读"}),
                json!({
                    "client_width": null,
                    "client_height": null,
                    "minimized": null,
                    "foreground": null,
                    "error": "仅 Windows 可读",
                }),
            );
        }
        Err(error) => {
            return (
                json!({
                    "running": null,
                    "pid": null,
                    "error": format!("读取游戏进程失败：{error}"),
                }),
                json!({"found": null, "error": "没有进程号，无法读取主窗口"}),
            );
        }
    };

    let pid = processes.first().map(|process| process.pid);
    let process_status = json!({
        "running": pid.is_some(),
        "pid": pid,
        "pids": processes.iter().map(|process| process.pid).collect::<Vec<_>>(),
    });
    (process_status, query_window(pid))
}

fn query_window(pid: Option<u32>) -> Value {
    let Some(pid) = pid else {
        return json!({"found": false});
    };

    match window::main_window(pid) {
        Ok(None) => json!({"found": false}),
        Ok(Some(window)) => {
            let (foreground, error) = match window::is_foreground(pid) {
                Ok(foreground) => (Some(foreground), None),
                Err(error) => (None, Some(format!("读取前台状态失败：{error}"))),
            };
            json!({
                "found": true,
                "client_width": window.client_width,
                "client_height": window.client_height,
                "minimized": window.minimized,
                "foreground": foreground,
                "error": error,
            })
        }
        Err(PlatformError::Unsupported) => json!({"found": null, "error": "仅 Windows 可读"}),
        Err(error) => json!({
            "found": null,
            "error": format!("读取主窗口失败：{error}"),
        }),
    }
}
