# Final Fantasy Resonance Agent

本项目让 AI 玩 Steam 版《FINAL FANTASY RESONANCE》。做法沿用姊妹项目 [Final-Fantasy-Pixel-Remaster](https://github.com/AutoGamePlaying/Final-Fantasy-Pixel-Remaster)：AI 读游戏数据和攻略，一次规划一整章，代码执行；先在离线环境里验证计划，真机只做最后确认。

目前处于探路阶段，方案见 [docs/plan.md](docs/plan.md)。

## 开发

需要 Node.js 24 或更新版本。

```bash
npm install
npm run check     # 类型检查 + eslint + prettier 检查 + node --test
npm test          # 只跑测试
npm run format    # 按 prettier 格式化
```

`npm run check` 里的分层 lint 规则在 `eslint-rules/layers.js`：`shared ← data ← io ← skills ← runner ← planner ← host`，`io/live` 和 `io/sim` 互不 import，跨层只走 `index.ts`。

### 游戏机配置

```bash
cp machine/machine.env.example machine/machine.env
```

填 `LAPTOP_HOST`、`LAPTOP_USER`、`LAPTOP_ROOT`（游戏机上的工作目录）、`FFR_GAME_DIR`（游戏的 `Win64` 目录）和 `FFR_APP_ID`（只有 `game-launch` 用）。`machine/machine.env` 不进版本库。运行时数据默认放 `.deps/`，可用 `FFR_DEPS` 换位置。

`game-launch` 依赖游戏机上的计划任务。把 `machine/run-in-session.ps1` 和 `machine/register-session-task.ps1` 放到 `LAPTOP_ROOT\deploy\laptop\`，然后在游戏机上用管理员 PowerShell 运行一次（脚本可重复运行）：

```powershell
powershell -ExecutionPolicy Bypass -File <LAPTOP_ROOT>\deploy\laptop\register-session-task.ps1 -AllowUser <LAPTOP_USER>
```

### UE4SS 管理

先把 UE4SS 放到 `.deps/ue4ss/`，目录里必须有 `dwmapi.dll` 和 `ue4ss/` 两项。

```bash
# 装进 Win64；已有同名文件就拒绝。清单写到 <LAPTOP_ROOT>/ue4ss-manifest.json
node src/host/cli/ue4ss-install.ts [--dry-run]

# 按清单删除；清单外的文件不碰
node src/host/cli/ue4ss-uninstall.ts [--dry-run]

# 同步 mod/FfrProbe/ 到 ue4ss/Mods/FfrProbe/，默认在 mods.txt 里启用
node src/host/cli/mod-sync.ts FfrProbe [--disable] [--dry-run]

# 取 ue4ss/UE4SS.log；--tag 只留包含该标签的行，游戏运行时也能读
node src/host/cli/ue4ss-log.ts [--tag "[FfrProbe]"] [--dry-run]

# 经计划任务在桌面会话里用 steam://rungameid/<appid> 启动游戏
node src/host/cli/game-launch.ts [--timeout 120] [--dry-run]
```

这些命令都可以加 `--machine-env <文件>`（或设 `FFR_MACHINE_ENV`）换配置文件；`--dry-run` 只打印计划，不连接游戏机。安装、卸载、同步和启动在游戏运行时都会拒绝，`ue4ss-log` 例外（用共享读）。

## R1 转储

R1 阶段在几个场面上各取一份只读转储，交给 `mod/FfrDump/` 模组执行。模组和命令之间的约定（目录、任务号、心跳、完成标记）见 [docs/r1-dump.md](docs/r1-dump.md)。

```bash
# 触发一次任务并等完成标记；ok:true 时默认把结果拉到 .deps/r1/<任务号>/
# job 是 snapshot / objects / usmap / jmap / sdk / uht 之一
node src/host/cli/dump-run.ts <job> --label <场面名> [--timeout <秒>] [--no-fetch] [--dry-run]

# 只拉一个已经完成的任务；游戏在不在运行都可以
node src/host/cli/dump-fetch.ts <任务号> [--dry-run]

# 看游戏是否在运行、心跳年龄和 busy、有没有待处理的触发文件、最近 10 个任务
node src/host/cli/dump-status.ts [--dry-run]
```

典型流程：

1. `node src/host/cli/mod-sync.ts FfrDump` 把模组同步到游戏机并启用；
2. `node src/host/cli/game-launch.ts` 启动游戏；
3. 用户在游戏里走到要转储的场面，停住不动；
4. `node src/host/cli/dump-run.ts snapshot --label field-1`；
5. `node src/host/cli/dump-run.ts usmap --label field-1`。

`dump-run` 要求游戏正在运行，其他两个命令在游戏运行时也能用。转储只放在 `.deps/`，不进版本库。

### R2 数据导出

`tools/ffr-export/run.sh` 可列出游戏资源、保存原始包字节，并在取得 R1 的 `.usmap` 后导出 JSON。安装方法、三个子命令和输出约定见 [tools/ffr-export/README.md](tools/ffr-export/README.md)。

## Rust 工作区与 AGP 游戏模块

仓库根目录是 Cargo workspace（`Cargo.toml`、`Cargo.lock` 与 `target/` 均在根目录），包含两个成员 crate：

- `crates/ffr-data`：离线游戏数据层，按需加载并缓存 `.deps/ffr/export/`（或 `$FFR_DEPS/ffr/export`）中的 `DataTable` / `CompositeDataTable`、`StringTable` 与 `Localization/<lang>.json`，提供语言感知的 `FText` 解析（默认 `zh-Hans`，缺失时不把日文冒充中文）以及章节、主线剧情的类型化查询。
- `crates/ffr-module`：独立的 AGP 外部游戏模块（工具集版本 `ffr-1`），通过 JSON-RPC 由 AGP 启动，经文件桥读取 `FfrBridge` 状态（`game_status`、`bridge_ping`），并提供离线游戏数据查询工具（`read_chapter`、`read_map`、`read_enemy`、`read_battle_group`、`read_skill`、`read_item`、`read_quest`、`read_shop`、`search_data`）与分页只读资源（`ffr://chapters`、`ffr://maps`、`ffr://enemies`、`ffr://skills`）。

开发和检查需要 Rust 工具链；在仓库根目录运行整个 workspace 的检查（`cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`）与 Windows 交叉编译：

```bash
./scripts/check-rust.sh
cargo xwin build --release --target x86_64-pc-windows-msvc -p ffr-module
```

产物是 `target/x86_64-pc-windows-msvc/release/ffr-module.exe`。游戏机上除可执行文件外，还需将开发机导出的游戏数据目录（即 `<FFR_DEPS>/ffr/export`，至少包含 `FFRS/Content/Datatable/` 与 `Localization/zh-Hans.json`）单独拷贝部署到游戏机，再在 AGP 运行配置的 `[module]` 段填写：

```toml
[module]
command = "C:/path/to/ffr-module.exe"
args = [
  "--game-dir", "D:/Games/FFRS/Win64",
  "--data-dir", "C:/path/to/ffr-export",
]
default_timeout_secs = 15
```

- `--game-dir` 也可由环境变量 `FFR_GAME_DIR` 提供；文件桥默认是 `<game-dir>/ue4ss/ffr-out/bridge`，需要单独指定时改用 `--bridge-dir`。
- `--data-dir` 默认取环境变量 `FFR_DEPS` 下的 `ffr/export`。如果数据目录未指定或不存在，模块仍可正常启动，状态类工具（`game_status`、`bridge_ping`）不受影响，数据类工具会返回 `is_error` 与中文说明。
- 游戏程序名可用 `--exe` 覆盖，默认是 `FFRS-Win64-Shipping.exe`。

## 许可证和声明

代码以 [MIT 许可证](LICENSE) 发布。

- 本项目是非官方的个人研究项目，与 SQUARE ENIX 没有任何关系；FINAL FANTASY 是 SQUARE ENIX 的注册商标。
- 仓库里不含任何游戏文件、程序集、素材、文字、导出的类定义或攻略原文；运行时用到的游戏数据都从使用者自己的安装里读取，放在 `.deps/`，不进版本库。
- 游戏内只做读取和模拟按键，不修改游戏数据、存档或联网内容。
- 使用者需要自己拥有正版游戏。
