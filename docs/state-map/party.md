# 状态地图：队伍、成长、物品和剧情进度

本文基于 98 份真机场面快照（序章至第 1 章土之神殿 Boss 战击败，涵盖成对对照与自动快照）、`20261125-battle-jmap` 反射数据及 R2 导出数据表，对 `docs/state-map-draft.md` 中“队伍与能力值”“招式/成长”“物品与装备”“剧情进度与旗标”“存档数据结构”相关候选做真机核对并定稿。

> [!NOTE]
> 快照采集模组 `FfrDump` 对数组/Map 限制导出前 32 项、结构体嵌套限制 3 层、单类对象限制 50 个实例；凡被截断而借助反射和数据表补全的细节，下文均已单独注明置信度。

---

## 0. 草稿候选核对总表

| 草稿候选 | 核对结论 | 说明 |
| --- | --- | --- |
| `CPP_MuchaSaveDataManager.m_pGameSaveData`（`CPP_MuchaGameSaveData`） | **证实（高）** | 98/98 快照唯一存在，承载队伍、经验、背包、装备、金币、剧情进度、旗标、宝箱等运行态持久化数据；但**战斗中 HP/MP 不实时写回**，且标题画面承载的是 `Setup` 占位数据 |
| `CPP_MapUnitParameterStorage` → `CPP_UnitParameter` | **部分推翻（高）** | `CPP_UnitParameter` 仅暴露 4 个反射属性（`UnitName` 等），等级、上限、能力值均为非反射 C++ 成员，不能直接读属性（须调用 getter 函数）；且场景中常残留待 GC 旧实例（多达 24 个） |
| `CPP_VM_UnitParam`（草稿原标“中”） | **证实并升格（高）** | 挂在全局单例 `CPP_ManualViewModelManager.UnitVMManager.UnitParamVMMap`（98/98 存在），按 `UnitId` 索引，直接暴露计算后的 HP/MP/上限/攻防属性与装备/技能槽，且**战斗中 HP/MP 实时同步** |
| `SaveData_UnitParameter.activeSkillIdList`（存已学会招式） | **推翻（高）** | 仅存普攻、大师技能或剧情注入技能；**升级学会的招式/魔法不写入该列表**，而是按当前等级从 `DT_UnitLevelParameter*` 动态派生 |
| `CPP_VM_UnitParam.ParamPointVMMap[10]`（Level） | **推翻（高）** | 枚举项 `eUnitParameterType::Level`（键 `10`）在实机中恒为 `-1`；等级在 `LevelProgressionVM`（或 `GetLevel()` / 经验查表） |
| `CPP_UnitListSubsystem` | **无反射属性（中）** | 98/98 存在但无任何反射属性，仅有 `GetJoinUnitList` 函数，不需要经它绕路 |
| `SaveData_Progress.Progress` / `eventFlag` | **证实（高）** | `Progress[0]` 即主线进度计数器 `{progress:0}`，`eventFlag`（长度 2048）下标即 `{flag:<id>}` 编号，与事件表、主线表 100% 对上 |
| `SaveData_Progress.EventName` | **尚无证据（低）** | 长度固定 512，98 份快照前 32 项全程为空串 `""` |
| `CPP_ScenarioSubsystem` | **证实部分（高）** | 98/98 存在，`m_CurrentDestinationMapIdList` 随主线推进实时更新；`m_CurrentMainScenarioMapObjectList` 因当前阶段无目标点配置，尚无非空对照 |
| `CPP_QuestSubsystem` / `questBoardList` / `request` | **部分证实（中）** | `MainQuestId`（`0 → 1001`）与 `QuestNoticeDataList` 已证实；支线委托与任务板在第 1 章开头未解锁，尚无变更对照 |
| `CPP_MuchaSaveDataManager.GameSaveDataSummaryMap` & `CPP_VM_SaveData` | **证实（高）** | 固定 11 槽（`0..10`），`0` 为自动存档槽，`1..10` 为手动槽；`save-ui` 实证了槽 `1` 占用与槽 `2..4` 为空 |

---

## 1. 队伍与角色能力值

### 1.1 成员、顺序与入队状态（置信度：高）

- **当前编队与站位顺序**：`CPP_MuchaGameSaveData.Party`（结构体 `SaveData_Party`）
  - `partyList: TArray<int32>`：当前出战队伍的 `UnitId` 顺序数组。
  - `guestUnitId: int32`：客座角色 `UnitId`（本次快照全程为 `-1`，尚无客座入队对照）。
- **全主角入队状态**：`CPP_MuchaGameSaveData.unitParameterList`（`TArray<SaveData_UnitParameter>`，固定长度 8，顺序对应 `UnitId` `1001..1008`）中每项的 `JoinStatus`（枚举 `eUnitJoinStatus`：`Join=0`、`NotJoin=1`、`Leave=2`、`TemporalLeave=3`、`Lost=4`）。
- **快照证据**：
  - `20261124-161016-title-snapshot`（Setup 占位）：`partyList = [1001, 1002, 1003, 1004]`。
  - `20261124-161345-cutscene-snapshot` → `162146-field-idle-snapshot`（序章）：`partyList = [1006]`，仅 `1006` 为 `Join`。
  - `20261124-163007-auto-10-snapshot` → `163107-auto-12-snapshot`（转为主角视角）：`partyList` 变为 `[1001, 1002]`，`1001`/`1002` 由 `NotJoin → Join`，`1006` 由 `Join → NotJoin → Leave` 且参数重置（`Experience: 1650154 → 0`、`HitPoint: 3584 → 200`）。
- **数据表对应（置信度：高）**：`UnitId` 100% 对应 `Unit/DT_UnitParameter.json` 主键 `ID`：
  - `1001` 雷因、`1002` 拉斯韦尔、`1003` 菲娜、`1004` 莉德、`1005` 尼科尔、`1006` 魔人菲娜、`1007` 杰科、`1008` 樱（另有剧情友军/客座 `1013`/`1014`/`1016`）。
- **注意**：`Leave` 状态的成员（如 `1006`）在后续战斗结算和帐篷休息时仍会在后台同步获得经验与恢复（`battle-result`：`1006` `Experience: 0 → 24`；`tent-after`：`1006` `HitPoint: 200 → 272`）。判断当前可用队伍**必须以 `Party.partyList` 为准**。

### 1.2 经验与等级（置信度：高）

- **累计经验**：`CPP_MuchaGameSaveData.unitParameterList[i].Experience: int32`。
  - **快照证据**：`battle3-start` → `battle-result`：`1001`/`1002` `0 → 24`；`item-use-after` → `level-up`：`84 → 147`；`before-earth-crystal` → `boss-defeated`：`173 → 473`。
- **等级换算规则（高置信度，已精确对账）**：
  - `SaveData_UnitParameter` **不存等级整数**，由累计 `Experience` 对照 `Unit/LevelParameter/Playable/DT_UnitLevelParameter*.json`（经 `DT_UnitParameter.LevelParamId` → `DT_UnitLevelParameterList` 定位）确定。
  - 表中 `RequiredExperience` 为**从上一级升到本级的增量经验**，并非累计值；达到 $L$ 级所需总经验为 $\sum_{k=1}^{L}\text{RequiredExperience}[k]$。
  - **数据核对**：雷因（1001）与拉斯韦尔（1002）阈值为 `Lv 1 = 0`、`Lv 2 = 52`、`Lv 3 = 142`（`52+90`）、`Lv 4 = 384`（`142+242`）；魔人菲娜（1006）为 `Lv 1 = 0`、`Lv 2 = 20`、`Lv 3 = 78`（`20+58`）。
    - `battle-result`（`Exp = 24`）：`1001`/`1002` 为 Lv 1，`1006` 升为 Lv 2（与 `WBP_Btl_ResultLayout_C.resultData` 一致）。
    - `level-up`（`Exp: 111 → 147`）：`1001`/`1002` 由 Lv 2 升至 Lv 3（`111 < 142 ≤ 147`）。
    - `boss-defeated`（`Exp: 173 → 473`）：`1001`/`1002` 由 Lv 3 升至 Lv 4（`173 < 384 ≤ 473`）。
- **运行时直接读等级**：
  - `CPP_ManualViewModelManager.UnitVMManager.UnitParamVMMap[UnitId].LevelProgressionVM`（`CPP_VM_UnitLevelProgression`，父类 `CPP_VM_Progression`）下持有 `StageVM: CPP_VM_Point`（等级）与 `PointsVM: CPP_VM_Point`（经验进度），另提供只读函数 `CPP_VM_UnitParam:GetLevel()`、`CPP_UnitParameter:GetLevel()`、`CPP_BFL_Unit:GetPlayableUnitLevel(UnitId)`。
  - 因 `FfrDump` 对 `CPP_VM_Point` 触及每类 50 实例上限，`StageVM`/`PointsVM` 实体未完整导出，直接读取这两项子对象标**中置信度**（反射与结构明确，需 FfrBridge v1 真机首次顺带核验）；而通过 `SaveData.Experience` + 离线表换算为**高置信度**且无结算延迟。

### 1.3 HP 与 MP（置信度：高）

- **野外事实源（双向同步）**：
  - `CPP_MuchaGameSaveData.unitParameterList[i].HitPoint` / `MagicPoint`；
  - `CPP_ManualViewModelManager.UnitVMManager.UnitParamVMMap[UnitId]` 的 `HitPointVM.CurrentPoint` / `HitPointVM.MaxPoint`、`MagicPointVM.CurrentPoint` / `MagicPointVM.MaxPoint`。
  - **快照证据**：
    - 用恢复药给男二回血（`170103-item-use-before` → `170127-item-use-after`）：`1002` 的 `SaveData.HitPoint` 与 `HitPointVM.CurrentPoint` 同步由 `52 → 252`（`MaxPoint = 288`）。
    - 帐篷恢复（`171309-tent-confirm-yes` → `171352-tent-after`）：`1001` 由 `147/15 → 382/43`，`1002` 由 `192/10 → 336/50`，两处完全同步。
- **战斗中同步差异（高置信度，关键避坑）**：
  - 对照 `172053-limit-2`（Boss 战中）与 `172158-boss-defeated`（刚进结算）：
    - `SaveData_UnitParameter` 里的 `HitPoint`/`MagicPoint` **仍停留在战前满值**（`1001: 382/43`、`1002: 336/50`），说明战斗中不写回存档对象，要到退出结算返回野外才落盘；
    - 而 `CPP_VM_UnitParam.HitPointVM`/`MagicPointVM` **已实时同步为战斗残值**（`1001: 287/33`、`1002: 231/45`）。
  - **结论**：桥读实时 HP/MP 及上限，应以 `CPP_VM_UnitParam` 为主、`SaveData` 仅作回落。

### 1.4 能力值与永久基础加成（置信度：高）

- **计算后能力值**：位于 `CPP_VM_UnitParam.ParamPointVMMap`（`TMap<eUnitParameterType, CPP_VM_UnitParamPoint*>`），每项读 `CurrentPoint`（AP 等范围值同时读 `MaxPoint`）：

| 枚举值 | `eUnitParameterType` | 界面名称 | 雷因 Lv 1 示例（含初始装备与策略+3） | 备注 |
| ---: | --- | --- | ---: | --- |
| `1` | `HitPoint` | 生命值 | `270 / 270` | 同 `HitPointVM` |
| `2` | `MagicPoint` | 法力值 | `35 / 35` | 同 `MagicPointVM` |
| `3` | `BreakPoint` | 破防槽 | `100 / 100` | `FlagIdList = [25]` 控制显示 |
| `4` | `LimitBurstPoint` | 极限技槽 | `0 / 100` | 类型 `CPP_VM_LimitPoint`，另含 `CurrentLevel`/`MaxLevel` |
| `5` | `Attack` | 力量 | `38` | 基础 35 + `baseParameterList[5]`(+3) |
| `6` | `Defence` | 体力 | `25` | 基础 25 |
| `7` | `Intelligence` | 魔力 | `16` | 基础 16 |
| `8` | `Mind` | 精神 | `28` | 基础 28 |
| `9` | `Agility` | 速度 | `7` | 基础 7 |
| `11` | `EquipCost` | AP（技能装备容量） | `0 / 100` | `CurrentPoint` 已用 / `MaxPoint` 上限 |
| `12` | `PhysicalAttack` | 物理攻击力 | `59` | 力量 38 + 武器 `10100`(+21) |
| `13` | `PhysicalDefense` | 物理防御力 | `30` | 体力 25 + 防具 `11000`(+5) |
| `14` | `MagicalAttack` | 魔法攻击力 | `25` | 魔力 16 + 武器 `10100`(+9) |
| `15` | `MagicalDefense` | 魔法防御力 | `31` | 精神 28 + 防具 `11000`(+3) |
| `16` | `Divine` | 神圣力 | `28` | 等于精神 28 |

  - 枚举值 `10`（`Level`）、`17`（`Role`）、`18`（`SkillDamageType`）、`19`（`AP`）的 `CurrentPoint`/`MaxPoint` 实测恒为 `-1`，不可使用。
- **永久性基础属性加成**：`SaveData_UnitParameter.baseParameterList: TArray<int32>`（长度 20，下标严格对应 `eUnitParameterType` 枚举值 `0..19`）。
  - **快照证据**：`163724-choice-strategy`（事件 `C01_006_01_B` 选了“攻撃上昇を選択”）→ `163737-auto-25`：`1001` 的 `baseParameterList[5]` 由 `0 → 3`，同时 `CPP_VM_UnitParam` 的 `Attack` 由 `35 → 38`、`PhysicalAttack` 由 `56 → 59`。
- **升级刷新时机注意（高置信度）**：在 `170702-level-up`、`170752-learn-skill`、`170831-learn-skill-2` 结算演出期间，`CPP_VM_UnitParam` 仍保持升级前的上限与能力值（雷因 `MaxHP = 326, Atk = 42`），直到关闭结算回到野外（`171013-before-earth-crystal`）才刷新为 Lv 3 数值（`MaxHP = 382, Atk = 46`）。

---

## 2. 招式和技能

### 2.1 已学会技能的真实存储机制（置信度：高）

对比 `165044-skill-list`、`170702-level-up`、`170752-learn-skill`、`170831-learn-skill-2` 与战后野外 `171013-before-earth-crystal`，证实了草稿未看清的分层机制：

1. **按等级习得的招式/魔法不存入 `SaveData_UnitParameter.activeSkillIdList`**：
   - 在 `skill-list` 中，雷因 Lv 1 在战斗“招式”子列表里显示 1 个招式“火炎”（`WBP_BtlCmd_ActionList_C` 与 `BP_BtlCmdList_Belong_C` 的 `ActionKeyList = [{Category: Skill, ID: 220010}]`，`CPP_BtlCmdFocusDelVal.actionId = 220010`），但此时 `SaveData_UnitParameter.activeSkillIdList` 仅有 `[401010]`（普攻“战斗”）。
   - 在 `learn-skill`（雷因 Lv 2→3 学会“撞击” `401080`）与 `learn-skill-2`（拉斯韦尔 Lv 2→3 学会“清霜” `402130`）之后，直至 `before-earth-crystal` 和 `boss-defeated`，两人的 `SaveData_UnitParameter.activeSkillIdList` 仍保持 `[401010]` 与 `[402010]`，并未追加 `401080` 或 `402130`。
   - 对照 `DT_UnitLevelParameterRain` / `DT_UnitLevelParameterLasswell`：`AddSkills` 在 Lv 1 列出 `[401040 无尽驱动, 220010 火炎]` / `[402040 歼月, 220050 冰结]`，Lv 3 列出 `[401080 撞击]` / `[402130 清霜]`，与 `learn-skill`/`learn-skill-2` 弹窗完全一致。
   - **结论**：角色已学会的等级技能由当前等级从 `DT_UnitLevelParameter*` 动态派生；运行时对应的完整主动技能列表通过只读函数 `CPP_UnitParameter:GetActiveSkillList(out OutList)`、`CPP_BtlUnitParameter:GetActiveSkillListWithVision(out OutList)` 或 `CPP_BFL_Unit:GetAllAddSkillIdByLevel(out, UnitId, Level)` 获取。
2. **`SaveData_UnitParameter` 里的四个技能相关字段**：
   - `activeSkillIdList` / `passiveSkillIdList` / `commandSkillIdList`：对应 `CPP_BFL_SaveDataUnit:GetMaster*SkillList`，存放**基础普攻、幻兵精通（Master）技能、以及剧情临时授予技能**（证据：序章 `field-idle` 中 `1006` 魔人菲娜被剧情授予 7 个技能 `[408010, 220070, 220030, 408050, 431120, 408030, 220110]`，离队后清空为 `[]`）。
   - `equipSkillList: TArray<int32>`（固定 8 槽，空位 `-1`）及镜像 `CPP_VM_UnitParam.EquippedSkillsVM.ActionIdList`（8 槽）：消耗 AP 装备的能力槽。当前 98 份快照均全为 `-1`，尚无装备能力后的真机对照（**置信度：中**）。
   - `btlCmdData`（`SaveData_UnitBtlCmdDataBundle`：`CommandId` + `cmdDataList`）：**不是技能表，而是战斗上次选中指令的光标记忆**（证据：`401010` 普攻 → 战斗选过招式后 `chest2-before` 变为 `1`（指令 1 = 招式）；序章 `1006` 放极限技后 `field-idle` 变为 `10`（指令 10 = 极限技））。

### 2.2 升级习得新技能的快照实证（置信度：高）

| 快照 | 关键对象与属性变化 | 对应数据表 |
| --- | --- | --- |
| `165044-skill-list` | `BP_BtlCmdList_Belong_C.ActionKeyList = [{Skill, 220010}]`；`CPP_BtlCmdFocusDelVal.actionId = 220010` | `DT_SkillData[220010]` = 火炎（`belongCommandList=[1]`, MP 5） |
| `170752-learn-skill` | `WBP_BtlResultPage_LevelUpWindow_C`：`IsShouldDisplay=true`、`m_UnitDetailIndex=0`、`m_UnitList` 含 `1001` 和 `1002`；`WBP_Btl_Result_UnitLevenUp_DetailWindow_C`：`m_UnitData.UnitId=1001`、`m_AfterHP=382`、`m_AfterMP=43`、`m_UnitData.addSkillList=[[401080, 3]]`；`WBP_Btl_Result_UnitAbilityList_C.m_NewAbilityList=[401080]` | `DT_SkillData[401080]` = 撞击（雷因 Lv 3 习得，`belongCommandList=[1]`, MP 5） |
| `170831-learn-skill-2` | `WBP_BtlResultPage_LevelUpWindow_C.m_UnitDetailIndex` 由 `0 → 1`；`WBP_Btl_Result_UnitLevenUp_DetailWindow_C`：`m_UnitData.UnitId=1002`、`m_AfterHP=336`、`m_AfterMP=50`、`m_UnitData.addSkillList=[[402130, 3]]`；`WBP_Btl_Result_UnitAbilityList_C.m_NewAbilityList=[402130]` | `DT_SkillData[402130]` = 清霜（拉斯韦尔 Lv 3 习得，`belongCommandList=[1]`, MP 5） |

---

## 3. 成长页面（`growth-page`）

- **页面定位（置信度：高）**：`165554-growth-page` 并非主菜单养成界面，而是**战斗结算界面 `WBP_Btl_ResultLayout_C` 的第 2 页——经验与成长页 `WBP_BtlResultPage_GrowthWindow_C`**。
- **页面切换证据**（`165453-battle-result` → `165554-growth-page`）：
  - `WBP_BtlResultPage_Layout_C.Class` 固定包含 2 页：`[0] WBP_BtlResultPage_RewardWindow_C`（奖励页）、`[1] WBP_BtlResultPage_GrowthWindow_C`（成长页）。
  - `WBP_BtlResultPage_Layout_C.CurrentPageIndex` 由 `0 → 1`。
  - `WBP_Btl_ResultPage_Growth_UnitList_C` 动画计数 `m_UnitSlideInAnimationFinishCount` 与 `m_UnitExpBarFinishCount` 均由 `0 → 2`（2 名参战成员的经验条播放完成）。
- **页面展示内容与底层数据（置信度：高）**：
  - **全局结算 ViewModel**：`CPP_VM_BattleResult`（单例，位于 `MVVMGameSubsystem.MVVMViewModelCollectionObject.CPP_VM_BattleResult`），其属性 `BtlResultData` 聚合本次战斗全部结算数据：
    - 收益汇总：`beforeMoney`、`baseMoney`、`bonusMoney`、`totalMoney`、`baseExperience`、`bonusExperience`、`totalExperience`、`baseAp`、`bonusAp`、`totalAp`、`isBonus`、`isBonusAchieveds`；
    - 敌方与掉落：`enemyDataList: TArray<int32>`、`itemDataList: TArray<{ItemId, Num, acquireLimitOverNum}>`（如 `boss-defeated`：`totalMoney=750, totalExperience=300, totalAp=45, enemyDataList=[401], itemDataList=[{ItemId:4, Num:1}]`）；
    - 角色成长：`unitDataList` 每项含 `UnitId`、`beforeBattleLevel`、`afterBattleLevel`、`beforeBattleExp`、`afterBattleExp`、`ap`、`defeatUnitNum`、`IsDead`、`isBattle`、`isPartyLeaveUnit`、`addSkillList`、`extraAddSkillList`、`increasedParameters`（注：`CPP_VM_BattleResult.BtlResultData.unitDataList` 只含参战成员，而 `WBP_Btl_ResultLayout_C.resultData.unitDataList` 额外包含 `isPartyLeaveUnit=true` 的离队成员 `1006`）；
    - 幻兵成长：`visionDataList`（当前第 1 章开头为空，幻兵系统解锁前无真机对照，**置信度：低**）。
  - **经验条子控件**：`WBP_UnitGrowth_Exp_C`（每名参战成员成对出现，`IsUnit=true` 为角色、`IsUnit=false` 为所装备幻兵），包含 `ID`、`BeforeLevel`、`AfterLevel`、`BeforeEXP`、`AfterEXP`、`DisplayLevel`、`IsLevelUpStaging`。
  - **升级详情子窗**：若存在 `afterBattleLevel > beforeBattleLevel`，按序弹出 `WBP_BtlResultPage_LevelUpWindow_C` → `WBP_Btl_Result_UnitLevenUp_DetailWindow_C`（类名源码拼作 `LevenUp`）与 `WBP_Btl_Result_UnitAbilityList_C`。

---

## 4. 物品、装备、金币与宝箱

### 4.1 道具库存与获取/消耗（置信度：高）

- **唯一事实源**：`CPP_MuchaGameSaveData.inventory`（`TArray<SaveData_Inventory>`，每项 `{ ItemId: int32, Num: int32 }`）。
  - `ItemId` 对应 `Item/` 下 6 张数据表共享的全局唯一编号（消耗品 `1..602`、装备 `10100..12103`、圣遗物 `30000..30018` 等）。
  - **关键规则**：**角色已穿戴的装备也计入 `inventory` 总数**（证据：`got-equipment` 中两人共装备 2 件 `11000` 皮甲、2 件 `12000` 体力耳环、各 1 把 `10100`/`10200` 武器，`inventory` 中全部列明对应数量；闲置件数 = `inventory.Num` − 已装备人数）。
- **成对快照证据**：
  - **消耗品使用**：`170103-item-use-before` → `170127-item-use-after`：`ItemId=1`（恢复药）`Num: 6 → 5`。
  - **帐篷消耗**：`171309-tent-confirm-yes` → `171352-tent-after`：`ItemId=500`（帐篷）`Num: 3 → 2`。
  - **宝箱 1（map 3001）**：`164552-chest-before` → `164653-chest-after`（期间开 2 箱）：`ItemId=1`（恢复药）`4 → 5`、`ItemId=3`（以太）`2 → 3`，对应 `CDT_MapGimmickTreasureBox` 的 `Dng_01Gra_43_0` / `Dng_01Gra_43_1`。
  - **宝箱 2（map 3000）**：`165748-chest2-before` → `165832-chest2-after`：`ItemId=4`（不死鸟之尾）`2 → 3`，对应 `Dng_01Gra_40_1`；且宝箱 Actor `BP_MapGimmick_TreasureBox_C_0.m_IsOpen` 由 `false → true`。
  - **获得装备（宝箱开出武器）**：`170127-item-use-after` → `170210-got-equipment`：`inventory.len` 由 `13 → 14`，新增 `{ItemId: 10201, Num: 1}`（`10201` = 野太刀，对应 `Dng_01Gra_40_2`），弹窗提示 `WBP_DialogBox`“获得了道具！”。
  - **战后掉落**：`171013-before-earth-crystal` → `172158-boss-defeated`：`ItemId=4` `3 → 4`，与 `CPP_VM_BattleResult.BtlResultData.itemDataList` 吻合。
- **避免误读 `inventoryProcedure`**：`CPP_MuchaGameSaveData.inventoryProcedure.itemDatas` 仅按最近入手顺序（MRU）记录变更历史（且 `Num` 常滞留为入手前旧值），**不可用于读取当前库存**。

### 4.2 角色当前装备（置信度：高）

- **持久化字段**：`SaveData_UnitParameter` 下的 `EquipWeaponId: int32`、`EquipArmorId: int32`、`EquipAccessoryIdList: TArray<int32>`（只存已装备项，空槽可能省略）、`equipVisionId: int32`（空为 `-1`）。
- **固定 5 槽 ViewModel 镜像**：`CPP_VM_UnitParam.EquippedItemsVM.ActionIdList`（长度恒为 5，与 `EquipSlotTypeList` 对应 `[Weapon, Armor, Accessory, Accessory, Vision]`，空槽填 `-1`，如雷因 `[10100, 11000, 12000, -1, -1]`）。
- **快照证据**：序章 `163007-auto-10` → `163107-auto-12` 魔人菲娜离队卸装时，`EquipWeaponId` `10699 → 10600`、`EquipArmorId` `11301 → -1`、`EquipAccessoryIdList` `[12029] → []`。
- **缺口**：`got-equipment` 仅拍到了获得“野太刀” `10201` 入包，未在菜单中实际换装，缺手动换装前后对照快照。

### 4.3 金币与商店（置信度：高 / 交易变更缺对照）

- **金币字段**：`CPP_MuchaGameSaveData.Player`（`SaveData_Player`）下的 `currentMoney: int32`（配套 `earnedMoney`、`spentMoney`）。
- **快照证据**：`title` `0` → `cutscene` `1000` → `battle3-start`→`battle-result` `1000 → 1040`（`+40`）→ `chest2-before` `1100` → `quest-list` `1140` → `level-up` `1240` → `before-earth-crystal` `1330` → `boss-defeated` `1330 → 2080`（`+750`，同期瞬态对象 `CPP_SaveDataPlayerMoneyDelegateVal` 记录 `beforeMoney=1330, AfterMoney=2080`）。
- **商店实况**：`rest-shop`、`shop-buy`、`shop-sell`、`shop-sell-2` 及 `auto-27..31` 期间用户仅浏览与翻页，未发生实际买卖，因此 `currentMoney` 维持 `1000`、`spentMoney = 0`；但访问商店 2 后 `CPP_MuchaGameSaveData.shop` 已登记限购项 `[{ShopID: 2, ItemId: 24, MaxOrderNum: 3, CurrentBuyNum: 0}]`（烟雾弹），`CPP_VM_Shop_TradeWindow` 也证实切换了 `IsBuy` 与 `CurrentItemList`。**缺少实际买入/卖出成交前后快照**来实证 `spentMoney` 与 `CurrentBuyNum` 递增。

### 4.4 宝箱开启状态持久化（置信度：中-高）

- 持久化在 `CPP_MuchaGameSaveData.Map.mapGimmick: TMap<int32, SaveData_MapGimmick>`（键为 `mapId`）→ `treasureBox: SaveData_TreasureBoxList` → `treasureBoxList: TArray<SaveData_TreasureBox>`（每项含 `ID`, `UniqueID`, `IsOpen`, `isDisableTrap`, `IsUnlock`）；同时 `CPP_BFL_SaveDataMap` 提供只读查询函数 `IsOpenTreasureBox`。
- **快照证据**：`chest-before` → `chest-after` 时 `Map.mapGimmick` 键集合由 `[2014, 3001]` 增至 `[2014, 3001, 3000]`；`chest2-before` → `chest2-after` 时场景宝箱 Actor `BP_MapGimmick_TreasureBox_C_0.m_IsOpen` 由 `false → true`（因 `FfrDump` 导出 `MapProperty<int32, Struct>` 时未展开内层结构体，`SaveData_TreasureBox.IsOpen` 的直接读写需在桥 v1 验证，标中-高）。

---

## 5. 剧情进度、章节与事件旗标

### 5.1 主线进度与章节（置信度：高）

- **存储位置**：`CPP_MuchaGameSaveData.Progress.Progress`（`TArray<int32>`，固定长度 200）。
  - `Progress[0]` 即全表通用条件表达式里的主线进度计数器 `{progress:0}`。
  - `Progress[1..31]` 在全部 98 份快照中均为 `0`（支线/子进度尚未使用）。
- **快照演进与 `DT_MainScenario` / `DT_ActivityData` 完整对齐**：

| 快照阶段 | `Progress[0]` | 对应事件（`CDT_TalkEventData_Prod`） | `DT_MainScenario` 进度段 / 目标 | `CPP_ScenarioSubsystem.m_CurrentDestinationMapIdList` |
| --- | ---: | --- | --- | --- |
| `title`（占位） | `0` | `DT_InitGameSaveData.Setup` | — | `[]` |
| `cutscene` | `1000` | `Opening` / `C01_001_01` | 序章起点（`DT_ActivityData` `1000`） | `[]` |
| `battle-start` .. `auto-09` | `1005` | `C01_001A` | 序章战斗 | `[]` |
| `auto-10` .. `auto-16` | `1010` | `C01_002_01` | 序章切主角视角 | `[]` |
| `auto-17` .. `choice-strategy` | `1020` | `C01_003_02_*` / `C01_005_01` | 第 1 章起点：`>=1020` 走出格兰谢尔特城（目标图 `3001`） | `[3001]` |
| `auto-25` .. `before-earth-crystal` | `1030` | `C01_006_01_B_Choice_After` | `>=1030` 调查土之神殿（目标图 `3001`） | `[3001]` |
| `teleport-ui` .. `boss-defeated` | `1040` | `C01_008_01` / `C01_008_01_B` | 仍命中 `>=1030` 调查土之神殿（至 `1050` 才切米特拉镇） | `[3001]` |

  - **时间节点细节**：用户标记在 `01:35:43 JST`（`auto-21`/`auto-22`，地图 `Event_00Eve_86_PL`）报“进入第一章”对应第 1 章标题卡过场动画；数据层的 `Progress[0]` 已于离开城前的 `auto-17` 推至 `1020`，与 `DT_ActivityData` 第 1 章起点 `1020` 完全吻合。
  - **`boss-defeated` 仍为 `1040` 的原因**：该快照抓拍在击败 Boss 刚进结算的瞬间，战后触发的事件 `C01_010_01`（将 `Progress[0]` 推至 `1050`“前往米特拉镇”）尚未执行。
- **章节判定规则（高置信度）**：与 `docs/data-tables.md` 完全一致——`Progress[0] < 1020` 为序章（`0`），`Progress[0] >= 1020` 按千位段 `Math.floor(Progress[0] / 1000)` 映射 `DT_ChapterData.ChapterNo`（`1..8` 章、`9` 通关；`save-ui` 实证 `MainProgress = 1030` → `ViewData.ChapterName = "第1章探究的开端"`）。运行时也可由 `CPP_ScenarioSubsystem:GetCurrentChapter()` 直接获取。

### 5.2 事件旗标（置信度：高）

- **存储位置**：`CPP_MuchaGameSaveData.Progress.eventFlag`（`TArray<bool>`，固定长度 **2048**），0-based 下标直接对应数据表条件 `{flag:<id>}` 与事件表 `OnFlagList` / `OffFlagList`。
- **多组快照与事件表精确互证（前 32 位）**：
  - `title`（Setup）：`[13..20, 24..31]` 为 `true`（与 `DT_InitGameSaveData.Setup.OnFlagList` 前 32 位完全吻合）。
  - `cutscene`（NewGame）：前 32 位全 `false`（初始开启的 `1160..1182` 在 32 位之后）。
  - `battle-start`：`[1, 8, 20]` 置 `true`（事件 `C01_001A` `OnFlag=[105, 8, 1]`）。
  - `battle-limit` → `battle-summon`：`30` 置 `true`，随后 `23` 置 `true`（`DT_UnitBattleCmd` 明确标注 `1006` 召唤指令解锁条件为 `EventFlag 23`）。
  - `auto-10`：`8` 置 `false`，`14, 15, 19` 置 `true`（事件 `C01_002_01` `OnFlag=[14, 15, 19, 23], OffFlag=[8]`，完全吻合）。
  - `choice-strategy` → `auto-25`：`20` 置 `false`、`22` 置 `true`（事件 `C01_006_01_B_Choice_After` `OnFlag=[22, 1225, 1226, 33], OffFlag=[20]`，完全吻合）。
  - `boss-dialog`：`25` 置 `true`（对应 `CPP_VM_UnitParam.ParamPointVMMap[3]` BreakPoint 的 `FlagIdList = [25]`，即破防槽教程解锁旗标）。
- **截断说明**：快照仅导出 `eventFlag[0..31]`，高位旗标（`32..2047`，如 `105`、`1160`、`1225`）在桥 Lua 中可直接遍历完整 2048 长度或调用 `CPP_BFL_SaveDataProgress:GetFlag(index)` 读取。

### 5.3 任务追踪（置信度：中）

- `CPP_QuestSubsystem`（`GameInstanceSubsystem`，98/98 存在）：`MainQuestId` 在 `title` 为 `0`，进游戏后为 `1001`（`QuestDataMap` 含 9 条主线任务 `1001..1009`）；推进至 `1030`（`auto-25`）时 `QuestNoticeDataList` 瞬态出现 `{QuestId: 1001, Progression: EQuestProgression::Progress}`。
- `SaveData.quest.PreferredQuestId`（全程 `-1`）、`SaveData.questBoardList`（8 条 `1000..1007`，均为 `EQuest_State::None`）与 `SaveData.request.DataMap`（空）因尚未解锁支线与委托，暂无变更快照对照。

---

## 6. 存档数据结构与存档槽（只读）

### 6.1 运行态管理器结构（置信度：高）

- `CPP_MuchaSaveDataManager`（`GameInstanceSubsystem`，98/98 存在）：
  - `m_pGameSaveData: CPP_MuchaGameSaveData*`：唯一运行态存档对象（共 29 个顶层结构体/数组字段，含 `Player`、`Party`、`unitParameterList`、`unitList`、`unitGuest`、`inventory`、`inventoryProcedure`、`visionList`、`artifactList`、`Progress`、`Map`、`shop`、`quest`、`questBoardList`、`request` 等）。
  - `m_pOptionSaveData: CPP_MuchaOptionSaveData*`：选项存档，含 `LastSavedDataIndex: int32`（记录最近一次存盘的槽位号）。
  - `GameSaveDataSummaryMap: TMap<int32, MuchaGameSaveDataSummary>`：全部存档槽的轻量摘要表。
  - `tempGameDifficulty: eGameDifficulty`、`bOptionLoadFailed: bool`。
- **标题画面占位警示**：未进游戏前（`title-snapshot`）`m_pGameSaveData` 已经非空，但装的是 `DT_InitGameSaveData.Setup` 测试占位数据（`Map.mapId = -1`、`Progress[0] = 0`、`PlayTime = 0.0`），不是真实存档；判断有效对局需检查 `Progress[0] > 0`（且当前 World 非标题图）。

### 6.2 存档槽与自动存档编号（置信度：高）

结合 `title`、`savepoint`、`save-ui`、`auto-35`、`auto-36`、`boss-defeated` 证实槽位体系：

- **总槽位数 11 个，键为 `0..10`**（`CPP_MuchaSaveDataManager.GameSaveDataSummaryMap.len = 11`）：
  - **槽 `0` = 自动存档槽**（不出现在手动保存界面的列表里）；
  - **槽 `1..10` = 手动存档槽**（界面显示 `Data 01` .. `Data 10`，`WBP_SaveDataList_C.SlotIndexList = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]`，10 个 `BP_SaveDataListItem_C` 映射 `ItemIndex 0..9 → SlotIndex 1..10`）。
- **快照证据**：
  - `LastSavedDataIndex` 变化：`title` 为 `-1` → 过图自动存档后 `savepoint` 变为 `0` → 用户在 `save-ui` 手动保存到槽 `1` 后变为 `1` → 后续推进再次触发自动存档后 `boss-defeated` 变回 `0`。
  - `save-ui` 中 `VM_SaveData_C` 实证：
    - `SlotIndex = 1`（`bIsAutoSave = false`）：`SaveDataSummary.bExists = true`、`LoadStatus = ESaveLoadStatus::Success (0)`、`CurrentMapId = 3001`、`MainProgress = 1030`、`PlayLevel = 1`、`PlayTimeSec = 1829.0`、`PartyUnitIds = [1001, 1002]`、`ViewData.ChapterName = "第1章探究的开端"`、`PlaceName = "土之神殿"`。
    - `SlotIndex = 2, 3, 4`：`bExists = false`、`LoadStatus = ESaveLoadStatus::FileReadFailed (7)`、`bShouldDisplaySaveData = false`（空槽；槽 `5..10` 因列表虚拟化未实例化 `VM_SaveData_C`）。
- **只读查询接口（给固定空槽位校验用）**：`CPP_MuchaSaveDataManager` 提供纯读方法 `GetSaveSlotNum()`、`IsValidGameSaveData(SlotIndex)`、`GetValidSaveSlotIndexes(out)`、`FindGameSaveDataSummary(SlotIndex, out)`、`GetLatestSaveSlotIndex()`。建议固定空槽位选在末尾槽（如槽 `10`），避开槽 `0`（自动存档）与用户常用前槽。

### 6.3 尚缺的真机快照

1. **读档界面（Load UI）快照**：直接拍到 `SlotIndex = 0, bIsAutoSave = true` 的 `VM_SaveData_C` 实例，补全自动存档摘要 UI 核对。
2. **手动更换装备前后成对快照**：验证换装后 `SaveData_UnitParameter.EquipWeaponId` 与 `CPP_VM_UnitParam` 攻防数值的实时同步。
3. **商店实际成交（买/卖）前后成对快照**：验证 `Player.currentMoney`/`spentMoney`、`inventory` 与 `SaveData.shop[].CurrentBuyNum` 联动。
4. **幻兵（Vision）与 AP 能力装备解锁后的快照**：验证 `visionList`、`equipVisionId`、`equipSkillList` 与 `WBP_UnitGrowth_Vision_C`。
5. **Boss 战后回归野外快照（`Progress[0] >= 1050`）**：验证 `1040 → 1050` 推进及 `CPP_ScenarioSubsystem.m_CurrentMainScenarioMapObjectList`（含具体目标点 `Point` / `TransitionPoint`）的填充形态。

---

## 7. FfrBridge v1 建议字段

沿用 `mod/FfrBridge/Scripts/ffrbridge/state.lua` 的只读原则（每 500 ms 在游戏线程 `pcall` + `IsValid()` 采样，单项出错记入 `errors` 不中断其余字段）。所有 `GameInstanceSubsystem` 在全部 98 份快照中均 100% 存活，可用 `FindFirstOf` 查找并在换图后验证有效性。

### 7.1 单例实例定位方式

| 简称 | 查找方式（UE4SS Lua） | 备注 |
| --- | --- | --- |
| `saveMgr` | `FindFirstOf("CPP_MuchaSaveDataManager")` | `GameInstanceSubsystem`，取 `saveData = saveMgr.m_pGameSaveData`、`optData = saveMgr.m_pOptionSaveData` |
| `uvmMgr` | `FindFirstOf("CPP_ManualViewModelManager").UnitVMManager`（或直接 `FindFirstOf("CPP_UnitVMManager")`） | 全局唯一，按 `UnitId` 查 `uvmMgr.UnitParamVMMap`（或调 `uvmMgr:GetUnitParamVM(unitId)`） |
| `scenSub` | `FindFirstOf("CPP_ScenarioSubsystem")` | `GameInstanceSubsystem` |
| `questSub` | `FindFirstOf("CPP_QuestSubsystem")` | `GameInstanceSubsystem` |
| `btlUiSub` | `FindFirstOf("CPP_UI_BattleSubsystem")` | `WorldSubsystem`，换图会重建，切勿跨图长期缓存 |
| `btlResVm` | `FindFirstOf("CPP_VM_BattleResult")` | 仅在 `btlUiSub.ResultWidget` 有效时读取 |

### 7.2 建议输出字段（共 14 个）

| # | `data` 字段名 | 类型 | 数据来源（对象路径与属性） | 采样频率与说明 |
| ---: | --- | --- | --- | --- |
| 1 | `save_ready` | `boolean` | `saveData.Progress.Progress[1] > 0`（Lua 1-based） | **每 0.5 s**；区分标题 `Setup` 占位数据与真实对局，为 `false` 时下游忽略队伍/进度字段 |
| 2 | `money` | `number \| null` | `saveData.Player.currentMoney` | **每 0.5 s** |
| 3 | `play_time` | `number \| null` | `saveData.Player.PlayTime`（保留 1 位小数） | **低频（每 2 s / 4 tick）**或随 tick 输出（单次浮点读取开销可忽略） |
| 4 | `party_ids` | `number[]` | `saveData.Party.partyList` | **每 0.5 s**；数组长度 ≤ 4，给出编队顺序 |
| 5 | `guest_unit_id` | `number \| null` | `saveData.Party.guestUnitId`（`-1` 输出 `null`） | **每 0.5 s** |
| 6 | `party` | `PartyMember[]` | 按 `party_ids` 遍历：从 `uvmMgr:GetUnitParamVM(uid)` 读 `HitPointVM.CurrentPoint/MaxPoint`、`MagicPointVM.CurrentPoint/MaxPoint`、`LevelProgressionVM.StageVM.CurrentPoint`（回落 `vm:GetLevel()`）；从 `saveData.unitParameterList` 对应项读 `Experience`、`EquipWeaponId`、`EquipArmorId`、`EquipAccessoryIdList`、`equipVisionId`、`JoinStatus` | **每 0.5 s**；仅遍历当前在队成员（≤ 4 人），确保战斗中与野外 HP/MP 均实时；战斗结算瞬间 `max_hp`/`level` 可能滞后 1 步，宿主机可结合 `exp` 查表复算 |
| 7 | `party_stats` | `Record<string, UnitStats> \| null` | 对 `party_ids` 读 `vm.ParamPointVMMap` 键 `5..9, 11..16` 的 `CurrentPoint`（及 `11` 的 `MaxPoint`）：`attack`, `defence`, `intelligence`, `mind`, `agility`, `equip_cost`, `max_equip_cost`, `physical_attack`, `physical_defense`, `magical_attack`, `magical_defense`, `divine` | **低频（每 2 s 或仅当队伍 `exp`/装备变化时重算缓存）**；每成员 11 次子对象属性读取，做简单脏检查即可几乎零开销 |
| 8 | `inventory` | `Array<{item_id: number, num: number}>` | `saveData.inventory`（遍历全量 `GetArrayNum()`，**不截断 32 项**） | **低频（每 2 s 或长度/金钱变化时刷新）**；含已穿戴装备总数，跳过 `inventoryProcedure` |
| 9 | `progress` | `number \| null` | `saveData.Progress.Progress[1]`（C++ 0-based `Progress[0]`） | **每 0.5 s**；对应 `{progress:0}` |
| 10 | `chapter` | `number \| null` | `progress == nil or progress == 0` → `null`；`progress < 1020` → `0`（序章）；否则 `math.floor(progress / 1000)`（可与 `scenSub:GetCurrentChapter()` 互验） | **每 0.5 s** |
| 11 | `destination_map_ids` | `number[]` | `scenSub.m_CurrentDestinationMapIdList` | **每 0.5 s**；数组极短（0–3 项） |
| 12 | `event_flags_true` | `number[]` | 遍历 `saveData.Progress.eventFlag` 全部 2048 个布尔位，仅收集值为 `true` 的 0-based 编号（如 `[14, 15, 22, 105, ...]`） | **低频（每 2 s / 4 tick）**；稀疏整数列表显著减小 JSON 体积，同时完整保留 32 位以上旗标 |
| 13 | `main_quest_id` | `number \| null` | `questSub.MainQuestId` | **每 0.5 s** |
| 14 | `last_saved_slot` | `number \| null` | `optData.LastSavedDataIndex`（`-1` 输出 `null`，`0` 为自动存档，`1..10` 为手动槽） | **每 0.5 s**；用于存档动作完成证据（结合 `save_time` / `SavedDateTime.Ticks` 更佳） |

- **可选扩展（战斗结算与存档槽查询）**：
  - 战斗结算详情可在 `btlUiSub.ResultWidget` 有效时附带读取 `btlResVm.BtlResultData`（`totalMoney`, `totalExperience`, `itemDataList`, `unitDataList`），该部分与战斗主题（10a）共享，实现时统一归口即可。
  - **存档槽占用清单**（槽 `0..10` 的 `bExists`、`MainProgress`、`CurrentMapId`、`DateTime`）变化极低，不建议塞进 500 ms `state.json`；建议在 FfrBridge v1 中增加一个只读命令 `save_slots`（调用 `saveMgr:FindGameSaveDataSummary(0..10)` 或读 `GameSaveDataSummaryMap`），由宿主机启动或存档前按需查询，核验固定空槽位未被占用。
