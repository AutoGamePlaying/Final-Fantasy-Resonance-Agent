# 状态地图：战斗

本文基于 R1 真机快照（`.deps/r1/` 下 2026-11-24 序章及第 1 章前期 30 份关键快照）、FfrBridge v0 状态轨迹（`trace-*.tsv`）、战斗期反射数据（`20261125-battle-jmap`）与离线数据表（`docs/data-tables.md`），定稿《FINAL FANTASY RESONANCE》战斗主题的状态读取规范。

> [!NOTE]
> 置信度定义：**高**＝经至少两组对照快照（状态前后变化）实证；**中**＝单份快照证实、或结构经反射/间接 UI 验证但存在 `FfrDump` 截断限制；**低**＝仅反射或数据表推断，尚未采集真机快照。

---

## 1. 战斗进行中的判断

### 为什么不能用 `GameMode` / `map` / `pawn_class`

在 `trace-42..50.tsv`、`trace-87..95.tsv` 及全部战斗快照的 `context.json` 中，进入战斗、执行指令、进入结算直至返回野外，`GameMode` 始终为 `CPP_MuchaGameModeField`，`map` 始终保持野外关卡名（如 `Dng_01Gra_40_PL`），`pawn_class` 始终为 `BP_MapUnit_Player_C`。战斗是在当前野外关卡之上以子关卡/管理子系统方式叠加运行的。

### 权威判定字段（置信度：高）

| 判定目的 | 类（实例数） | 属性 | 实测变化与证据 |
| --- | --- | --- | --- |
| 是否在战斗中（含结算） | `CPP_UI_BattleSubsystem`（单例 1/1） | `bDuringBattle: bool` | `title` / `field-idle` / `field-walk` / `boss-start` / `boss-closeup-talk` 为 `false`；从开战到结算完毕（`battle-start`…`weakness-hit`、`battle-result`…`learn-skill-2`、`boss-dialog`…`boss-defeated`）全程为 `true` |
| 顶层流程任务 | `CPP_TaskManager`（单例 1/1） | `m_pTask: CPP_Task*` | 读取其运行时类名：`CPP_TitleTaskMain`（标题）、`CPP_MapTaskMain`（野外/战前事件）、`CPP_BtlTaskStartup`（开战事件）、`CPP_BtlTaskCommand`（常规回合）、`CPP_BtlTaskAdditionalTurn`（追加阶段）、`CPP_BtlTaskWin`（胜利结算）；反射中另有 `CPP_BtlTaskLose`（败北，待抓） |

> [!WARNING]
> **禁止用 `ForEachUObject` 扫描 `CPP_BtlTask*`**：内存里会残留多达十余个未 GC 的历史 `CPP_BtlTaskCommand` / `CPP_BtlTaskAdditionalTurn` 实例（如 `level-up` 快照）。必须读取单例 `CPP_TaskManager.m_pTask` 指针。

---

## 2. 战斗阶段与演出/对话/教学

单靠 `bDuringBattle` 无法区分“等玩家输入”和“播动画/对话/教学/结算”。经 30 份快照交叉核对，可用下列单例字段按优先级无歧义划分战斗子阶段：

| 优先级 | 子阶段 | 判定条件 | 置信度 | 快照证据 |
| --- | --- | --- | --- | --- |
| 1 | 教学弹窗阻塞 | `CPP_TutorialSubsystem.m_TutorialWidget ~= nil` | 高 | `battle-dialog`、`tutorial-break-bonus` 非空；`cmd-cursor-1`、`tutorial-break`、`tutorial-charge`、`boss-break`、`limit-2` 为空 |
| 2 | 对话/事件演出 | `CPP_TalkEventSubsystem.m_pEventManager ~= nil` 或 `CPP_VM_Battle.bIsEvent == true` | 高 | `battle-dialog`（`TalkEvent` 非空）、`boss-dialog`（二者均为真）、`boss-break`（`TalkEvent=nil` 但 `bIsEvent=true, bIsDisplayBattleWidget=false`） |
| 3 | 战斗结算 | `CPP_TaskManager.m_pTask` 为 `CPP_BtlTaskWin` 且 `CPP_UI_BattleSubsystem.ResultWidget ~= nil` 且 `ResultWidget.IsFinishPage == false` | 高 | `battle-result` / `growth-page` / `level-up` / `learn-skill` / `learn-skill-2` / `boss-defeated` 均为 `CPP_BtlTaskWin` + `IsFinishPage=false`；下一场战斗 `boss-dialog` / `boss-break` / `limit-2` 中 `ResultWidget` 仍非空但 `IsFinishPage=true` |
| 4 | 招式/行动执行中 | `CPP_BtlManager.m_pBattleJobManager.m_WorkingJobList` 长度 > 0（或 `m_WaitingJobList` 长度 > 0） | 高 | `battle-start` / `battle-command` / `battle-limit` / `battle-summon` / `tutorial-charge` 为 `1/0`（`CPP_BtlJobSkill`）；待机选指令时为 `0/0`；极限技演出中 `CPP_VM_Battle.bIsDisplayBattleWidget=false, bIsSequenceUIVisible=false`（`battle-limit`） |
| 5 | 等待指令输入 | `VM_BtlCmd_C.bDisplayCmd == true`（且上述 1–4 均不成立）；细分由 `CPP_UI_BtlCmdManager.CmdState` 给出：`List`(0) / `Target`(1) | 高 | `battle3-start` / `cmd-cursor-1` / `cmd-cursor-2` / `skill-list` / `limit-2` 为 `true + List`；`target-a` / `target-b` / `battle2-start` 为 `true + Target` |
| 6 | 回合过渡/等待 | `bDuringBattle == true` 且以上均不成立 | 高 | `weakness-hit`（动作结算与下一人接管间隙）、`tutorial-break`（关教学瞬间）、`CPP_BtlTaskStartup` 开场过渡 |

### 回合阶段（`eBtlPhaseType` / `eBtlCmdType`）

- `CPP_UI_BattleSubsystem.BtlPhaseType` 与 `CPP_UI_BtlCmdManager.CmdType` 均区分 `Normal`(0)、`Additional`(1)、`Finish`(2)。
- **实证（高）**：常规回合（`cmd-cursor-1` 等）二者均为 `Normal`；Boss 破防后的追加回合（`limit-2`）二者均为 `Additional`，同时 `CPP_TaskManager.m_pTask` 切为 `CPP_BtlTaskAdditionalTurn`。
- **边界细节（高）**：`tutorial-break-bonus`（全面破防教学刚弹出时）`CmdType` 已先切为 `Additional` 且 `ActionUnitUniqueIdList=[1, 0]`，但 `BtlPhaseType` 仍短暂保持 `Normal`，直到关掉教学进入 `CPP_BtlTaskAdditionalTurn` 后同步为 `Additional`。
- **未抓（低）**：`Finish` 阶段（全面破防后追加阶段末尾的幻兵终结技）尚未出现在序章/第 1 章开头的快照中。

### 关键避坑点

- **`WBP_Tutorial` 控件会复用来缓存旧文案**：关闭教学后 `WBP_Tutorial` 仍存在于内存并保留上一条标题/正文（例如 `cmd-cursor-1` 留着“主要功能”，`boss-break` 留着“危险行动”）。**只能**通过 `CPP_TutorialSubsystem.m_TutorialWidget ~= nil` 判定教学是否正在显示。
- **`CPP_UI_BattleSubsystem.BtlWidget` 始终为 `null`**：全部快照中均为 `null`，不要依赖它判断战斗 UI 是否存在；请用 `CPP_VM_Battle.bIsDisplayBattleWidget` 和单例 `WBP_BattleLayout_C`。
- **`bDisplayCmd` 在弹窗下层可保持 `true`**：`battle-dialog` 与 `tutorial-break-bonus` 中 `bDisplayCmd` 为 `true`，但输入被教学/事件遮挡。因此必须按上表优先级先检查教学与事件。

---

## 3. 当前行动者与时间轴

### 当前行动者（置信度：高）

| 属性 | 来源对象 | 实测变化与说明 |
| --- | --- | --- |
| `CurrentActionUnitUniqueId: int32` | `CPP_UI_BattleSubsystem`（与 `CPP_VM_Battle.CurrentActionUnitUniqueId` 同步） | 战斗内参战单位下标（`UniqueID`）：`battle3-start` / `cmd-cursor-1` / `cmd-cursor-2` / `skill-list` / `target-a` / `target-b` 为 `0`（雷因）；`weakness-hit` / `tutorial-break` / `limit-2` 为 `1`（拉斯韦尔）；非操作期（`battle-dialog` / `battle-limit` / `battle-result` / `boss-dialog` / `tutorial-charge` / `boss-break`）为 `-1` |
| `ActionUnitUniqueIdList: TArray<int32>` | `CPP_UI_BtlCmdManager` | 当前阶段可行动队列：普通回合为 `[0]`，全面破防追加阶段（`tutorial-break-bonus`、`limit-2`）为 `[1, 0]` |
| `ActionVM.bIsInAction` / `bIsInExPhase` | `CPP_VM_BtlUnitParam`（按单位） | `bIsInAction=true` 仅在当前行动者上成立（`cmd-cursor-1` 在 Unit 0，`limit-2` 在 Unit 1）；进入追加阶段时全队 `bIsInExPhase=true`（`tutorial-break-bonus`、`limit-2`） |

> 附：`CPP_BtlUnitManager.m_pCmdSelectUnit` 指向最近一次选指令的 `CPP_BtlUnit*`，但在技能执行中与结算页不会清空；判断当前是否轮到某人操作应以 `CurrentActionUnitUniqueId >= 0` 配合 `bDisplayCmd == true` 为准。

### 时间轴与行动顺序（置信度：高）

核心对象为单例 `VM_Timeline_C`（类 `CPP_VM_BtlTimeLine`）：

- **`TimelineUnified.UnifiedIds: TArray<int32>`**：当前回合与下一回合按顺序展开的 `UniqueID` 列表。
  - 例：`cmd-cursor-1` 为 `[0, 1, 2, 3, 1, 2, 3, 0]`（雷因、拉斯韦尔、钢蝠 A/B 交替）；
  - Boss 破防对照：`tutorial-charge` 为 `[1, 2, 0, 1, 2, 0]` → `boss-break` 切为 `[0, 1, 0, 1, 2, 0]` → `tutorial-break-bonus` / `limit-2` 切为 `[1, 0, 1, 2, 0]`。
- **`TurnVMMap`（`0`＝`CPP_VM_BtlCurrentTurn`，`1`＝`CPP_VM_BtlTurn`）的 `.EntryDataList`**：每项含 `UnitUniqueId`、`BelongPhaseType`（`Normal`/`Additional`）、`BelongTimeline`（`Current`/`Next`）。
  - 破防前后对照：`tutorial-charge` 中 `TurnVMMap[0]` 为 `[(2, Normal, Current), (0, Normal, Current)]` → `boss-break` 中直接变为 `[(1, Additional, Current), (0, Additional, Current)]`，实证 Boss `2` 的本回合行动已被取消、我方进入追加行动。
- **`bIsCmdState: bool`**：选指令/选目标期（`battle2-start`、`battle3-start`…`target-b`、`tutorial-break-bonus`、`limit-2`）为 `true`，其余为 `false`。
- **`RiskUnitUniqueIdList: TArray<int32>`**：正在蓄力/预告高危攻击的敌方 `UniqueID` 列表（见第 6 节）。
- 底层对照：`CPP_BtlUnitTimelineScheduler.m_AdditionalTimeline` 平时为空 `[]`，Boss 破防后（`tutorial-charge`…`limit-2`）变为 `[CPP_BtlUnit(1), CPP_BtlUnit(0)]`。

---

## 4. 指令菜单、招式列表与习得技能

### 菜单开关、层级与光标（置信度：高）

基于三组连续成对快照：`cmd-cursor-1`（光标在“战斗”）→ `cmd-cursor-2`（光标在“招式”）→ `skill-list`（进入“招式”子菜单，光标在“火炎”），以及 `limit-2`（进入“极限技”子菜单，光标在“歼月”）：

| 属性 | 来源对象 | `cmd-cursor-1` | `cmd-cursor-2` | `skill-list` | `limit-2` |
| --- | --- | --- | --- | --- | --- |
| `bDisplayCmd` | `VM_BtlCmd_C` | `true` | `true` | `true` | `true` |
| `CmdState` | `CPP_UI_BtlCmdManager` | `List` | `List` | `List` | `List` |
| `AncestorIdList` | `VM_BtlCmd_C` | `[]` | `[]` | `[1]` | `[10]` |
| `FocusCmdData.EntryIndex` | `VM_BtlCmd_C` | `0` | `1` | `0` | `0` |
| `FocusCmdData.Value.ActionKey.ID` | `VM_BtlCmd_C` | `401010`（战斗） | `1`（招式） | `220010`（火炎） | `402040`（歼月） |
| `FocusSkillId` | `CPP_UI_BattleSubsystem` / `CPP_VM_Battle` | `401010` | `1` | `220010` | `402040` |

- `CPP_BtlCmdStack.Commands` 同步记录压栈深度（`0 → 0 → 1 → 2`；`target-a` 中压入 `[1, 220010]`，`AncestorIdList=[1, 220010]`）。
- `CPP_VM_Tooltip_Skill.ActionData` 同步刷新当前光标项的本地化 `Name` 与 `Description`（如 `战斗` → `招式` → `火炎`），但对桥而言直接传 `ID` 给宿主查离线数据表更轻量。
- 注意：`EnhancedUI` 基类的 `FocuedItemDataMap` 在战斗菜单里无需使用，`VM_BtlCmd_C.FocusCmdData` 更直接且无脏对象干扰。

### 一级指令与二级招式列表（置信度：高）

菜单条目列表挂在当前激活的列表框架上，**必须经单例 `WBP_BattleLayout_C` 向下导航**（因为内存中会同时残留多达 6 个旧 `WBP_BtlCmd_ListFrame_C` 和 2 个 `WBP_BtlCmdLayout_C`）：

```text
WBP_BattleLayout_C (单例)
  └─ WBP_BtlCmdLayout (WBP_BtlCmdLayout_C)
       └─ ContentWidget
            ├─ [List 态] WBP_BtlCmd_ListFrame_C
            │    ├─ WBP_BtlCmd_PrimaryList.WBP_List.ActionKeyList  (一级菜单)
            │    └─ WBP_BtlCmd_SecondaryList.WBP_List.ActionKeyList (二级子列表)
            └─ [Target 态] WBP_BtlCmd_UnitSelect_C
```

实测 `ActionKeyList`（每项含 `Category: eActionCategory` 与 `ID: int32`）与离线表映射：

- **一级菜单（`PrimaryList`）**：
  - 雷因（`cmd-cursor-1` / `cmd-cursor-2` / `skill-list`）：`[401010, 1, 400010, 7, 400020]` → 战斗 / 招式 / 防御 / 道具 / 逃走。
  - 拉斯韦尔（`limit-2`）：`[402010, 1, 400010, 7, 400020]` → 战斗 / 招式 / 防御 / 道具 / 逃走。
  - 编号规律：小 ID（`1`=招式、`5`=幻兵召唤、`6`=幻兽召唤、`7`=道具、`10`=极限技等）查 `DT_CommandSkillData`；6 位大 ID（`401010`、`400010`、`400020`、`220010`、`401080`、`402040` 等）查 `DT_SkillData`。
- **二级菜单（`SecondaryList`）**：
  - `cmd-cursor-1` / `cmd-cursor-2`：`[]`（未展开子菜单）。
  - `skill-list`（雷因 Lv1，展开 `Cmd[1]` 招式）：`[220010]`（火炎，MP 5，break 17）。
  - `limit-2`（拉斯韦尔展开 `Cmd[10]` 极限技）：`[402040]`（歼月，LB 100，break 27）；同快照中雷因上一回合残留的旧 `ListFrame` 上可见其 Lv3 后招式列表已更新为 `[401080, 220010]`（撞击 + 火炎）。

### 升级习得新招式（置信度：高）

对照 `level-up` → `learn-skill` → `learn-skill-2`：

- `WBP_Btl_Result_UnitAbilityList_C.m_NewAbilityList`：`[401080]`（雷因习得“撞击”）→ `[402130]`（拉斯韦尔习得“清霜”）；同对象 `m_UnitData.addSkillList` 为 `[[401080, 3]]` → `[[402130, 3]]`。
- `WBP_BtlResultPage_LevelUpWindow_C.m_UnitDetailIndex` 由 `0` 变为 `1`。
- **重要避坑**：`CPP_MuchaGameSaveData.unitParameterList[*].activeSkillIdList` 在升到 Lv3 学会“撞击”“清霜”后仍只有 `[401010]` / `[402010]`（普攻），**不包含等级习得技能**。角色当前可用招式须以战斗内 `SecondaryList.ActionKeyList`、或 `CPP_UnitParameter:GetActiveSkillList()` / 离线 `DT_UnitLevelParameter` 结合当前等级为准。

---

## 5. 选目标

基于三组对照快照：`skill-list`（选招式）→ `target-a`（选目标，钢蝠 A）→ `target-b`（切换至钢蝠 B），辅以 `battle2-start`（防御选自身目标）。

### 核心 ViewModel：`VM_BtlCmdUnitSelect_C`（置信度：高）

位于 `VM_BtlCmd_C.UnitSelectVM`（单例）：

| 属性 | `skill-list` | `target-a`（钢蝠 A） | `target-b`（钢蝠 B） | `battle2-start`（防御） | 说明 |
| --- | --- | --- | --- | --- | --- |
| `CPP_UI_BtlCmdManager.CmdState` | `List` | `Target` | `Target` | `Target` | 是否进入目标选择态 |
| `FocusBaseUniqueId` | `-1` | `2` | `3` | `0` | **当前选中的主目标 `UniqueID`** |
| `targetUniqueIdList` | `[]` | `[2]` | `[3]` | `[0]` | 当前命中的全部目标 `UniqueID` 列表（群攻时含多个） |
| `AllTargetUniqueIdList` | `[]` | `[0, 1, 2, 3]` | `[0, 1, 2, 3]` | `[0]` | 本技能允许切换的全部可选目标池 |
| `TargetUniqueIdListMap` | `{}` | `{0:…, 1:我方, 2:敌方}` | 同左 | `{…}` | 按 `eRelation` 分组的可选目标池 |
| `DefaultFocusUniqueIdMap` | `{}` | `{2: 2, 1: 0}` | `{2: 3, 1: 0}` | `{…}` | 各阵营当前默认/记忆焦点 |
| `SkillDefaultRelation` | `None` | `Enemies` | `Enemies` | `Friendlies` | 技能默认指向阵营 |
| `bCanBeAoE` | `false` | `true` | `true` | `false` | 当前魔法是否支持切换全体化 |
| `CmdValue.IsAll` | `false` | `false` | `false` | `false` | 当前是否处于全体化模式（全体化态尚未实抓，见缺口） |
| `CmdSkillData.ID` / `CmdValue.ActionKey.ID` | `-1` | `220010`（火炎） | `220010`（火炎） | `400010`（防御） | 待释放技能 ID |

### 单位级同步与破防预测（置信度：高）

- **`CPP_VM_BtlUnitParam.TargetSelectState`**（`eSkillTargetSelectStateType`：`None`=0 / `Main`=1 / `Sub`=2）：
  - `target-a`：Unit 2（钢蝠 A）=`Main`，Unit 3（钢蝠 B）=`None`；
  - `target-b`：Unit 2=`None`，Unit 3=`Main`；
  - `battle2-start`：Unit 0=`Main`。
- **破防槽削减预测**：选中钢蝠 A/B 释放火炎（`breakDamageValue=17`，钢蝠总 BP `50`）时，对应目标的 `WBP_PredictProgress_C` 变为 `bEnablePredict=true, PredictPercent=0.66`（即 `(50 - 17) / 50 = 0.66`）。
- **同类敌人字母编号（A/B/C）**：`WBP_Btl_LibraUnitName` 下 `WBP_Btl_Identification.Text_Name` 在 `target-a` → `target-b` 中由 `'A'` 变为 `'B'`；反射中 `CPP_BtlUnitParameter:GetDuplicationId()` / `CPP_VM_BtlUnitParam:GetDuplicationId()` 提供同名编号。
- **按键行为记录**：用户实测 A/D 与 W/S 均可切换目标；执行器每次按键后直接回读 `FocusBaseUniqueId` 是否等于期望目标 `UniqueID` 即可闭环确认，无需硬编码按键方向与站位顺序。

---

## 6. 敌人与我方单位状态

### 单位总入口（置信度：高）

`CPP_UI_BattleSubsystem.BtlUnitManager`（`CPP_UI_BtlUnitManager`，单例）持有 `UnitParamVMMap: TMap<int32, CPP_VM_BtlUnitParam*>`，以战场 `UniqueID`（`0..N-1`）索引全部参战敌我单位。每项 `CPP_VM_BtlUnitParam` 均直接持有：

- `UniqueID: int32`、`UnitId: int32`、`ShortName: FText`、`bIsVisible: bool`、`bIsAnalyzed: bool`
- `BtlUnitParam: CPP_BtlUnitParameter*` → `m_pOwnerBtlUnit: CPP_BtlUnit*` → `m_pAI: CPP_BtlUnitAI*`
- `HitPointVM` / `MagicPointVM` / `BreakPointVM`（`VM_UnitParamPoint_C*`）与 `LimitPointVM`（`CPP_VM_LimitPoint*`）

### 编号与数据表映射（置信度：高）

- `CPP_VM_BtlUnitParam.UnitId` 与 `DT_UnitParameter.ID` 100% 对应：
  - 我方：`1001` 雷因、`1002` 拉斯韦尔、`1006` 魔人菲娜（序章显示 `ShortName='？？？'`）；
  - 敌方：`1` 士兵、`2` 钢蝠（HP 68 / BP 50 / 弱冰风）、`90` 魔导装甲、`278` 突击兵、`401` 贝尔摩达（Boss，HP 2000 / BP 145 / 弱火暗、雷减半）、`412` 新型魔导兵器、`445` 铁巨人。
- **战斗组 ID（置信度：中）**：`CPP_BtlManager` 未直接反射 `BattleGroupId` 属性，反射函数库存在 `CPP_BFL_Battle:GetEncountBattleGroupdId()`（待在桥中验证）；此外可用敌方 `UnitId` 列表与 `DT_BattleGroup` 互证（如 `[2, 2]` → `1001`，`[401]` → `9004`）。
- **阵营与生死/Boss 标志**：`CPP_BtlUnitParameter`（及 `CPP_UnitParameter`）提供 `GetRelation()`（`eRelation::Friendlies`=1 / `Enemies`=2）、`IsBoss()`、`IsDead()`、`IsBreak()`、`GetLevel()`、`GetDuplicationId()`；同时观察到敌人战败退场后 `CPP_VM_BtlUnitParam.bIsVisible` 翻为 `false`（`battle-start` Unit 2 士兵、`weakness-hit` Unit 3 钢蝠 B）。

### HP、MP 与破防槽 BP（置信度：中-高，附截断说明）

> [!IMPORTANT]
> **快照截断说明**：`FfrDump` 对每个类最多转储 50 个对象，而 `VM_UnitParamPoint_C` 在战斗中存在 153–156 个实例，导致战斗单位的 `HitPointVM` / `MagicPointVM` / `BreakPointVM` 实例未落盘进 `objects.jsonl`。但在 FfrBridge v1 中是通过 `CPP_VM_BtlUnitParam` 指针直接读取，不存在扫描上限。

现有多层证据已将结构与动态更新全部闭环：

1. **VM 结构（高）**：同快照已转储的 50 个 `VM_UnitParamPoint_C`（继承 `CPP_VM_UnitParamPoint` → `CPP_VM_Point`）均暴露 `CurrentPoint: int32`、`MaxPoint: int32`、`ParamType: eUnitParameterType`（`HitPoint`=1 / `MagicPoint`=2 / `BreakPoint`=3）、`PointMap`、`PredictPointVM`。
2. **指针与槽值同步（高）**：每个 `CPP_VM_BtlUnitParam` 均持有非空 `HitPointVM` / `MagicPointVM` / `BreakPointVM` 指针；绑定到 Boss `BreakPointVM` 的 `WBP_Btl_PointGauge_C.CurrentPercent` 在 `tutorial-break` → `tutorial-charge` / `boss-break` / `limit-2` 中由 `1.0` 变为 `0.0`。
3. **备选 C++ Getter（中）**：`CPP_BtlUnitParameter`（继承 `CPP_UnitParameter`）反射暴露 `GetHitPoint()`、`GetMaxHitPoint()`、`GetMagicPoint()`、`GetMaxMagicPoint()`、`GetBreakPoint()`、`GetMaxBreakPoint()` 及 `GetDisplay*` 系列，可在 FfrBridge v1 中作为 `PointVM` 的验证与兜底路径。
4. **推翻草稿假设（高）**：对比 `weakness-hit`（雷因 HUD 法力 `35 → 30`）及 Boss 战（拉斯韦尔 HUD 生命 `336 → 231 → 126`、法力 `50 → 45`），期间 `CPP_MuchaGameSaveData.unitParameterList` 始终冻结在战前值（`270/35`、`336/50`），**证明存档结构在战斗中不会实时同步**，战斗 HP/MP 必须读战斗单位对象。

### 弱点与揭露状态（置信度：高）

- 属性：`CPP_VM_BtlUnitParam.ElemResistDataList: TArray<UIUnitElemResistData>`，元素含 `{element: eElement, resistance: eResistance, bIsAnalyzed: bool}`。
- **成对证据**：
  - 钢蝠（`UnitId=2`）：`target-a` / `target-b` 为 `[(Wind, Weakness, false), (Ice, Weakness, false)]` → 命中冰属性后 `weakness-hit` 变为 `[(Wind, Weakness, false), (Ice, Weakness, true)]`。
  - Boss 贝尔摩达（`UnitId=401`）：`boss-dialog` / `tutorial-break` 为 `[(Dark, Weakness, false), (Fire, Weakness, false), (Thunder, Halved, false)]` → 命中火属性后 `tutorial-charge` / `boss-break` 变为 `(Fire, Weakness, true)`。
- **重要事实**：即便 `bIsAnalyzed == false`（界面显示为“？”），`ElemResistDataList` 内已完整包含真实弱点与耐性，并与离线 `DT_UnitParameter.ElementResistanceList` 一致。

### 破防 / 眩晕状态（置信度：高）

对照 `tutorial-break`（破防前）→ `tutorial-charge` / `boss-break` / `tutorial-break-bonus` / `limit-2`（破防后）：

| 读取位置 | 破防前 | 破防后 | 说明 |
| --- | --- | --- | --- |
| `CPP_BtlUnitParameter.m_ContEffectList` | `[]` | 含 `CPP_ContFxBreak` 实例 | 破防持续效果本体，其 `m_pActionUnitParameter` 指向破防者、`m_pTargetUnitParameter` 指向被破防者 |
| `CPP_UnitParameter:IsBreak()`（getter） | （反射确认） | （反射确认） | C++ 原生判定，桥中优先调用，或遍历 `m_ContEffectList` 检查 `CPP_ContFxBreak` |
| `CPP_VM_TimeLineEntry.bIsBreak` | `false` | `true`（`tutorial-charge` / `boss-break`） | 时间轴条目上的破防标记；Boss 被移除本回合后下一回合条目为 `false` |
| `WBP_Btl_PointGauge_C.CurrentPercent`（BP） | `1.0` | `0.0` | 破防槽归零 |

### 蓄力 / 斗气 / 危险行动状态（置信度：高）

对照 `tutorial-break`（普通）→ `tutorial-charge` / `boss-break`（斗气预告）→ `tutorial-break-bonus` / `limit-2`（破防打断后解除）：

| 读取位置 | 普通 | 危险行动预告中 | 打断解除后 | 说明 |
| --- | --- | --- | --- | --- |
| `VM_Timeline_C.RiskUnitUniqueIdList` | `[]` | `[2]` | `[]` | **最简洁的全局高危敌人 `UniqueID` 列表** |
| `CPP_VM_TimeLineEntry.bIsRisky` | `false` | `true`（Unit 2 本回合条目） | `false` | 时间轴图标斗气/危险标志 |
| `WBP_BtlTimeLine_RiskIcon_C` 可见性 | `Collapsed`(1) | `SelfHitTestInvisible`(4) | `Collapsed`(1) | 时间轴斗气警告图标 |
| `CPP_BtlHighRiskAtkDelVal.uniqueIdList` | `[]` | `[2]` | `[]` | 触发时间轴高危 UI 的事件载荷 |
| `CPP_BtlUnitAI.m_ScheduledSkillId` + `IsScheduledHighRiskSkill()` | `500020` | （可对照 `DT_BtlUnitAIParameter.highRiskSkillIdList`，如 Boss 401 为 `505040` 强击冲撞） | `500020` | 敌方 AI 下次预定招式；`CPP_VM_BtlUnitParam:IsScheduledHighRiskSkill()` 亦提供直接 getter |

> 说明：`CPP_BtlUnit.m_pHighRiskAtkAuraEffect` 在 `tutorial-charge` 和 `boss-break` 中均为 `null`（抓拍瞬间 Boss 已被击破，特效已清或未挂在此指针），置信度标**低**，不要依赖视觉特效指针，应读 `VM_Timeline_C.RiskUnitUniqueIdList` 与 `IsScheduledHighRiskSkill()`。

### 我方极限槽与召唤

- **极限槽（置信度：高）**：`CPP_VM_BtlUnitParam.LimitPointVM`（类 `CPP_VM_LimitPoint`，未截断，全部转储）含 `CurrentPoint`、`MaxPoint`、`CurrentLevel`、`MaxLevel`。
  - 实测演进：`cmd-cursor-1` 雷因/拉斯韦尔 `0/100 (Lv 0/1)` → `battle-summon` 魔人菲娜 `5/100` → `weakness-hit` 拉斯韦尔 `12/100` → `tutorial-charge` / `boss-break` 拉斯韦尔 `65/100` → **`limit-2` 拉斯韦尔 `100/100 (CurrentLevel=1, MaxLevel=1)`**（极限技“歼月”可用）；`battle-limit` 为释放极限技 `408030`（混沌抉择）的演出中，`CurrentPoint` 已归 `0`。
  - 结合教学表 `ST_Tutorial_LB2`：习得 Lv2 极限技后 `MaxLevel=2`、`MaxPoint=200`。
- **召唤（置信度：中）**：
  - `battle-summon` 实证：执行记录 `CPP_UI_BtlCmdManager.ExecuteDataList[0].CommandId = 6`（`DT_CommandSkillData[6]` 幻兽召唤），`FocusCmdData.ActionKey.ID = 431120`（`DT_SkillData[431120]` 巴哈姆特，MP 50，`belong=[6]`），执行走普通 `CPP_BtlJobSkill`。
  - 反射显示召唤体出战后涉及 `CPP_BtlUnitParameter.{m_pVision, m_pVisionOwner}` 及 `CPP_BtlUnitAI:GetRemainingActionNumSummonedUnit()`，但 `battle-summon` 处于吟唱动画中、召唤体尚未入场，故在场召唤体属性置信度为**低**（缺快照）。

---

## 7. 战斗结算

基于 6 份结算快照：`battle-result`（报酬页）、`growth-page`（成长页）、`level-up`（升级页）、`learn-skill`（雷因习得“撞击”）、`learn-skill-2`（拉斯韦尔习得“清霜”）、`boss-defeated`（Boss 战结算）。

### 结算界面活跃与翻页判定（置信度：高）

- **是否正在结算**：必须同时满足 `CPP_TaskManager.m_pTask` 为 `CPP_BtlTaskWin` 且 `CPP_UI_BattleSubsystem.ResultWidget.IsFinishPage == false`（对应内层 `WBP_BtlResultPage_Layout.IsFinishePage == false`，注意游戏源码拼写 `IsFinishePage`）。
- **推翻草稿隐含假设**：`WBP_Btl_ResultLayout_C` 在首场结算后会被缓存复用，离开结算甚至进入下一场战斗（`boss-start` / `boss-dialog` / `boss-break` / `limit-2`）时 `ResultWidget` 仍非空，只是 `IsFinishPage` 变为 `true`。不可仅凭 `ResultWidget ~= nil` 判结算。
- **当前子页**：`ResultWidget.WBP_BtlResultPage_Layout.CurrentPageIndex`：`0`＝报酬页（`battle-result`），`1`＝成长/升级/习得招式页（`growth-page`、`level-up`、`learn-skill`、`learn-skill-2`、`boss-defeated`）。
- **不可用字段**：`WBP_Btl_ResultLayout_C.State` 与 `CPP_VM_BattleResult.SceneTypeStack` 在全部 6 份结算快照中均保持 `None` / `[]`，不要依赖。

### 报酬与成长数据（置信度：高）

单例 `CPP_VM_BattleResult.BtlResultData`（结构体 `BTL_RESULT_DATA`）：

| 字段 | 钢蝠战示例（`battle-result`） | Boss 战示例（`boss-defeated`） | 说明 |
| --- | --- | --- | --- |
| `baseMoney` / `totalMoney` / `beforeMoney` | `40` / `40` / `1000` | `750` / `750` / `1330` | 获得金币与战前金币 |
| `baseExperience` / `totalExperience` | `24` / `24` | `300` / `300` | 获得经验值 |
| `baseAp` / `totalAp` | `4` / `4` | `45` / `45` | 获得 MRP（精通点数） |
| `enemyDataList` | `[2, 2]` | `[401]` | 击败的敌方 `UnitId` 列表 |
| `itemDataList` | `[]` | `[{ItemId: 4, Num: 1}]` | 掉落道具列表 |
| `isBonus` / `isBonusAchieveds` | `false` | `false` | 奖励达成标志（无伤 / 破防终结 / 过量击杀） |
| `unitDataList` | `1001/1002: Lv 1→1, Exp 0→24` | `1001/1002: Lv 3→4, Exp 173→473` | 每名我方升级前后等级、经验与 `addSkillList`（如 `[[401080, 3]]`） |

> 注意：`BtlResultData` 在退出结算后保留上一场数据直到下次胜利覆盖，仅在“结算活跃”条件下才代表当前结算。

---

## 8. 战斗中的演出与无对话框镜头拉近

对比 `battle-dialog`、`boss-dialog`、`boss-start`、`boss-closeup-talk` 与 `boss-break`：

| 场面快照 | `bDuringBattle` | `m_pTask` | `TalkEvent` (`m_pEventManager`) | `CPP_VM_Battle.bIsEvent` / `bIsDisplayBattleWidget` | `WindowManager.m_pWindow` 窗口类 | 实质与应对 |
| --- | --- | --- | --- | --- | --- | --- |
| `battle-dialog` | `true` | `CPP_BtlTaskStartup` | 非空 | `false` / `true` | `WBP_TalkWindow_C`（同时弹出 `WBP_TutorialLayout_C` 教学） | 战中标准对话框 + 教学弹窗，按确认推进 |
| `boss-dialog` | `true` | `CPP_BtlTaskStartup` | 非空 | `true` / `false` | `WBP_TalkWindow_C`（雷因台词） | 战中标准对话框，HUD 隐藏，按确认推进 |
| `boss-start` / `boss-closeup-talk` | **`false`** | **`CPP_MapTaskMain`** | **非空**（`m_pEventPlayer` 为 `CPP_TalkEventSequence`，且 `m_pSequenceActor` & `m_pSequencePlayer` 非空） | `nil` / `nil` | **`WBP_TalkEvent_Caption_telop_C`**（屏幕字幕“嘎喔喔喔喔喔喔！！！”） | **关键发现**：`boss-closeup-talk`（用户观察“无对话框、只有镜头拉近”）实为进入战斗系统前的野外 `LevelSequence` 剧情演出，窗口用字幕控件而非普通对话框；两份快照差异是 `boss-closeup-talk` 时 `CPP_GameWidgetInputManager.InputBlockSet.TokenSet` 从 `['WindowDeactivate']` 清空且 Skip/Auto 提示消失，但 `m_pEventManager` 全程非空 |
| `boss-break` | `true` | `CPP_BtlTaskCommand` | `nil` | **`true` / `false`** | 无 | **战中无对话纯演出**（全面破防镜头特写与横幅动画），靠 `bIsEvent == true` 与 `bIsDisplayBattleWidget == false` 准确识别为“演出等待中” |

### 统一“演出/对话等待中”判据（置信度：高）

1. **事件系统演出（覆盖战前拉近 `boss-closeup-talk` 及战中对话 `battle-dialog` / `boss-dialog`）**：`CPP_TalkEventSubsystem.m_pEventManager ~= nil`。进一步区分“有对话框需按确认”还是“纯镜头/字幕播片”可看 `m_pEventManager.m_pWidget.WindowManager.m_pWindow` 的类名（`WBP_TalkWindow_C` vs `WBP_TalkEvent_Caption_telop_C`）及 `m_pEventManager.m_pScene.m_pEventPlayer` 是否为 `CPP_TalkEventSequence`。
2. **战中纯战斗演出（覆盖 `boss-break` 及 `battle-limit`）**：`CPP_VM_Battle.bIsEvent == true` 或 `CPP_VM_Battle.bIsDisplayBattleWidget == false` 或 `m_WorkingJobList` 非空。

---

## 9. 对草稿（`docs/state-map-draft.md` §2）候选的核对结果

| 草稿候选 | 核对结论 | 实机依据与修正说明 |
| --- | --- | --- |
| `CPP_UI_BattleSubsystem.{bDuringBattle, BtlPhaseType, CurrentActionUnitUniqueId, FocusSkillId, BtlCmdManager, BtlUnitManager}` | **证实** | 全部为单例稳定读写，与成对快照 100% 吻合 |
| `CPP_UI_BattleSubsystem.BtlWidget` | **推翻** | 全程为 `null`，改用 `CPP_VM_Battle.bIsDisplayBattleWidget` 与单例 `WBP_BattleLayout_C` |
| `CPP_UI_BattleSubsystem.ResultWidget`（仅判非空） | **推翻/修正** | 结算控件会缓存复用，离开结算后仍非空；必须配合 `CPP_TaskManager.m_pTask == CPP_BtlTaskWin` 与 `ResultWidget.IsFinishPage == false` |
| `CPP_BtlManager.m_pBattleJobManager.{m_WorkingJobList, m_WaitingJobList}` | **证实** | 技能/极限技/召唤执行中精确非空，选指令期为空 |
| `CPP_BtlUnitManager.{m_UnitList, m_pCmdSelectUnit, m_pTimelineScheduler}` | **部分证实/修正** | `m_UnitList` 与 `m_AdditionalTimeline` 证实；但 `m_pCmdSelectUnit` 在非选指令期不清空，改用 `CurrentActionUnitUniqueId`；单位遍历更推荐 `CPP_UI_BtlUnitManager.UnitParamVMMap` |
| `CPP_VM_BtlTimeLine.{TimelineUnified.UnifiedIds, TurnVMMap, bIsCmdState}` | **证实** | 完整行动顺序与追加阶段变化已实证；并新增证实高价值字段 `RiskUnitUniqueIdList` |
| `CPP_UI_BtlCmdManager.{CmdType, CmdState, ActionUnitUniqueIdList}` + `CPP_VM_BtlCmd.{bDisplayCmd, FocusCmdData, AncestorIdList, UnitSelectVM}` | **证实** | 完整覆盖菜单开闭、一级/二级层级、光标项与目标选择；列表内容需经 `WBP_BattleLayout_C.WBP_BtlCmdLayout.ContentWidget` 读取 |
| `CPP_VM_BtlCmdUnitSelect.{FocusBaseUniqueId, targetUniqueIdList, AllTargetUniqueIdList, TargetUniqueIdListMap, bCanBeAoE}` + `CPP_VM_BtlUnitParam.TargetSelectState` | **证实** | `target-a` 对 `target-b` 完整实证 |
| `CPP_BtlTask*`（直接扫描） | **修正** | 不能遍历 `CPP_BtlTask*` 实例（旧实例未 GC 堆积）；必须经 `CPP_TaskManager.m_pTask` 读取当前活动任务 |
| `CPP_MuchaGameSaveData.unitParameterList`（战中 HP/MP 与习得技能） | **推翻（战中场景）** | 战中 HP/MP 不同步，且 `activeSkillIdList` 不含等级习得招式 |
| `CPP_BtlUnitParameter:GetHitPoint/GetBreakPoint`、`CPP_BFL_Battle:GetEncountBattleGroupdId`、`CPP_BtlTaskLose`、`eBtlPhaseType::Finish` | **尚缺证据** | 反射已确认签名，受快照截断或未覆盖到对应场面，需 FfrBridge v1 实机验收或补抓快照 |

---

## 10. 尚缺快照清单

1. **战败（`CPP_BtlTaskLose`）与逃跑成功**：确认战败/逃跑结算时的任务类、弹窗及退出战斗时机。
2. **终结技阶段（`eBtlPhaseType::Finish` / `eBtlCmdType::Finish`）**：装备幻兵并触发全面破防后追加阶段末尾的终结技选择界面。
3. **道具子菜单（`Cmd[7]`）与魔法全体化切换（`CmdValue.IsAll == true`）**：确认 `SecondaryList.ActionKeyList` 在道具类下的 `Category` 枚举值、库存数量读法，以及全体化时 `targetUniqueIdList` 与 `TargetSelectState`（`Main`/`Sub`）的变化。
4. **召唤体/幻兵在场状态**：`battle-summon` 停在吟唱动画，尚未拍到召唤体落地进入 `UnitParamVMMap` 后 `m_pVision` / 剩余行动次数的状态。
5. **Boss 蓄力未破防的纯净态**：`tutorial-charge` 拍摄瞬间已打出破防（`CPP_ContFxBreak` 已挂上），需补拍一张 Boss 冒斗气且尚未破防时的快照，确认 `m_ScheduledSkillId == 505040` 与 `m_pHighRiskAtkAuraEffect` 表现。
6. **战斗内 `VM_UnitParamPoint_C` 直读验证**：因 `FfrDump` 每类 50 条上限截断了战斗单位的 `HitPointVM`/`MagicPointVM`/`BreakPointVM`，需在 FfrBridge v1 首次真机验收时核对直读 `CurrentPoint`/`MaxPoint` 与 C++ getter `GetHitPoint()`/`GetBreakPoint()` 的一致性。
7. **战斗状态异常（中毒/沉默/睡眠/麻痹/石化等）**：确认 `m_ContEffectList` 中各类异常对象类名与 `GetStatusCondition()` / `GetLifeTurn()` 返回值。

---

## 11. FfrBridge v1 建议字段

延续 `mod/FfrBridge/Scripts/ffrbridge/state.lua` 的只读 0.5 秒轮询架构，建议为战斗主题新增 **14 项字段/字段组**（4 个常驻顶层轻量字段 + `battle` 命名空间下 7 个核心状态字段与 3 个结构化分组 `cmd` / `units` / `result`）。

### 字段定义与读取路径

| # | 输出字段 | 类型 | 实例查找方式与属性读法 | 采样条件 |
| --- | --- | --- | --- | --- |
| 1 | `in_battle` | `boolean` | `FindFirstOf("CPP_UI_BattleSubsystem").bDuringBattle` | 常驻（每 0.5s） |
| 2 | `task_class` | `string \| null` | `FindFirstOf("CPP_TaskManager").m_pTask` 的短类名（如 `"CPP_BtlTaskCommand"`） | 常驻（每 0.5s） |
| 3 | `in_talk_event` | `boolean` | `FindFirstOf("CPP_TalkEventSubsystem").m_pEventManager` 是否有效（`IsValid()`） | 常驻（每 0.5s） |
| 4 | `in_tutorial` | `boolean` | `FindFirstOf("CPP_TutorialSubsystem").m_TutorialWidget` 是否有效 | 常驻（每 0.5s） |
| 5 | `battle.phase` | `string` | `uiBattle.BtlPhaseType` 映射为 `"Normal" \| "Additional" \| "Finish"` | 仅 `in_battle` |
| 6 | `battle.is_event` | `boolean` | `FindFirstOf("CPP_VM_Battle").bIsEvent` | 仅 `in_battle` |
| 7 | `battle.is_ui_visible` | `boolean` | `FindFirstOf("CPP_VM_Battle").bIsDisplayBattleWidget` | 仅 `in_battle` |
| 8 | `battle.working_job_count` | `number` | `#btlMgr.m_pBattleJobManager.m_WorkingJobList + #btlMgr.m_pBattleJobManager.m_WaitingJobList`（`btlMgr = FindFirstOf("CPP_BtlManager")`） | 仅 `in_battle` |
| 9 | `battle.current_actor_uid` | `number` | `uiBattle.CurrentActionUnitUniqueId`（无行动者为 `-1`） | 仅 `in_battle` |
| 10 | `battle.timeline` | `number[]` | `FindFirstOf("VM_Timeline_C").TimelineUnified.UnifiedIds`（遍历 `TArray`） | 仅 `in_battle` |
| 11 | `battle.risk_uids` | `number[]` | `FindFirstOf("VM_Timeline_C").RiskUnitUniqueIdList` | 仅 `in_battle` |
| 12 | `battle.cmd` | `object \| null` | 见下表（由 `VM_BtlCmd_C`、`uiBattle.BtlCmdManager`、`WBP_BattleLayout_C` 组装） | 仅 `in_battle` |
| 13 | `battle.units` | `object[]` | 遍历 `uiBattle.BtlUnitManager.UnitParamVMMap`（每单位字段见下表） | 仅 `in_battle` |
| 14 | `battle.result` | `object \| null` | 见下表（由 `uiBattle.ResultWidget` 与 `FindFirstOf("CPP_VM_BattleResult")` 组装） | 仅 `task_class == "CPP_BtlTaskWin"` 且 `ResultWidget.IsFinishPage == false` |

#### `battle.cmd` 子结构

- `display: boolean`：`vmCmd.bDisplayCmd`（`vmCmd = FindFirstOf("VM_BtlCmd_C")`）
- `type: string`：`cmdMgr.CmdType`（`Normal`/`Additional`/`Finish`，`cmdMgr = uiBattle.BtlCmdManager`）
- `state: string`：`cmdMgr.CmdState`（`List`/`Target`）
- `ancestors: number[]`：`vmCmd.AncestorIdList`
- `focus_index: number`：`vmCmd.FocusCmdData.EntryIndex`
- `focus_action_id: number`：`vmCmd.FocusCmdData.Value.ActionKey.ID`
- `primary_ids: number[]`（仅 `display` 且 `state == "List"`）：`battleLayout.WBP_BtlCmdLayout.ContentWidget.WBP_BtlCmd_PrimaryList.WBP_List.ActionKeyList[*].ID`（`battleLayout = FindFirstOf("WBP_BattleLayout_C")`）
- `secondary_ids: number[]`（仅 `display` 且 `state == "List"` 且 `#ancestors > 0`）：同 `ContentWidget.WBP_BtlCmd_SecondaryList.WBP_List.ActionKeyList[*].ID`
- `target: object | null`（仅 `display` 且 `state == "Target"`，取 `sel = vmCmd.UnitSelectVM`）：
  - `skill_id`: `sel.CmdValue.ActionKey.ID`
  - `focus_uid`: `sel.FocusBaseUniqueId`
  - `target_uids`: `sel.targetUniqueIdList`
  - `all_target_uids`: `sel.AllTargetUniqueIdList`
  - `can_aoe`: `sel.bCanBeAoE`
  - `is_all`: `sel.CmdValue.IsAll`

#### `battle.units[]` 每单位子结构

由 `uiBattle.BtlUnitManager.UnitParamVMMap` 遍历 `(uid, vm)`，再取 `param = vm.BtlUnitParam`：

- 基础：`uid`（`vm.UniqueID`）、`unit_id`（`vm.UnitId`）、`name`（`vm.ShortName:ToString()`）、`relation`（`param:GetRelation()`，`1`=我方/`2`=敌方）、`is_boss`（`param:IsBoss()`）、`is_dead`（`param:IsDead()`，兜底 `not vm.bIsVisible`）
- 数值：`hp` / `max_hp`（`vm.HitPointVM.CurrentPoint` / `MaxPoint`，兜底 `param:GetHitPoint()` / `GetMaxHitPoint()`）、`mp` / `max_mp`（`vm.MagicPointVM.*`，兜底 `GetMagicPoint()`）、`bp` / `max_bp`（`vm.BreakPointVM.*`，兜底 `GetBreakPoint()`）、`lb` / `lb_max` / `lb_level`（`vm.LimitPointVM.{CurrentPoint, MaxPoint, CurrentLevel}`）
- 战斗态：`is_break`（`param:IsBreak()`）、`is_risky`（`vm:IsScheduledHighRiskSkill()` 或查 `uid ∈ battle.risk_uids`）、`scheduled_skill_id`（`param.m_pOwnerBtlUnit.m_pAI.m_ScheduledSkillId`，仅敌方）、`in_action`（`vm.ActionVM.bIsInAction`）、`in_ex_phase`（`vm.ActionVM.bIsInExPhase`）、`target_state`（`vm.TargetSelectState`：`None`/`Main`/`Sub`）
- 弱点（仅敌方）：`weaknesses` — 遍历 `vm.ElemResistDataList`，输出 `{ element, resistance, analyzed }`（仅保留非 `None` 项以控体积）

#### `battle.result` 子结构

仅当 `task_class == "CPP_BtlTaskWin"` 且 `uiBattle.ResultWidget:IsValid()` 且 `not uiBattle.ResultWidget.IsFinishPage` 时读取：

- `page_index`: `uiBattle.ResultWidget.WBP_BtlResultPage_Layout.CurrentPageIndex`（`0`=报酬页，`1`=成长/升级页）
- `total_exp` / `total_money` / `total_ap`: `vmRes.BtlResultData.{totalExperience, totalMoney, totalAp}`（`vmRes = FindFirstOf("CPP_VM_BattleResult")`）
- `enemy_ids`: `vmRes.BtlResultData.enemyDataList`
- `items`: `vmRes.BtlResultData.itemDataList` 映射为 `{ item_id, num }` 数组

### 更新频率、性能与对象失效防护

1. **按需门控**：非战斗时仅执行 4 次单例 `FindFirstOf` 与单属性读取（`battle = null`），开销可忽略；战斗内每 0.5 秒完整采样一次（参战单位通常 3–8 个，单帧 Lua 开销 < 1 ms）。
2. **禁止跨帧缓存裸 UObject 引用**：开战、换图、回合结束、结算销毁都会令旧 `CPP_VM_BtlUnitParam`、`WBP_BtlCmd_ListFrame_C`、`CPP_BtlJob` 失效。每次 0.5 秒采样都从 `FindFirstOf` 单例重新向下解引用，每级指针先做 `pcall` + `IsValid()` 判空；若出现异常，记录进 `errors` 数组且不中断其余字段采样。
3. **不跨类暴力扫描**：除上述已验证 `alive == 1` 的单例（`CPP_UI_BattleSubsystem`、`CPP_TaskManager`、`CPP_TalkEventSubsystem`、`CPP_TutorialSubsystem`、`CPP_BtlManager`、`CPP_VM_Battle`、`VM_BtlCmd_C`、`VM_Timeline_C`、`CPP_VM_BattleResult`、`WBP_BattleLayout_C`）外，一律经父对象指针链向下取子对象，避开残留未 GC 对象（特别是 `CPP_BtlTask*`、`WBP_BtlCmdLayout_C`、`WBP_BtlCmd_ListFrame_C`）。
4. **操作后高频确认（供 R4/R5 参考）**：默认 0.5 秒写 `state.json` 足够观察状态迁移；执行器在发出按键（如切光标/切目标）后，只需等待下一次桥快照刷新并核对 `focus_index` / `focus_action_id` / `target.focus_uid` 即可完成闭环，无需把基础采样率盲目拉高到每帧。
