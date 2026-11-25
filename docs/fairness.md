# 公平性设计：为什么默认不泄露未揭示弱点

## 1. 背景：游戏内可见信息与内存/数据表的落差

在《FINAL FANTASY RESONANCE》的战斗中，敌人的属性弱点与抗性默认以“？”遮盖；玩家需要用对应属性命中目标，或通过侦测类能力（Libra）逐项揭露后，UI 才会显示该属性的弱点或抗性标记（见 [`docs/battle-rules.md`](battle-rules.md) §1）。

然而，从技术上我们可以同时从两处提前拿到完整的弱点与抗性表：

1. **运行时内存**：`CPP_VM_BtlUnitParam.ElemResistDataList` 在敌人刚入场、`bIsAnalyzed == false` 时就已经装着真实的 `element` 与 `resistance`（`FfrBridge` v1 照实采集为 `{ element, resistance, analyzed }`）。
2. **离线静态数据**：解包后的 `Unit/DT_UnitParameter.json` 记录了全部敌人的 `ElementResistanceList`。

如果 `ffr-module` 在 `read_game_state` 里直接把这两处未揭露的属性全部喂给 AI，模型就会在第一回合、未做任何试探的情况下精确知道所有未知敌人的弱点与吸收属性。

## 2. 为什么默认关闭（`--reveal-hidden-weaknesses` 默认 `false`）

本项目面向公开评测与真实游玩场景，**比赛与评测必须保证信息公平**：

- **与人类玩家信息对齐**：人类玩家在盲打或初见 Boss 时只能看到屏幕上已揭示的弱点以及“还有多少个未知弱点格（？）”，必须靠试探、记忆或侦测来补全信息。AI 在实时决策时默认只应获得等价的可见信息，不能靠读内存或解包表作弊。
- **文本与 JSON 双重隔离**：当开关关闭时，`read_game_state` 不仅在中文摘要里隐去未揭示属性（只提示“数据表另有 N 个未揭示弱点”），在结构化 JSON 里同样**不输出**未揭示的弱点/抗性元素列表（只给 `revealed`、`unrevealed_count` 和 `hidden_revealed: false`），杜绝“摘要藏了、JSON 漏了”的旁路泄露。
- **保留诚实计数**：告知模型“另有 N 个未揭示弱点”与游戏界面上显示剩余 N 个“？”格子的信息量等价——模型能判断“还值得继续试探其他属性”或“已全部揭示无需再试”，同时不会提前得知具体是哪种属性。

## 3. 什么时候打开 `--reveal-hidden-weaknesses`

启动 `ffr-module` 时传入 `--reveal-hidden-weaknesses`（或在测试/嵌入代码中调用 `FfrModule::with_reveal_hidden_weaknesses(true)`）后，`read_game_state` 会区分并同时输出 `revealed` 与 `unrevealed` / `all` 弱点和抗性。适用场景包括：

- **开发与回归调试**：核对 `FfrBridge` 运行时弱点数组与 `DT_UnitParameter` 静态表是否一致；
- **全信息/速通规则集**：明确允许查攻略或全量数据表的非盲打评测（等价于允许人类玩家对照攻略本游玩）；
- **离线规则表验证**：在 R5/R6 阶段验证战斗执行器与破防计算逻辑。

## 4. 与静态数据查询工具的边界

`ffr-module` 另提供 `ToolCategory::Data` 类别的离线查询工具（如 `read_enemy`、`read_battle_group`）与目录资源（`ffr://enemies`），用于离线规划、任务准备和允许查表的数据辅助模式。

在严格盲打或不允许赛中查敌方弱点表的比赛配置中，AGP 宿主可按工具类别策略禁用 `ToolCategory::Data`（或屏蔽敌方图鉴类工具），仅保留 `ToolCategory::Status`；得益于 `read_game_state` 默认关闭 `--reveal-hidden-weaknesses`，实时状态工具本身在开箱即用的状态下即满足公平性要求。
