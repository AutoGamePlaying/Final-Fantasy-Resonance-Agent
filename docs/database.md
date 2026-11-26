# 本地游戏数据库（`ffr-db`）

本地 SQLite 游戏数据库的表结构、索引、构建器与命令行工具（`ffr-data`、`ffr-db`）已移至独立导出仓库 [final-fantasy-resonance-export](https://github.com/AutoGamePlaying/final-fantasy-resonance-export)，完整设计、建库与验收命令见 [final-fantasy-resonance-export/docs/database.md](https://github.com/AutoGamePlaying/final-fantasy-resonance-export/blob/main/docs/database.md)。

本仓库的 `crates/ffr-module` 通过 Cargo 路径依赖（`../final-fantasy-resonance-export`）引用导出仓库中的 `ffr-data` 与 `ffr-db`，只经只读 API `GameDb` 查询数据；给模型展示什么、隐藏什么仍由工具层按 [docs/fairness.md](fairness.md) 决定。

## 使用方式（`--db`）

- 默认读取 `$FFR_DEPS/ffr/db/ffr.sqlite`（未设置 `FFR_DEPS` 时不自动猜测路径），也可用 `ffr-module --db <文件>` 显式指定数据库文件。
- 仅在未传 `--db` 且显式传入 `--data-dir <导出目录>` 时，模块才回退到旧版导出 JSON 路径（留作对照）；`--db` 与 `--data-dir` 不能同时指定。
- 数据库文件与导出数据一样**不进版本库**；部署到游戏机时使用开发机不带 `--snapshots` 构建的单个 `ffr.sqlite` 文件。
