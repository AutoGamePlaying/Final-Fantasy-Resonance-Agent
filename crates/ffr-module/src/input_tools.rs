//! Opt-in R4 input injection tools (`ToolCategory::Action`, `side_effects: true`).
//!
//! Exposed by [`crate::FfrModule`] ONLY when `--enable-input` is explicitly set.
//! Manages the `<bridge_dir>/input.enabled` switch file and forwards `press` and `move`
//! tool calls to the in-game `FfrBridge` `press` command via [`FileChannel::send`].

use std::f64::consts::FRAC_1_SQRT_2;
use std::io::ErrorKind;
use std::path::Path;

use agp_game::{CallContext, Content, ToolCategory, ToolDescriptor, ToolOutput};
use agp_platform::FileChannel;
use serde_json::{Map, Value, json};
use tokio::fs;

use crate::state_tools::{obj_bool, obj_i64, obj_object, obj_str};

/// Switch file inside the bridge directory that unlocks FfrBridge's `press` command.
pub const INPUT_SWITCH_FILE: &str = "input.enabled";

/// Tool name for discrete semantic action presses.
pub const PRESS_TOOL: &str = "press";

/// Tool name for continuous field movement.
pub const MOVE_TOOL: &str = "move";

const MAX_PRESS_TIMES: i64 = 20;
const MAX_HOLD_MS: i64 = 5000;
const MAX_MOVE_MS: i64 = 5000;

/// Whitelisted semantic action names accepted by `press` (matches `topics/input.lua` discrete actions).
pub const PRESS_ACTIONS: [&str; 13] = [
    "confirm",
    "cancel",
    "up",
    "down",
    "left",
    "right",
    "page_prev",
    "page_next",
    "menu",
    "system_menu",
    "map",
    "quick_heal",
    "interact",
];

/// Supported 8-way movement directions mapped to normalized 2D stick vectors `(x, y)`.
/// Matches `BP_MapUnit_Player_C.m_OldInputActionValue` axis convention (up=(0,1), right=(1,0)).
const MOVE_DIRECTIONS: [(&str, f64, f64, &str); 8] = [
    ("up", 0.0, 1.0, "上/前"),
    ("down", 0.0, -1.0, "下/后"),
    ("left", -1.0, 0.0, "左"),
    ("right", 1.0, 0.0, "右"),
    ("up_left", -FRAC_1_SQRT_2, FRAC_1_SQRT_2, "左上"),
    ("up_right", FRAC_1_SQRT_2, FRAC_1_SQRT_2, "右上"),
    ("down_left", -FRAC_1_SQRT_2, -FRAC_1_SQRT_2, "左下"),
    ("down_right", FRAC_1_SQRT_2, -FRAC_1_SQRT_2, "右下"),
];

/// Returns `true` if `name` is one of the opt-in input action tools.
pub fn is_input_tool(name: &str) -> bool {
    matches!(name, PRESS_TOOL | MOVE_TOOL)
}

/// Build descriptors for the opt-in input tools (`press` and `move`).
pub fn input_tool_descriptors() -> Vec<ToolDescriptor> {
    let direction_names: Vec<&str> = MOVE_DIRECTIONS.iter().map(|(d, _, _, _)| *d).collect();
    vec![
        ToolDescriptor {
            name: PRESS_TOOL.to_owned(),
            description: "向游戏内 Enhanced Input 注入白名单语义按键（confirm/cancel/up/down/left/right/page_prev/page_next/menu/system_menu/map/quick_heal/interact），并返回注入前后的界面类型与光标位置；仅在 --enable-input 模式下可用。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": PRESS_ACTIONS,
                        "description": "白名单语义动作名。"
                    },
                    "times": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_PRESS_TIMES,
                        "description": "可选，连续按压次数（默认 1，最大 20）。"
                    },
                    "hold_ms": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": MAX_HOLD_MS,
                        "description": "可选，持续按住毫秒数（默认 0，最大 5000）。"
                    }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
            category: ToolCategory::Action,
            side_effects: true,
            timeout_ms: None,
        },
        ToolDescriptor {
            name: MOVE_TOOL.to_owned(),
            description: "在野外持续向指定方向注入移动输入（IA_Map_UnitMove）指定毫秒数后自动停止，返回注入前后的界面类型与光标状态；仅在 --enable-input 模式下可用。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "direction": {
                        "type": "string",
                        "enum": direction_names,
                        "description": "移动方向：up=前(0,1)、down=后(0,-1)、left=左(-1,0)、right=右(1,0) 及四个斜方向。"
                    },
                    "ms": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_MOVE_MS,
                        "description": "持续移动时长（毫秒，1..5000）。"
                    }
                },
                "required": ["direction", "ms"],
                "additionalProperties": false
            }),
            category: ToolCategory::Action,
            side_effects: true,
            timeout_ms: None,
        },
    ]
}

/// Synchronize `<bridge_dir>/input.enabled` with `enable_input`:
/// - when `true`, creates `bridge_dir` if needed and writes `input.enabled`;
/// - when `false`, removes `input.enabled` if it exists (without creating `bridge_dir`).
pub async fn sync_input_switch(bridge_dir: &Path, enable_input: bool) -> std::io::Result<()> {
    let switch_path = bridge_dir.join(INPUT_SWITCH_FILE);
    if enable_input {
        fs::create_dir_all(bridge_dir).await?;
        fs::write(&switch_path, b"enabled\n").await
    } else {
        remove_input_switch(bridge_dir).await
    }
}

/// Remove `<bridge_dir>/input.enabled` if it exists.
pub async fn remove_input_switch(bridge_dir: &Path) -> std::io::Result<()> {
    let switch_path = bridge_dir.join(INPUT_SWITCH_FILE);
    match fs::remove_file(&switch_path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Best-effort synchronous removal of `<bridge_dir>/input.enabled` for process shutdown guards.
pub fn remove_input_switch_blocking(bridge_dir: &Path) {
    let switch_path = bridge_dir.join(INPUT_SWITCH_FILE);
    let _ = std::fs::remove_file(switch_path);
}

/// Dispatch one opt-in input tool call (`press` or `move`).
pub async fn call_input_tool(
    channel: &FileChannel,
    name: &str,
    args: Value,
    ctx: &CallContext,
) -> ToolOutput {
    let Some(obj) = args.as_object() else {
        return arg_error(format!("工具 `{name}` 的参数必须是 JSON 对象"));
    };

    match name {
        PRESS_TOOL => handle_press(channel, obj, ctx).await,
        MOVE_TOOL => handle_move(channel, obj, ctx).await,
        _ => arg_error(format!("未知输入工具 `{name}`")),
    }
}

async fn handle_press(
    channel: &FileChannel,
    args: &Map<String, Value>,
    ctx: &CallContext,
) -> ToolOutput {
    if let Err(msg) = reject_unknown_keys(PRESS_TOOL, args, &["action", "times", "hold_ms"]) {
        return arg_error(msg);
    }

    let Some(action) = args.get("action").and_then(Value::as_str).map(str::trim) else {
        return arg_error("工具 `press` 必须提供字符串参数 `action`".to_owned());
    };
    if !PRESS_ACTIONS.contains(&action) {
        return arg_error(format!(
            "工具 `press` 不支持动作 `{action}`，允许值：{}",
            PRESS_ACTIONS.join("、")
        ));
    }

    let times = match parse_optional_i64_range(PRESS_TOOL, args, "times", 1, MAX_PRESS_TIMES) {
        Ok(Some(v)) => v,
        Ok(None) => 1,
        Err(msg) => return arg_error(msg),
    };
    let hold_ms = match parse_optional_i64_range(PRESS_TOOL, args, "hold_ms", 0, MAX_HOLD_MS) {
        Ok(Some(v)) => v,
        Ok(None) => 0,
        Err(msg) => return arg_error(msg),
    };

    let bridge_args = json!({
        "action": action,
        "times": times,
        "hold_ms": hold_ms,
    });
    let result = match send_press_command(channel, bridge_args, ctx).await {
        Ok(val) => val,
        Err(output) => return output,
    };

    let Some(res) = result.as_object() else {
        return bridge_format_error("回复 result 不是对象");
    };

    let (summary, payload) = format_press_result(
        format!("已注入按键 `{action}`（{times} 次，按住 {hold_ms} ms）"),
        res,
        None,
    );
    ToolOutput::new(vec![Content::text(summary), Content::json(payload)])
}

async fn handle_move(
    channel: &FileChannel,
    args: &Map<String, Value>,
    ctx: &CallContext,
) -> ToolOutput {
    if let Err(msg) = reject_unknown_keys(MOVE_TOOL, args, &["direction", "ms"]) {
        return arg_error(msg);
    }

    let Some(direction) = args.get("direction").and_then(Value::as_str).map(str::trim) else {
        return arg_error("工具 `move` 必须提供字符串参数 `direction`".to_owned());
    };
    let Some(&(_, x, y, dir_zh)) = MOVE_DIRECTIONS.iter().find(|(d, _, _, _)| *d == direction)
    else {
        let allowed: Vec<&str> = MOVE_DIRECTIONS.iter().map(|(d, _, _, _)| *d).collect();
        return arg_error(format!(
            "工具 `move` 不支持方向 `{direction}`，允许值：{}",
            allowed.join("、")
        ));
    };

    let Some(raw_ms) = args.get("ms") else {
        return arg_error("工具 `move` 必须提供整数参数 `ms`".to_owned());
    };
    let Some(ms) = raw_ms.as_i64() else {
        return arg_error("工具 `move` 的参数 `ms` 必须是整数".to_owned());
    };
    if !(1..=MAX_MOVE_MS).contains(&ms) {
        return arg_error(format!(
            "工具 `move` 的参数 `ms` 必须在 1..={MAX_MOVE_MS} 范围内"
        ));
    }

    let bridge_args = json!({
        "action": "move",
        "x": x,
        "y": y,
        "ms": ms,
    });
    let result = match send_press_command(channel, bridge_args, ctx).await {
        Ok(val) => val,
        Err(output) => return output,
    };

    let Some(res) = result.as_object() else {
        return bridge_format_error("回复 result 不是对象");
    };

    let extra = json!({
        "direction": direction,
        "direction_zh": dir_zh,
        "x": x,
        "y": y,
        "ms": ms,
    });
    let (summary, payload) = format_press_result(
        format!("已注入移动：方向 {dir_zh}（{direction}, x={x}, y={y}），持续 {ms} ms"),
        res,
        Some(extra),
    );
    ToolOutput::new(vec![Content::text(summary), Content::json(payload)])
}

fn format_press_result(
    headline: String,
    res: &Map<String, Value>,
    extra: Option<Value>,
) -> (String, Value) {
    let before = obj_object(res, "before");
    let after = obj_object(res, "after");
    let window_deactivated = obj_bool(res, "window_deactivated").unwrap_or(false);

    let before_desc = format_obs(before);
    let after_desc = format_obs(after);
    let method = obj_object(res, "injected")
        .and_then(|inj| obj_str(inj, "method"))
        .unwrap_or("未知");
    let ia = obj_object(res, "injected")
        .and_then(|inj| obj_str(inj, "input_action"))
        .unwrap_or("未知");

    let mut lines = vec![
        headline,
        format!("- 注入方式：{method}（{ia}）"),
        format!("- 画面与光标：{before_desc} → {after_desc}"),
    ];
    if window_deactivated {
        lines.push(
            "- 警告：检测到 WindowDeactivate（游戏窗口失焦），界面输入可能被游戏 InputBlockSet 拦截。"
                .to_owned(),
        );
    }

    let mut payload = json!({
        "ok": true,
        "action": res.get("action").cloned().unwrap_or(Value::Null),
        "times": res.get("times").cloned().unwrap_or(Value::Null),
        "hold_ms": res.get("hold_ms").cloned().unwrap_or(Value::Null),
        "injected": res.get("injected").cloned().unwrap_or(Value::Null),
        "before": res.get("before").cloned().unwrap_or(Value::Null),
        "after": res.get("after").cloned().unwrap_or(Value::Null),
        "window_deactivated": window_deactivated,
    });
    if let (Some(target), Some(Value::Object(ext))) = (payload.as_object_mut(), extra) {
        for (k, v) in ext {
            target.insert(k, v);
        }
    }

    (lines.join("\n"), payload)
}

fn format_obs(obs: Option<&Map<String, Value>>) -> String {
    let Some(obs) = obs else {
        return "未知".to_owned();
    };
    let screen = obj_str(obs, "screen_kind").unwrap_or("未知");
    match obj_i64(obs, "cursor") {
        Some(cursor) => format!("{screen}（光标={cursor}）"),
        None => format!("{screen}（光标=无）"),
    }
}

async fn send_press_command(
    channel: &FileChannel,
    args: Value,
    ctx: &CallContext,
) -> Result<Value, ToolOutput> {
    let ack = channel
        .send("press", args, &ctx.cancellation)
        .await
        .map_err(|error| {
            let mut output = ToolOutput::new(vec![
                Content::text(format!("执行桥命令 `press` 失败：{error}")),
                Content::json(json!({
                    "ok": false,
                    "op": "press",
                    "error": error.to_string(),
                })),
            ]);
            output.is_error = true;
            output
        })?;

    if !ack.ok {
        let reason = ack.error.unwrap_or_else(|| "模组未返回错误原因".to_owned());
        let mut output = ToolOutput::new(vec![
            Content::text(format!("游戏内模组拒绝执行 `press`：{reason}")),
            Content::json(json!({
                "ok": false,
                "op": "press",
                "error": reason,
            })),
        ]);
        output.is_error = true;
        return Err(output);
    }

    ack.result
        .ok_or_else(|| bridge_format_error("模组返回 ok=true 但缺少 result 字段"))
}

fn bridge_format_error(reason: &str) -> ToolOutput {
    let message = format!("桥命令 `press` 回复格式异常：{reason}");
    let mut output = ToolOutput::new(vec![
        Content::text(&message),
        Content::json(json!({
            "ok": false,
            "op": "press",
            "error": message,
        })),
    ]);
    output.is_error = true;
    output
}

fn arg_error(message: String) -> ToolOutput {
    let mut output = ToolOutput::new(vec![Content::text(message)]);
    output.is_error = true;
    output
}

fn reject_unknown_keys(
    tool: &str,
    args: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(), String> {
    let unknown: Vec<&str> = args
        .keys()
        .map(String::as_str)
        .filter(|key| !allowed.contains(key))
        .collect();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "工具 `{tool}` 收到未知参数：{}",
            unknown.join("、")
        ))
    }
}

fn parse_optional_i64_range(
    tool: &str,
    args: &Map<String, Value>,
    key: &str,
    min: i64,
    max: i64,
) -> Result<Option<i64>, String> {
    let Some(raw) = args.get(key) else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let Some(val) = raw.as_i64() else {
        return Err(format!("工具 `{tool}` 的参数 `{key}` 必须是整数"));
    };
    if !(min..=max).contains(&val) {
        return Err(format!(
            "工具 `{tool}` 的参数 `{key}` 必须在 {min}..={max} 范围内"
        ));
    }
    Ok(Some(val))
}
