//! Stdio entry point for the FFR external AGP game module.

use std::env;
use std::path::PathBuf;

use agp_game::serve_stdio;
use anyhow::{Context, anyhow};
use clap::Parser;
use ffr_module::{FfrModule, INPUT_SWITCH_FILE, remove_input_switch_blocking};

#[derive(Debug, Parser)]
#[command(name = "ffr-module", version, about = "FFR 的 AGP 外部游戏模块")]
struct Args {
    /// 游戏的 Win64 目录；也可用 FFR_GAME_DIR 环境变量
    #[arg(long, value_name = "Win64 目录")]
    game_dir: Option<PathBuf>,

    /// 文件桥目录；默认是 <game-dir>/ue4ss/ffr-out/bridge
    #[arg(long, value_name = "目录")]
    bridge_dir: Option<PathBuf>,

    /// 游戏数据导出目录；默认取环境变量 FFR_DEPS 下的 ffr/export
    #[arg(long, value_name = "导出目录")]
    data_dir: Option<PathBuf>,

    /// 游戏程序名
    #[arg(long, default_value = "FFRS-Win64-Shipping.exe")]
    exe: String,

    /// 战斗状态中是否揭示游戏内尚未揭露的敌人弱点与抗性（默认关闭，仅显示已揭示项）
    #[arg(long)]
    reveal_hidden_weaknesses: bool,

    /// 是否开启模拟输入工具（press / move）并在桥目录维护 input.enabled 开关文件（默认关闭）
    #[arg(long)]
    enable_input: bool,
}

struct InputSwitchGuard {
    bridge_dir: PathBuf,
    armed: bool,
}

impl Drop for InputSwitchGuard {
    fn drop(&mut self) {
        if self.armed {
            remove_input_switch_blocking(&self.bridge_dir);
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let game_dir = args
        .game_dir
        .or_else(|| env::var_os("FFR_GAME_DIR").map(PathBuf::from));
    let bridge_dir = match args.bridge_dir {
        Some(dir) => dir,
        None => game_dir
            .map(|dir| dir.join("ue4ss/ffr-out/bridge"))
            .ok_or_else(|| {
                anyhow!("没有指定桥目录：请使用 --game-dir、环境变量 FFR_GAME_DIR 或 --bridge-dir")
            })?,
    };
    let data_dir = args
        .data_dir
        .or_else(|| ffr_data::export_dir_from_env().ok());

    let module = FfrModule::new(bridge_dir.clone(), args.exe)
        .with_data_dir(data_dir)
        .with_reveal_hidden_weaknesses(args.reveal_hidden_weaknesses)
        .with_enable_input(args.enable_input);

    module
        .sync_input_switch()
        .await
        .with_context(|| format!("同步输入开关文件 {INPUT_SWITCH_FILE} 失败"))?;
    let _switch_guard = InputSwitchGuard {
        bridge_dir,
        armed: args.enable_input,
    };

    serve_stdio(module).await.context("运行 AGP 游戏模块失败")?;
    Ok(())
}
