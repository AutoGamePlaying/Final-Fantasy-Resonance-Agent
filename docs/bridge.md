# 文件桥模组 `FfrBridge`（v1）

`mod/FfrBridge/` 是游戏内 UE4SS Lua 模组，负责把《FINAL FANTASY RESONANCE》的运行时状态提供给本机的 AGP 游戏模块（`ffr-module.exe`）。模组**默认只读**：不修改游戏数据、内存数值或存档；R4 模拟输入命令 `press` 默认关闭，仅在桥目录存在由 AGP 显式创建的 `input.enabled` 开关文件时才允许通过引擎 Enhanced Input 注入白名单输入（详见 [`docs/input.md`](input.md)）。

文件桥传输协议（文件命名、原子写入、`state.json` / `command.json` / `ack.json` 外层格式和重试规则）见 [`../agp/docs/file-bridge.md`](../../agp/docs/file-bridge.md)，本文记录共鸣模组的桥目录、启用方法、顶层 v0 兼容字段以及 v1 分主题字段与按需命令。

## 1. 桥目录

`<游戏 Win64 目录>/ue4ss/ffr-out/bridge/`

- 目录由 AGP 一侧创建，模组不建目录。
- 目录不存在时，模组什么都不写，也不输出错误日志；AGP 创建目录后下一个 500 毫秒周期自动开始写 `state.json`。
- 同一种运行期错误 60 秒内只在 `UE4SS.log` 记一次，标签为 `[FfrBridge]`。

## 2. 启用和停用

在游戏未运行时，从开发机同步并启用：

```sh
node src/host/cli/mod-sync.ts FfrBridge
```

停用：

```sh
node src/host/cli/mod-sync.ts FfrBridge --disable
```

查看模组日志：

```sh
node src/host/cli/ue4ss-log.ts --tag "[FfrBridge]"
```

同步完成后重启游戏生效。

## 3. `state.json` 顶层字段（保留 v0 兼容）

v1 在保留全部 v0 顶层字段（兼容 `ffr-module` 现有 `game_status`）的基础上，把 `bridge_version` 升级为 `"1"`，并挂载四个主题子对象 `field`、`battle`、`ui`、`party`。某个主题采集抛错时只在 `errors` 中记录一条错误并把该主题输出为 `null`，不影响其余主题。

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `world` | `string \| null` | 当前 `World` 对象的完整名（`GetFullName()`），取不到写 `null` |
| `map` | `string \| null` | 从 `world` 提取的地图短名（如 `World /Game/Maps/Field01.Field01` → `Field01`），取不到写 `null` |
| `player_controller_class` | `string \| null` | `PlayerController` 的类完整名，取不到写 `null` |
| `pawn_class` | `string \| null` | `PlayerController.Pawn` 的类完整名，取不到写 `null` |
| `game_mode_class` | `string \| null` | `GameModeBase` 的类完整名，取不到写 `null` |
| `game_instance_class` | `string \| null` | `GameInstance` 的类完整名，取不到写 `null` |
| `pawn_location` | `{ x: number, y: number, z: number } \| null` | `Pawn:K2_GetActorLocation()` 坐标，各分量保留 1 位小数；取不到或出错写 `null` |
| `field` | `object \| null` | 野外、地图、移动与交互状态（见 §4.1） |
| `battle` | `object \| null` | 战斗状态（见 §4.2，未实现前或非战斗期为 `null`） |
| `ui` | `object \| null` | 对话、选项、菜单、商店、教学、存档界面状态（见 §4.3） |
| `party` | `object \| null` | 队伍、成长、物品与进度摘要（见 §4.4） |
| `errors` | `string[]` | 本次采样中各项读取失败、主题异常或对象缺失的错误说明；某一项出错不影响其余字段 |
| `bridge_version` | `"1"` | 共鸣 `data` 结构版本号，v1 固定为字符串 `"1"` |

## 4. v1 字段

四个主题模块位于 `mod/FfrBridge/Scripts/ffrbridge/topics/{field,battle,ui,party}.lua`，统一通过 `state.lua` 提供的 `ctx`（`find`、`findAll`、`err`、`cache`、`clockMs`、`tick`）采样，在游戏线程内以 `pcall` 隔离运行。

### 4.1 `field` 主题（`topics/field.lua`，已实现）

依据 [`docs/state-map/field.md`](state-map/field.md) §8 实现全部 21 个野外/地图字段（16 个高频 + 5 个低频）及镜头旋转辅助字段。

#### 高频字段（每 500 ms 采样）

| 字段 | 类型 | 来源对象与属性 / Getter | 数据表对应与说明 |
| --- | --- | --- | --- |
| `field.map_id` | `integer \| null` | `CPP_MiniMapSubsystem.PropertyKey.mapId`（兜底 `CPP_MapManager:GetMapId()`） | 对应 `CDT_MapData_Prod.ID`；标题图为 `-1` |
| `field.area_id` | `integer \| null` | `CPP_MiniMapSubsystem.AreaManager.CurrentAreaId`（兜底 `CPP_MapManager:GetMapAreaId()`） | 对应 `CPP_Map_NavMeshBoundsVolume.m_AreaId` |
| `field.is_loading` | `boolean \| null` | `CPP_GameWidgetManagerSubsystem.m_LoadScrManager.IsShowLoadingScreen`（兜底 `CPP_LoadingScreenManager`） | 换图读条期为 `true` |
| `field.in_battle` | `boolean \| null` | `CPP_UI_BattleSubsystem.bDuringBattle` | 战斗期为 `true` |
| `field.in_event` | `boolean \| null` | `CPP_TalkEventSubsystem.m_pEventManager:IsValid()` | 覆盖过场、对话、商店、存档点、开箱提示 |
| `field.in_dialog` | `boolean \| null` | `CPP_GameWidgetManagerSubsystem.m_DialogManager.DialogWidget:IsValid()`（兜底 `CPP_DialogManager`） | 覆盖系统确认/获得物品弹窗 |
| `field.in_menu` | `boolean \| null` | `CPP_MainMenuSubsystem:IsOpenMenu()`（兜底 `CPP_VM_MainMenu.SceneDataStack` 长度 `> 0`） | 覆盖主菜单及其子页 |
| `field.can_control` | `boolean` | 派生门控：`pawn` 有效且 `visible && m_IsEnableUnitInput && m_IsEnableMovement`、非过场藏身坐标 `(25408, 384, -500)`、`is_loading/in_battle/in_event/in_dialog/in_menu` 全为 `false`、且 `m_GameLayout:GetInputTargetLayer()` 不在系统/弹窗/事件层 | 任一前置条件未知或未满足均为 `false` |
| `field.camera_deg` | `number \| null` | `VM_MiniMap_C.CameraDirectionDeg` | 镜头水平角（1 位小数），供旋转镜头区方向映射 |
| `field.camera_internal_rotation` | `boolean \| null` | `BP_MapUnit_Player_C.m_MapCameraInternalRotation` | 当前区域是否启用内部旋转镜头 |
| `field.player.location` | `{ x, y, z } \| null` | `PlayerController.Pawn:K2_GetActorLocation()` | 1 位小数；过场藏身位为 `(25408, 384, -500)` |
| `field.player.visible` | `boolean \| null` | `BP_MapUnit_Player_C.m_IsVisible` | 过场候场与战斗期间为 `false` |
| `field.player.unit_id` | `integer \| null` | `BP_MapUnit_Player_C.m_MapUnitId` | 对应 `DT_MapUnitPlayableData` / `DT_UnitParameter.ID`（序章 `60`，第 1 章起 `10`） |
| `field.player.direction` | `string \| null` | `VM_MiniMap_C.PlayerDirection`（`eMapDirectionType` 枚举短名） | 实时 8 方向（不读恒为 `North` 的 `pawn.m_Direction`） |
| `field.player.direction_deg` | `number \| null` | `VM_MiniMap_C.PlayerDirectionDegree` | 1 位小数（`0` 北、`90` 东、`180` 南、`-90` 西） |
| `field.player.movement_method` | `string \| null` | `BP_MapUnit_Player_C.m_MapMovementMethod`（`eMapMovementMethod` 枚举短名） | `Walking` / `Chocobo` / `AirShip` / `Ship` / `MagickArmor` |
| `field.player.speed_2d` | `number \| null` | `pawn.m_pPawnMovementComponent.Velocity` 平面模长（兜底按相邻两拍位移与 `Δclock_ms` 计算） | 1 位小数，单位 `u/s` |
| `field.interact_target` | `object \| null` | 优先 `pawn.m_pInteractMapGimmickList[1]`，其次 `pawn.m_InteractMapUnitList[1]`，附读 `pawn.m_InteractEventId.EventId` | `{ kind, unique_id, event_id, interact_type, location, shop_id?, inn_id?, is_open?, can_teleport? }`；`kind` 为 `"treasure_box"` / `"save_point"` / `"gimmick"` / `"npc"` |

#### 低频字段（换图完成、切区、事件/弹窗结束或每 5 秒刷新）

| 字段 | 类型 | 来源对象与属性 / Getter | 数据表对应与说明 |
| --- | --- | --- | --- |
| `field.quest_targets` | `{ main_quest_id, preferred_quest_id, destination_map_ids }` | `CPP_QuestSubsystem.{MainQuestId, PreferredQuestId}` + `CPP_ScenarioSubsystem.m_CurrentDestinationMapIdList` | `main_quest_id` / `preferred_quest_id` 对 `DT_Quest.ID`；`destination_map_ids` 对 `CDT_MapData_Prod.ID` |
| `field.unlocked_landmarks` | `integer[]` | `VM_WldMapLandmarkList_C.AllLandmarkIdList` | 对 `CDT_MapData_Prod.ID` 与 `DA_MapLandmark`；未开过大地图时为空数组 `[]` |
| `field.chests` | `Array<{ unique_id, chest_id, is_open, item_id, item_num, money, location }>` | `CPP_MapManager.m_MapGimmickManager.m_MapGimmickList` 筛宝箱，读 `m_UniqueId`、`m_Id`、`m_IsOpen`、`m_TreasureBoxData`、`K2_GetActorLocation()` | `(map_id, chest_id)` 对 `CDT_MapGimmickTreasureBox`；`item_id` 对 `DT_Item.ID`（不读旧字段 `m_ItemId`） |
| `field.save_points` | `Array<{ unique_id, type, is_teleport, can_teleport, target_map_id, target_point_id, location }>` | 同上列表单趟筛存档点，读 `m_UniqueId`、`m_Type`、`m_IsTeleport`、`CanTeleprot()`、`m_TranditionMapId`、`m_TranditionPoint`、`K2_GetActorLocation()` | `target_map_id` 对 `CDT_MapData_Prod.ID`；`target_point_id` 对 `CPP_MapTransitionPoint.m_PointID` |
| `field.transitions_and_npcs` | `{ triggers: [...], npcs: [...] }` | `m_pMapTransitionTriggerStorage.m_TransitionTriggerList`（`unique_id`, `map_id`, `point_id`, `is_same_map`, `is_auto`, `location`）+ `m_MapUnitStorage.m_MapUnits` 筛可见 NPC（`unique_id`, `unit_id`, `interact_type`, `shop_id`, `inn_id`, `location`） | `triggers[*].map_id` 对 `CDT_MapData_Prod.ID`；`point_id` 对 `CPP_MapTransitionPoint.m_PointID` |

### 4.2 `battle` 主题（`topics/battle.lua`）

依据 [`docs/state-map/battle.md`](state-map/battle.md) §11 与 `topics/battle.lua` 文件头注释整理。非战斗时仅输出前 4 个常驻字段（其余战斗期字段为 `null`）。

| 字段 | 类型 | 来源对象与属性 / Getter | 采样条件与数据表对应 |
| --- | --- | --- | --- |
| `battle.in_battle` | `boolean \| null` | `CPP_UI_BattleSubsystem.bDuringBattle` | 常驻（每 500 ms） |
| `battle.task_class` | `string \| null` | `CPP_TaskManager.m_pTask` 短类名（如 `"CPP_BtlTaskCommand"`） | 常驻（每 500 ms） |
| `battle.in_talk_event` | `boolean \| null` | `CPP_TalkEventSubsystem.m_pEventManager:IsValid()` | 常驻（每 500 ms） |
| `battle.in_tutorial` | `boolean \| null` | `CPP_TutorialSubsystem.m_TutorialWidget:IsValid()` | 常驻（每 500 ms） |
| `battle.phase` | `string \| null` | `uiBattle.BtlPhaseType`（`"Normal"` / `"Additional"` / `"Finish"`） | `in_battle == true` |
| `battle.is_event` | `boolean \| null` | `CPP_VM_Battle.bIsEvent` | `in_battle == true` |
| `battle.is_ui_visible` | `boolean \| null` | `CPP_VM_Battle.bIsDisplayBattleWidget` | `in_battle == true` |
| `battle.working_job_count` | `integer \| null` | `CPP_BtlManager.m_pBattleJobManager` 的 `#m_WorkingJobList + #m_WaitingJobList` | `in_battle == true` |
| `battle.current_actor_uid` | `integer \| null` | `uiBattle.CurrentActionUnitUniqueId`（无行动者 `-1`） | `in_battle == true` |
| `battle.timeline` | `integer[] \| null` | `VM_Timeline_C.TimelineUnified.UnifiedIds` | 战斗单位 `UniqueID` 序列；`in_battle == true` |
| `battle.risk_uids` | `integer[] \| null` | `VM_Timeline_C.RiskUnitUniqueIdList` | 预告危险技能的单位 `UniqueID`；`in_battle == true` |
| `battle.cmd` | `object \| null` | `VM_BtlCmd_C`、`uiBattle.BtlCmdManager`、`WBP_BattleLayout_C`：`{ display, type, state, ancestors, focus_index, focus_action_id, primary_ids, secondary_ids, target: { skill_id, focus_uid, target_uids, all_target_uids, can_aoe, is_all } \| null }` | `in_battle == true`；ID 对 `DT_CommandSkillData.ID` / `DT_SkillData.ID` / `Item/*` |
| `battle.units` | `object[] \| null` | `uiBattle.BtlUnitManager.UnitParamVMMap`（按 `uid` 升序）：每项含 `uid`, `unit_id`（对 `DT_UnitParameter.ID`）, `name`, `relation`（`1`=我方/`2`=敌方）, `is_boss`, `is_dead`, `hp/max_hp`, `mp/max_mp`, `bp/max_bp`, `lb/lb_max/lb_level`, `is_break`, `is_risky`, `scheduled_skill_id`（敌方，对 `DT_SkillData.ID`）, `in_action`, `in_ex_phase`, `target_state`, `weaknesses`（敌方非 `None` 项 `{ element, resistance, analyzed }`） | `in_battle == true` |
| `battle.result` | `object \| null` | `uiBattle.ResultWidget` + `CPP_VM_BattleResult.BtlResultData`：`{ page_index, total_exp, total_money, total_ap, enemy_ids, items: [{ item_id, num }] }` | `task_class == "CPP_BtlTaskWin"` 且 `IsFinishPage == false` |

### 4.3 `ui` 主题（`topics/ui.lua`）

依据 [`docs/state-map/ui.md`](state-map/ui.md) §8 与 `topics/ui.lua` 文件头注释整理。5 个常驻轻量路由字段每 500 ms 输出，8 个界面子结构仅在对应界面活跃时非空，其余为 `null`。

| 字段 | 类型 | 触发条件 / 来源对象与结构 | 说明与数据表对应 |
| --- | --- | --- | --- |
| `ui.screen_kind` | `string` | 固定优先级仲裁：`"loading"` → `"movie"` → `"tutorial"` → `"dialog"` → `"choice"` → `"talk"` → `"cutscene"` → `"save_ui"` → `"system_menu"` → `"shop"` → `"savepoint"` → `"main_menu"` → `"battle_result"` → `"battle"` → `"title"` → `"field"` | 常驻，画面一级状态 |
| `ui.active_layer` | `string \| null` | `WBP_OverAllLayout_C` 当前输入目标图层 `GameplayTag`（如 `"UI.Layer.System"`） | 常驻 |
| `ui.active_widget` | `string \| null` | 活跃图层栈顶控件短类名（核对 `IsActivated()` 且非开闭动画，过滤 CommonUI 池化残留） | 常驻 |
| `ui.action_guide_tag` | `string \| null` | `CPP_ActionGuideSubsystem.GuideTag.TagName`（`"None"` 输出 `null`） | 常驻 |
| `ui.input_blocked` | `string[]` | `CPP_GameWidgetInputManager.InputBlockSet.TokenSet`（兜底 `CPP_BFL_UI_Input:IsBlocked()`） | 常驻；含 `"WindowDeactivate"` 表示失焦屏蔽 |
| `ui.movie` | `{ active, subtitle } \| null` | `CPP_MovieSubsystem.m_pMovieScreen` 有效时展开 | 过场影片及字幕 |
| `ui.talk` | `{ event_id, window_class, is_cutscene, speaker, text, waiting_key, text_finished, can_skip, is_auto } \| null` | `CPP_TalkEventSubsystem.m_pEventManager` 有效且非 `movie`/`shop`/`savepoint`/`save_ui` | 文本按原样截断至 200 字符 |
| `ui.choice` | `{ source, dialog_tag, choice_tag, message, options, focus_index } \| null` | `CPP_DialogManager.DialogWidget` 有效或事件 `WidgetStack` 栈顶为 `*ChoiceDialog*` | `source` 为 `"dialog"` 或 `"event_choice"` |
| `ui.tutorial` | `{ name, page_index, page_count, caption, text, can_close } \| null` | `CPP_TutorialSubsystem.m_TutorialWidget` 有效 | 教学弹窗页码与文本 |
| `ui.menu` | `{ kind, scene_stack, entries, focus_index, sub_state } \| null` | `system_menu` / `main_menu` / `savepoint` / `title` 活跃时展开 | 主菜单 `sub_state` 按 `Journal`/`Heal`/`Item`/`Map` 展开，ID 对 `DT_QuestData.ID`、`DT_UnitParameter.ID`、`DT_ItemData.ID`、`DT_SkillData.ID` |
| `ui.shop` | `{ shop_id, shop_type, scene_stack, is_buy, category_index, categories, cmd_focus_index, item_focus_index, tooltip_item_id, total_amount, items } \| null` | `Stack_System` 栈顶 `WBP_SHOP_Layout_C` 且 `CPP_VM_Colosseum_Shop` 有效 | `shop_id` 对 `DT_ShopList.ID`；`items[*].id` 与 `tooltip_item_id` 对 `DT_ItemData.ID` |
| `ui.save_ui` | `{ mode, slot_indices, focus_index } \| null` | `Stack_System_Front1` 栈顶 `WBP_SaveLayout_C` / `WBP_LoadLayout_C` | `mode` 为 `"save"` 或 `"load"` |
| `ui.battle_result` | `{ page_index, has_level_up_window } \| null` | `task_class == "CPP_BtlTaskWin"` 且 `ResultWidget` 有效且 `IsFinishPage == false` | 区分报酬/普通成长/升级习得页 |

### 4.4 `party` 主题（`topics/party.lua`）

依据 [`docs/state-map/party.md`](state-map/party.md) §7 与 `topics/party.lua` 文件头注释整理。物品栏与 2048 个事件旗标在周期快照中只输出摘要与变化，全量由按需命令获取。

| 字段 | 类型 | 来源对象与属性 / Getter | 采样频率与数据表对应 |
| --- | --- | --- | --- |
| `party.save_ready` | `boolean` | `saveData.Progress.Progress[1] > 0` | 每 500 ms；`false` 表示标题占位数据 |
| `party.money` | `integer \| null` | `saveData.Player.currentMoney` | 每 500 ms |
| `party.play_time` | `number \| null` | `saveData.Player.PlayTime`（秒，保留 1 位小数） | 每 500 ms |
| `party.party_ids` | `integer[] \| null` | `saveData.Party.partyList`（编队顺序，≤ 4 人） | 每 500 ms；对 `DT_UnitParameter.ID` |
| `party.guest_unit_id` | `integer \| null` | `saveData.Party.guestUnitId`（`-1` 输出 `null`） | 每 500 ms；对 `DT_UnitParameter.ID` |
| `party.party` | `PartyMember[] \| null` | 按 `party_ids` 结合 `CPP_VM_UnitParam`（实时 `hp/max_hp/mp/max_mp/level`）与 `SaveData_UnitParameter`（`exp`, `weapon_id`, `armor_id`, `accessory_ids`, `vision_id`, `join_status`） | 每 500 ms；装备 ID 对 `Item/*.json` |
| `party.party_stats` | `Record<string, UnitStats \| null> \| null` | `ParamPointVMMap` 键 `5..9, 11..16`（`attack`, `defence`, `intelligence`, `mind`, `agility`, `equip_cost`, `max_equip_cost`, `physical_attack`, `physical_defense`, `magical_attack`, `magical_defense`, `divine`） | 低频（每 4 tick / 2 s 或脏变化刷新） |
| `party.inventory` | `{ count, total_qty, hash, changed } \| null` | `saveData.inventory` 摘要与增量 `changed: [{ item_id, before, after, delta }]`（最多 32 条） | 低频（每 4 tick 或长度/金钱变化）；全量走 `inventory` 命令 |
| `party.progress` | `integer \| null` | `saveData.Progress.Progress[1]`（对应 `{progress:0}`） | 每 500 ms |
| `party.chapter` | `integer \| null` | `progress <= 0` 为 `null`，`< 1020` 为 `0`（序章），否则 `floor(progress / 1000)` | 每 500 ms；对 `DT_ChapterData.ChapterNo` |
| `party.destination_map_ids` | `integer[] \| null` | `CPP_ScenarioSubsystem.m_CurrentDestinationMapIdList` | 每 500 ms；对 `CDT_MapData_Prod.ID` |
| `party.event_flags` | `{ total, true_count, hash, added, removed } \| null` | `saveData.Progress.eventFlag` 2048 位摘要与增量（0-based 编号，最多各 64 条） | 低频（每 4 tick 或 `progress` 变化）；全量走 `event_flags` 命令 |
| `party.main_quest_id` | `integer \| null` | `CPP_QuestSubsystem.MainQuestId` | 每 500 ms；对 `DT_QuestData.ID` |
| `party.last_saved_slot` | `integer \| null` | `optData.LastSavedDataIndex`（`-1` 输出 `null`，`0`=自动、`1..10`=手动） | 每 500 ms |

### 4.5 按需命令（只读查询与默认关闭的 R4 输入）

`protocol.lua` 在初始化时汇总四个主题及 `topics/input.lua` 的 `M.commands`；若任意模块之间命令名重复，或覆盖内置命令 `ping`，模组启动时立即报错。收到 `command.json` 后在游戏线程中分派调用并将结果写入 `ack.json`；写入 `ack.json` 前若已存在不同 `id` 的未取走回复会先记日志再覆盖，写完 `ack.json` 后须重新读取 `command.json` 并核对 `id` 一致才删除，否则保留不动（读取或解析失败时不删并按 60 秒限频记日志）。

| 命令名（`op`） | 所属模块 | 参数（`args`） | 返回（`result`）与说明 |
| --- | --- | --- | --- |
| `ping` | 内置（`protocol.lua`） | `{}` | `{ pong: true, seq: <当前 seq> }` |
| `query_nav_path` | `field` | `{ start: { x, y, z }, goal: { x, y, z } }` | `{ valid, partial, length, points: [{ x, y, z }, ...] }`；调用只读 `NavigationSystemV1:FindPathToLocationSynchronously` |
| `input_state` | `ui` | `{ mapping_names?: string[], slot?: integer }`（可选） | `{ active_imcs, swap_decide, input_enabled, input_blocked, key_bindings }`；查询 Enhanced Input 上下文与物理键码 |
| `save_slots` | `party` | `{}` | `{ slot_count, last_saved_slot, slots: [{ slot_index, is_auto_save, exists, load_status, main_progress, chapter, current_map_id, play_level, play_time, date_time_ticks, difficulty, is_cleared, party_unit_ids }] }` |
| `inventory` | `party` | `{}` | `{ count, total_qty, hash, items: [{ item_id, num }] }`（未截断全量物品栏） |
| `event_flags` | `party` | `{}` | `{ total, true_count, hash, true_ids: integer[] }`（0..2047 中全部为 `true` 的 0-based 旗标编号） |
| `press` | `input`（**默认关闭**） | `{ action: "<白名单动作>", times?: 1..20, hold_ms?: 0..5000 }`；`action == "move"` 时附 `{ x: -1..1, y: -1..1, ms: 1..5000 }` | **须桥目录存在 `input.enabled`**（否则返回 `ok: false`，“输入未开启”）；连续调用最小间隔 50 ms；每次按键写 `[FfrBridge] input ...` 日志；返回 `{ action, times, hold_ms, injected, before: { screen_kind, cursor, cursor_source, input_blocked }, after: { screen_kind, cursor, cursor_source, input_blocked }, window_deactivated }`。白名单与 `IA_*` 映射依据见 [`docs/input.md`](input.md) |

### 4.6 性能与对象失效防护

- **单周期查找去重**：`state.lua` 在每个 500 ms tick 内为 `ctx.find(className)` 维护周期内缓存（含负缓存），v0 采样与四个主题查找同名类时不会重复调用 `FindFirstOf`。
- **跨周期单例缓存（每周期查找次数）**：四个主题均不调用 `FindAllOf` 扫描大类，单例统一缓存在各主题私有 `ctx.cache` 中并在使用前核对 `IsValid()`，换图时丢弃 World 级引用：
  - `field`：换图后首个冷周期最多 11 次 `ctx.find`（净增 10 次），同图稳态周期 **0 次**；检测到 `World` 变更或 `is_loading == true` 时立即丢弃全部 World 级缓存与低频地图实体缓存；
  - `battle`：非战斗冷周期 4 次、战斗冷周期 8–10 次，稳态周期 **0 次**；`World`/`map` 变化或进出战斗时清理战斗级缓存；
  - `ui`：冷周期最多 8–9 次（活跃界面额外 0–2 次），稳态周期仅 1 次 `ctx.find("World")`（复用 `state.lua` 周期缓存，净增 **0 次**）；
  - `party`：冷周期 4 次，稳态周期 **0 次**；`party_stats` / 物品栏摘要 / 2048 位事件旗标每 4 tick（2 秒）或脏变化时刷新。
- **低频与按需分流**：宝箱、存档点、传送触发器、可见 NPC 仅在换图稳定、切区、事件/弹窗结束或每 5 秒单趟遍历 `CPP_MapManager` 内部列表；寻路查询、输入绑定、存档槽摘要、全量物品栏与全量事件旗标只走按需命令，不进 500 ms 轮询。

## 5. 和 `FfrDump` 共存

`FfrBridge` 和 `FfrDump` 可以同时在 `mods.txt` 中启用、互不影响：

- 模组目录独立：`ue4ss/Mods/FfrBridge/` 与 `ue4ss/Mods/FfrDump/`，`mod-sync` 替换其中一个不会动另一个。
- 输出路径独立：`FfrBridge` 只读写 `ue4ss/ffr-out/bridge/`，`FfrDump` 只读写 `ue4ss/ffr-out/heartbeat.json`、`trigger.txt` 和 `jobs/`。
- Lua 模块命名空间与日志标签独立：`ffrbridge.*` / `[FfrBridge]` 与 `ffrdump.*` / `[FfrDump]`。
- 两者均为只读。注意 `FfrDump` 执行 `snapshot` 时会在游戏线程耗时数秒至数十秒，期间 `FfrBridge` 的状态采样会随之暂停，快照结束后自动恢复。

## 6. 模块怎么用这些字段

`crates/ffr-module/`（工具集版本 `ffr-2`）把 `FfrBridge` v1 的原始状态与 `ffr-data` 静态数据表合并，向模型暴露 8 个 `status` 类只读工具（原有 `game_status` 保留原样转发、`bridge_ping` 不变）：

- **`read_game_state`**（参数 `{ sections?: ["field","battle","ui","party"] }`）：
  1. **新鲜度校验**：一律经 `FileChannel::read_fresh()` 读取 `state.json`。桥文件缺失或陈旧时直接返回错误并照实报“状态停止更新”，绝不用旧数据冒充当前画面。
  2. **场面判断与自动选区**：以 `ui.screen_kind` 和 `battle.in_battle` 仲裁顶层场景，生成一行中文总结（例如野外地图与可交互宝箱、战斗轮到谁选指令/目标、对话说话人与等按键、商店名称与金币）。省略 `sections` 时按场面自动挑选主题：野外选 `["field", "party"]`（失焦阻塞时附 `ui`），战斗选 `["battle"]`（有教学/对话/阻塞时附 `ui`），结算选 `["battle", "ui", "party"]`，对话/弹窗/教学/过场选 `["ui", "field"]`（战中选 `["ui", "battle"]`），商店/主菜单/存档界面/系统菜单选 `["ui", "party"]`，标题选 `["ui"]`。
  3. **静态表翻译（可选降级）**：配置 `--data-dir` / `FFR_DEPS` 时，将 `field.map_id`、`chests[*].item_id`、`transitions[*].map_id`、`battle.units[*].unit_id`、`battle.cmd.*`、`battle.units[*].scheduled_skill_id`、`ui.shop.{shop_id, items}`、`party.{chapter, progress, party[*], destination_map_ids}` 翻译成简体中文名与当前主线目标；未配置导出目录时退回原始编号与游戏内文本，不阻断启动和调用。
  4. **战斗弱点公平过滤**：`battle.units[*].weaknesses` 和 `DT_UnitParameter` 默认只向模型展示 `analyzed == true` 的已揭示弱点与抗性，其余只报“数据表另有 N 个未揭示弱点”；仅在显式传入 `--reveal-hidden-weaknesses` 时展开未揭示项（设计理由见 [`docs/fairness.md`](fairness.md)）。
- **5 个按需只读命令工具**：`read_save_slots`（`save_slots`）、`read_inventory`（`inventory`）、`read_event_flags`（`event_flags`，可选 `flag_ids` 定点核查）、`read_input_state`（`input_state`，透传可选 `mapping_names`/`slot`）和 `query_nav_path`（`query_nav_path`，透传 `start`/`goal`）。全部经 `FileChannel::send` 发给游戏内模组，再结合 `ffr-data` 把存档槽地图/章节/队员、物品栏中文名与分类、输入屏蔽告警和导航路径翻译为中文摘要 + 结构化 JSON。
- **2 个默认关闭的模拟输入工具（仅 `--enable-input` 时暴露，类别 `action`，`side_effects: true`）**：`press`（`{ action, times?, hold_ms? }`）和 `move`（`{ direction, ms }`），均经 `FileChannel::send` 转发为游戏内 `press` 命令。启动未带 `--enable-input` 时这两个工具不会出现在工具列表中，且会自动清理桥目录残留的 `input.enabled`；带 `--enable-input` 启动与会话开始时创建 `input.enabled`，会话结束或进程退出时自动删除 `input.enabled`。
