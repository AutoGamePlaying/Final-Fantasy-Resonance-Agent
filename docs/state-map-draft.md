# 状态地图草稿（R1 反射分析）

本文是基于标题画面导出的 UE4SS `.jmap` 反射数据（UE 5.6）起草的 R1 状态地图**草稿**。它按主题列出最可能承载游戏状态的类、关键属性/函数、判断依据、置信度以及后续真机快照的核对方式；全量属性偏移清单由 `src/host/cli/jmap-survey.ts` 生成到 `.deps/r1/survey/`（不进版本库）。

> [!NOTE]
> 本文全部条目均来自静态反射（类名、父类、属性类型与指针链），尚未经过野外、战斗、对话、菜单、商店、存档等真机场面快照核对。真机核对时如发现字段不更新、只读镜像或实际走别的类，以真机快照为准更新本文。

## 总体架构判断

从反射结构看，本游戏（主 C++ 模块 `/Script/mucha`，共 1457 个类、996 个结构体、269 个枚举；另有通用 UI 插件 `/Script/EnhancedUI` 和标题时已加载的 572 个 `/Game/` 蓝图生成类）的状态高度集中在三类对象上：

1. **持久化运行态单例（GameInstance 级）**：`CPP_MuchaSaveDataManager`（`GameInstanceSubsystem`）持有 `m_pGameSaveData: CPP_MuchaGameSaveData*`。该对象不仅用于存盘，更包含当前金钱、队伍、角色参数、背包、剧情进度/旗标、地图位置、任务板、商店限购等全套字段，是野外/剧情/队伍最核心的候选单一事实源。
2. **场面级管理器（World 级）**：进入具体地图后由 `CPP_MuchaWorldSetting` 的开关（`isUseBattleManager` / `isUseMapManager` / `isUseMainMenu`）配合 `CPP_MapManager`、`CPP_BtlManager`、`CPP_UI_BattleSubsystem`、`CPP_MainMenuSubsystem`、`CPP_InputManager` 等 `WorldSubsystem` 管理实时状态。
3. **UI 栈与 MVVM ViewModel（EnhancedUI + CommonUI）**：`CPP_GameWidgetManagerSubsystem` 继承自 `/Script/EnhancedUI.CPP_EU_GameLayoutSubsystem`，持有 `m_GameLayout`（`CPP_WBP_PrimaryGameLayout`）。菜单光标、选项列表、当前输入目标高度统一在 `EnhancedUI` 的 `CPP_EU_WBP_Activatable` 与 `CPP_WBP_MenuBase` 上，数据绑定多通过 `CPP_VM_*`（`MVVMViewModelBase`）暴露。

与 `docs/data-tables.md` 的互证关系：jmap 中有 254 个继承自 `TableRowBase` / `MuchaTableRowBase` 的行结构，下文各主题均标出对应的数据表名（如 `DT_ChapterData`、`DT_UnitParameter`、`DT_ShopList`），供离线数据查询与运行时 ID 反查互相印证。

---

## 1. 队伍和角色能力值

| 候选类 | 关键属性 / 函数 | 判断依据 | 置信度 | 核对场面 |
| --- | --- | --- | --- | --- |
| `CPP_MuchaSaveDataManager` → `CPP_MuchaGameSaveData` | `m_pGameSaveData`；其下 `Party`（`SaveData_Party`: `partyList: TArray<int32>`, `guestUnitId`）、`unitParameterList: TArray<SaveData_UnitParameter>`、`unitGuest: SaveData_UnitGuest`、`Player`（`SaveData_Player`: `currentMoney`, `PlayTime`）、`inventory: TArray<SaveData_Inventory>` | `GameInstanceSubsystem` 持有 `SaveGame` 子类；`SaveData_UnitParameter` 完整包含 `UnitId`、`Experience`、`HitPoint`、`MagicPoint`、`baseParameterList`、`EquipWeaponId`、`EquipArmorId`、`EquipAccessoryIdList`、`equipVisionId`、`JoinStatus`（`eUnitJoinStatus`）和各类技能列表 | 高 | 野外、菜单、战斗后 |
| `CPP_UnitParameter` / `CPP_MapUnitParameterStorage` | `CPP_MapManager.m_pMapUnitParameterStorage` → `m_UnitParameterList: TArray<CPP_UnitParameter*>`；函数 `GetUnitId`、`GetLevel`、`GetHitPoint`、`GetMaxHitPoint`、`GetMagicPoint`、`GetMaxMagicPoint`、`GetParameterByType`（按 `eUnitParameterType` 取攻/防/魔/精/敏等）、`GetStatusConditionList`、`IsDead` | `SaveData_UnitParameter` 只存经验值、当前 HP/MP 与基础加成，不直接存等级与装备计算后的上限/战斗属性；计算后的动态参数由 `CPP_UnitParameter` 提供，且对应 `CPP_BFL_Unit` / `CPP_BFL_SaveDataParty` 蓝图函数库 | 高 | 野外、菜单 |
| `CPP_VM_UnitParam` | `HitPointVM`、`MagicPointVM`、`LimitPointVM`、`LevelProgressionVM`、`ParamPointVMMap: TMap<eUnitParameterType, CPP_VM_UnitParamPoint*>`、`EquippedItemsVM`、`EquippedSkillsVM`、`ShortName` | UI 展示层 ViewModel，派生自 `CPP_VM_UnitBase`，把 `CPP_UnitParameter` 数值投影给菜单和战斗 HUD（蓝图 `WBP_UnitMainInfo_C` 等） | 中 | 菜单、战斗 |
| `CPP_UnitListSubsystem` | 函数 `GetJoinUnitList` | `GameInstanceSubsystem`，无反射属性，大概率作为查询已入队角色的薄封装，需确认是否内部直读 `CPP_MuchaGameSaveData` | 低 | 野外、菜单 |

- **数据表印证**：`UnitId` 与基础/成长数值对应 `DT_UnitParameter`、`DT_PlayableUnitParameter`、`DT_UnitLevelParameterList`、`DT_UnitParameterCalc`；状态异常对应 `DT_StatusConditionDetailData`；背包/装备 ID 对应 `DT_ConsumablesItemData`、`DT_EquipmentItemData`、`DT_ValuableItemData`、`DT_VisionItemData`。
- **真机核对重点**：确认战斗中或回血后 `CPP_MuchaGameSaveData.unitParameterList` 的 HP/MP 是否实时同步，以及 `CPP_UnitParameter` 的计算属性是否有未暴露给反射的 C++ 字段（若有，需通过安全 getter 或 `CPP_VM_UnitParam` 读）。

---

## 2. 战斗：回合、行动顺序、指令菜单、目标

| 候选类 | 关键属性 / 函数 | 判断依据 | 置信度 | 核对场面 |
| --- | --- | --- | --- | --- |
| `CPP_BtlManager` | `m_pUnitManager: CPP_BtlUnitManager*`、`m_pBattleJobManager: CPP_BtlJobManager*`（`m_WaitingJobList` / `m_WorkingJobList`）、`m_pBtlInput: CPP_BtlInput*`、`m_pAutoCmdManager`、`m_pGameSpeedManager` | `TickableWorldSubsystem`，战斗核心调度器；聚合单位、行动任务队列、战斗输入和自动战斗 | 高 | 战斗 |
| `CPP_UI_BattleSubsystem` | `bDuringBattle: bool`、`BtlPhaseType: eBtlPhaseType`（`Normal`/`Additional`/`Finish`）、`CurrentActionUnitUniqueId: int32`、`FocusSkillId: int32`、`BtlCmdManager: CPP_UI_BtlCmdManager*`、`BtlUnitManager: CPP_UI_BtlUnitManager*`、`BtlWidget`、`ResultWidget` | `WorldSubsystem`，最直接的“是否在战斗、当前阶段、当前行动者 UniqueId、结算界面是否打开”汇总入口 | 高 | 战斗（指令中、演出中、结算） |
| `CPP_BtlUnitManager` + `CPP_BtlUnit` + `CPP_BtlUnitParameter` | `m_UnitList: TArray<CPP_BtlUnit*>`、`m_pCmdSelectUnit: CPP_BtlUnit*`、`m_TargetUnitList: TArray<CPP_BtlUnit*>`、`m_pTimelineScheduler: CPP_BtlUnitTimelineScheduler*`；`CPP_BtlUnit.m_pUnitParameter: CPP_BtlUnitParameter*`（继承 `CPP_UnitParameter`，含 `GetDisplayHitPoint`、`GetDisplayMagicPoint`、`GetDisplayBreakPoint`、`GetRelation` 区分敌我 `eRelation`、`IsBoss`、`IsDead`、`IsBreak`、`IsAnalyzed`） | 明确同时包含敌我全部参战单位、当前选指令单位、已锁定目标列表，以及每个单位的完整战斗参数（HP/MP/Break/敌我阵营/异常状态） | 高 | 战斗 |
| `CPP_VM_BtlTimeLine` + `CPP_VM_BtlTurn` / `CPP_BtlUnitTimelineScheduler` | `CPP_VM_BtlTimeLine.TimelineUnified`（`UIBtlTimelineUnified`: `SegmentSetMap` 按 `EUIBtlTurnTimeLine` 的 `Current`/`Next`/`Additional` 分段、`UnifiedIds: TArray<int32>`）、`TurnVMMap`、`bIsCmdState`；`CPP_VM_BtlTurn.EntryDataList`、`bActiveTurn`；`CPP_BtlUnitTimelineScheduler.m_AdditionalTimeline` | 战斗采用时间轴/回合段顺序（含追加回合 `Additional`）；`CPP_BtlUnitTimelineScheduler` 反射仅暴露追加轴，完整当前/下回合行动顺序大概率在 `CPP_VM_BtlTimeLine`（蓝图 `VM_Timeline_C`）中更易读取 | 中 | 战斗 |
| `CPP_UI_BtlCmdManager` + `CPP_VM_BtlCmd` | `CmdType: eBtlCmdType`（`Normal`/`Additional`/`Finish`）、`CmdState: EBtlCmd_State`（`List`/`Target`）、`ActionUnitUniqueIdList`；`CPP_VM_BtlCmd` 持 `bDisplayCmd`、`CmdStack`（`CPP_BtlCmdStack.Commands: TArray<BtlCmdStackData>`）、`FocusCmdData: BtlCmd_EntryValue`（含 `EntryIndex` 与 `ActionKey`）、`ListIndexMap`、`UnitSelectVM: CPP_VM_BtlCmdUnitSelect*` | 完整刻画指令菜单是否弹出、处于列表还是选目标态、层级栈（一级/二级指令蓝图 `WBP_BtlCmd_PrimaryList_C` / `WBP_BtlCmd_SecondaryList_C`）以及当前高亮指令 | 高 | 战斗（指令菜单） |
| `CPP_VM_BtlCmdUnitSelect` + `CPP_VM_BtlUnitParam` | `FocusBaseUniqueId: int32`、`TargetUniqueIdListMap: TMap<eRelation, BtlCmdUniqueIdList>`、`AllTargetUniqueIdList`、`targetUniqueIdList`、`bCanBeAoE`；`CPP_VM_BtlUnitParam.TargetSelectState`（`None`/`Main`/`Sub`） | 选目标阶段的专用 ViewModel（蓝图 `VM_BtlCmdUnitSelect_C`），直接列出可选目标池、当前光标所在单位 UniqueId 以及是否群攻 | 高 | 战斗（选目标） |
| `CPP_BtlTaskCommand` | `m_pActionUnit: CPP_BtlUnit*`、`m_pPrevActionUnit`、`m_UnitDeadStateList` | `CPP_Task` 子类，还有同族 `CPP_BtlTaskBeginTurn`、`CPP_BtlTaskAdditionalTurn`、`CPP_BtlTaskWin`、`CPP_BtlTaskLose`，可辅助判别战斗流程节点 | 中 | 战斗 |

- **数据表印证**：敌人编组与站位对应 `DT_BattleGroup`（`BATTLE_GROUP_DATA`）、`DT_BattleUnitLayout`；角色默认指令结构对应 `DT_UnitBattleCmd`；技能效果与目标规则对应 `DT_SkillData`、`DT_SkillTargetData`、`DT_CommandSkillData`。
- **真机核对重点**：确认能否通过 `CPP_UI_BattleSubsystem.BtlUnitManager.UnitParamVMMap` 稳定拿到所有敌我单位（含 `UniqueID` 与 HP/Break），以及 `CPP_VM_BtlTimeLine.TimelineUnified.UnifiedIds` 是否给出完整行动顺序。

---

## 3. 野外：玩家角色、坐标、朝向、移动方式

| 候选类 | 关键属性 / 函数 | 判断依据 | 置信度 | 核对场面 |
| --- | --- | --- | --- | --- |
| `CPP_MapUnit_Player`（父类 `CPP_MapUnit_Playable` → `CPP_MapUnit` → `CPP_Unit` → `ACharacter`） | 位置/旋转：UE `Actor` 变换（`K2_GetActorLocation` / `K2_GetActorRotation`）；移动态：`m_MapMovementMethod: eMapMovementMethod`（`Walking`/`AirShip`/`Chocobo`/`Ship`/`MagickArmor`）、`m_IsDash: bool`、`m_IsEnableMovement: bool`、`m_IsEnableUnitInput: bool`；朝向：`m_Direction: eMapDirectionType`（8 方向：`North`..`NorthWest`）；输入向量：`m_InputActionValue: Vector2D`、`m_MoveVector: Vector2D`；当前可交互对象：`m_pInteractMapUnit`、`m_InteractMapUnitList`、`m_pInteractMapGimmickList`、`m_InteractEventId` | 继承自引擎 `ACharacter` 并持有 `PawnMovementComponent*`、速度上限（`m_MaxWalkingSpeed`/`m_MaxRunningSpeed`/`m_MaxJogSpeed`）、滑动/射线偏移、连续二维输入向量与 `CPP_Map_NavMeshBoundsVolume`，强烈说明野外移动为**连续 3D/2D 坐标移动**（非格子移动），同时朝向离散化为 8 方向枚举供精灵图动画与存档记录 | 高 | 野外 |
| `CPP_MapUnitStorage`（由 `CPP_MapManager.m_MapUnitStorage` 持有） | `m_MapUnits: TArray<CPP_MapUnit*>`、`m_MapUnitObjects: TArray<CPP_MapUnit_Object*>`、`m_MapUnitNPCAreaMap` | 管理当前地图全部地图单位（含玩家、NPC、载具），用于定位玩家 Actor 与附近 NPC 坐标 | 高 | 野外 |
| `CPP_FieldObjectSubsystem` | `ObjectMap: TMap<int32, FieldObjectData>`、`CollectedFldObjManager`、`GetActorAtId` | `WorldSubsystem`，统一索引野外交互物/地图对象及其 ID | 中 | 野外 |
| `CPP_MuchaGameSaveData.Map`（`SaveData_Map`） | `unitLocation: Vector`、`unitDirection: eMapDirectionType`、`IsRidingAirShip`、`IsRidingChocobo`、`IsRidingRaft`、`airShipLocation`、`encountCount` | 存档结构中保存玩家坐标、8 方向朝向和载具状态；但实时移动时是否每帧写回尚不明确，实时坐标应优先读 Player Pawn | 中 | 野外（移动前后对比） |
| `CPP_MuchaPlayerController` | 继承自 `/Script/Engine.PlayerController`，无额外反射属性 | 玩家控制器的标准入口，通过其 `Pawn` / `AcknowledgedPawn` 核对当前操控的是否直接是 `CPP_MapUnit_Player` 或载具子类 `CPP_MapUnit_Vehicle` / `CPP_MapUnit_Ship` | 高 | 野外 |

- **数据表印证**：移动速度与参数对应 `DT_MapPlayableUnitMoveData`、`DT_MapUnitPlayableData`、`DT_MapTerrainData`；遇敌表对应 `DT_EncountGroup`、`DT_EncountStagingData`。
- **真机核对重点**：重点验证（1）连续移动与碰撞是否真的走 `PawnMovementComponent` + NavMesh；（2）`m_InteractMapUnitList` / `m_pInteractMapGimmickList` 在靠近 NPC 或宝箱时是否立即更新，这对 R5“调查”原语至关重要。

---

## 4. 地图和场景切换

| 候选类 | 关键属性 / 函数 | 判断依据 | 置信度 | 核对场面 |
| --- | --- | --- | --- | --- |
| `CPP_MapManager` | 函数 `GetMapId`、`GetOldMapId`、`GetMapAreaId`、`GetCurrentIslandType`（`eMapIslandType`）、`GetEncountMovementLengthParcentage`；属性 `m_MapTransitionPointList: TArray<CPP_MapTransitionPoint*>`、`m_pMapTransitionTriggerStorage`、`m_pSublevelLoadManager: CPP_MapSubLevelLoadManager*`、`m_pSubLevels`、`m_BattleFinishTransition: MAP_LOCATION`、`m_MapGimmickManager`、`m_MapCannotSaveAreaStorage`、`m_TransitionEventList`、`m_AfterTransitionEvetnList` | `WorldSubsystem`（51 个属性、23 个函数），是地图 ID、子关卡流式加载、出口触发器、传送点与机关的总控中心 | 高 | 野外、换图前后 |
| `CPP_MapTransitionPoint` | `m_PointID: int32`、`m_Direction: eMapDirectionType`、`m_PointName: FText`、`m_AreaBoxList: TArray<TSoftObjectPtr<CPP_Map_NavMeshBoundsVolume>>` | 关卡内出入口/落点 Actor，直接对应 `MAP_LOCATION.pointId`，并与地图数据表里的切换点相连 | 高 | 野外 |
| `CPP_MapSubLevelLoadManager` | `m_LoadLevelStreamingList`、`m_StreamableLoadLevelList`、`m_CompletedeLoadeLevelList`、`m_IndoorLoadLevel`、`IsCompletedLoadByIndoor` | 追踪室内外与大地图子关卡的异步流式加载进度，适合判断“换图是否完成” | 中 | 换图、进出建筑 |
| `CPP_FadeManager` + `CPP_GameWidgetManagerSubsystem.m_LoadScrManager` | `CPP_FadeManager`（`GameInstanceSubsystem`）；`CPP_LoadingScreenManager`（由 `CPP_GameWidgetManagerSubsystem.m_LoadScrManager` 持有） | 场景切换通常伴随淡入淡出或加载遮罩，二者配合子关卡完成状态可作为“切换稳定可操作”的判据 | 中 | 标题进野外、换图 |
| `CPP_MuchaGameSaveData.Map`（`SaveData_Map`） | `mapId: int32`、`lastIslandVisited: eMapIslandType`、`perMapSaveDataMap: TMap<int32, SaveData_PerMap>`（`bIsKnown`/`bIsVisited`/`bIsCleared`）、`mapGimmick: TMap<int32, SaveData_MapGimmick>`（含宝箱状态 `SaveData_TreasureBox`） | 记录当前地图 ID、各图探索状态以及每张图的宝箱/机关状态 | 高 | 野外、换图后 |
| `CPP_MuchaWorldSetting` | `isUseBattleManager`、`isUseMapManager`、`isUseMainMenu` | `AWorldSettings` 子类，三个布尔位可辅助区分当前 World 类型（如标题/启动图 vs 野外图） | 中 | 标题、野外 |

- **数据表印证**：地图元信息对应 `CDT_MapData_Prod`（及分地区 `DT_MapData_*`）、`DT_MapIslandData`、`DT_MapRegionData`、`DT_MapReleaseConditionData`；宝箱/机关对应 `Map/Gimmick/TreasureBox/` 下的数据表。
- **真机核对重点**：核对换图时是切换整个 Persistent Level 还是只切子关卡（`m_pSublevelLoadManager`），以及 `SaveData_Map.mapId` 与 `CPP_MapManager:GetMapId()` 在切图瞬间的更新时机。

---

## 5. 对话和选项

| 候选类 | 关键属性 / 函数 | 判断依据 | 置信度 | 核对场面 |
| --- | --- | --- | --- | --- |
| `CPP_TalkEventSubsystem` | `m_pEventManager: CPP_TalkEventManager*`、`m_pInteractActorUnit`、`m_pInteractTargetUnit`、`m_pInteractTargetGimmick`、`m_pPreAction`；配合函数库 `CPP_BFL_TalkEvent`（`IsPlayingEvent`、`GetCurrentEventId`、`GetCurrentEventData`） | `GameInstanceSubsystem`，剧情事件与 NPC 对话的总入口；`m_pEventManager` 非空大概率意味着事件正在播放 | 高 | 对话 |
| `CPP_TalkEventManager` → `CPP_WBP_TalkEvent`（父类 `CPP_WBP_Talk`） | `m_pWidget: CPP_WBP_TalkEvent*`、`m_pScene: CPP_TalkEventScene*`、`m_NotifyObtainDialogWidget`；`CPP_WBP_Talk` 含 `WindowManager: CPP_WBP_TalkWindowManager*`、`m_pTalkDataContainer`、`m_pCurrentTalkData: CPP_TalkData*`（函数 `GetSpeaker`、`GetText`、`GetEmotion`）、`m_pAutoProceed`；`CPP_WBP_TalkEvent` 含 `WidgetStack`、`m_DialogChoiceTag: GameplayTag`、`CanSkip`、`CanTalkSkip`、`IsAutoPlay` | 对话运行时完整管线：从事件管理器直达当前台词对象（说话人 + 文本）与窗口控制器 | 高 | 对话 |
| `CPP_WBP_TalkWindow`（父类 `CPP_WBP_TextWindow`） | `m_pTextBlock: CPP_GradualTextBlock*`、`m_pSpeaker: CPP_WBP_TalkSpeaker*`、`m_ProceedTextSpeed`；蓝图 `WBP_TalkBattle_Caption_C` 亦继承此类 | 逐字显示对话框本体；`m_pTextBlock` 可用于读取当前已渲染/完整台词文本，结合 `texts.jsonl` 验证 | 高 | 对话、战斗字幕 |
| `CPP_DialogManager` + `CPP_WBP_DialogLayout` / `CPP_WBP_DialogBox` + `EnhancedUI.CPP_EU_WBP_DialogChoice` | `CPP_GameWidgetManagerSubsystem.m_DialogManager` → `DialogWidget: CommonActivatableWidget*`；`DialogBoxInfo`（`DialogTag`、`Message: FText`、`ChoiceTag: GameplayTag`）；`CPP_EU_WBP_DialogChoice`（继承 `CPP_WBP_MenuBase`）含 `m_ChoiceData: EU_DialogChoiceData`（`ChoiceTag`、`ChoiceList: TArray<FText>`）及光标状态 `FocuedItemDataMap` | 覆盖事件选项弹窗（`ETalkEventChoiceDialogType`）及系统确认框；`ChoiceList` 给出全部选项文案，`CPP_WBP_MenuBase` 给出当前光标项 | 高 | 对话（含选项）、确认弹窗 |
| `CPP_ChatSubsystem` / `CPP_BacklogSubsystem` | `m_pNoticeWidget: CPP_WBP_ChatNotice*`、`m_pChatWidget: CPP_WBP_Chat*` | 对应野外边走边聊（Active Time Chat）与对话回看记录，需与阻塞式 `TalkEvent` 区分，避免把非阻塞小聊误判为对话锁屏 | 中 | 野外、对话 |

- **数据表印证**：对话事件主表 `CDT_TalkEventData_Prod` 与 `DT_TalkEventCategoryData`，分章对白文本在 `Datatable/Event/Text/Chapter01..08/`；通用弹窗与选项对应 `CDT_DialogBox`（`DialogBoxInfo`）和 `DT_DIalogChoice`；边走边聊对应 `ChatEventData` / `ChatRowData`。
- **真机核对重点**：确认逐字打印是否结束（`CPP_GradualTextBlock` 的内部状态），以及对话选项出现时到底是挂在 `CPP_WBP_TalkEvent.WidgetStack` 还是 `CPP_DialogManager.DialogWidget`。

---

## 6. 菜单和光标

| 候选类 | 关键属性 / 函数 | 判断依据 | 置信度 | 核对场面 |
| --- | --- | --- | --- | --- |
| `CPP_GameWidgetManagerSubsystem` → `CPP_WBP_PrimaryGameLayout`（父类 `/Script/EnhancedUI.CPP_EU_WBP_GameLayout`） | `m_GameLayout` 含 `Layers: TMap<GameplayTag, CommonActivatableWidgetContainerBase*>`、`CurrentInputTargetLayer`、`bAcceptsInput`；蓝图 `WBP_OverAllLayout_C` 为其具体实现；函数 `GetTopLayer`、`GetActiveWidgetFromLayer`、`GetInputTargetLayer` | 整个游戏 UI 的顶层容器与输入分发根，直接告诉你当前哪个图层（主菜单、弹窗、战斗、标题、提示）处于活动栈顶且接收输入 | 高 | 标题、菜单、对话、战斗、商店 |
| `/Script/EnhancedUI.CPP_EU_WBP_Activatable`（及子类 `CPP_WBP_MuchaActivatable`） | `IsInputTarget: bool`、`InputTargetWidget: TWeakObjectPtr<Widget>`、`InputTargetWidgetStack: TArray<...>`、`bCanReceiveInput: bool`、`IsOpeningProcess: bool`、`IsClosingProcess: bool`、`InputConfig: UIInputConfig` | 几乎全部全屏界面和子面板（主菜单、标题 `WBP_TitleLayout_C`、商店、对话、弹窗）的共同基类；`InputTargetWidget` 精确指向当前拥有焦点的子菜单，`IsOpeningProcess`/`IsClosingProcess` 可判断动效是否就绪 | 高 | 标题、菜单、商店、对话 |
| `/Script/EnhancedUI.CPP_WBP_MenuBase`（及接口 `CPP_BI_MenuBase`、子类 `CPP_EU_WBP_DirectionGridMenu` / `CPP_EU_WBP_PanelMenu` / `CPP_EU_WBP_DialogChoice`） | `FocuedItemDataMap: TMap<EEnhancedMenu_FocuedTimeType, EnhancedMenu_FocusedItemData>`（值含 `Index: int32`、`Widget: Object*`）、`SelectedItemList: TArray<Widget*>`、`FirstFocusItemIndex: int32`；接口函数 `GetFocusedItemIndex`、`GetFocusedItemWidget`、`GetItemList` | **全局菜单光标的统一底层实现**（注意源码属性名拼作 `FocuedItemDataMap`）；列表型控件普遍通过 `CPP_EU_WBP_MenuPath.PathMenu` 委托给它 | 高 | 标题、菜单、战斗、商店、对话选项 |
| `CPP_MainMenuSubsystem` + `CPP_VM_MainMenu` | `CPP_MainMenuSubsystem`：`SceneDataMap: TMap<GameplayTag, MainMenuSceneData>`、`SceneEnableMap`、`bIsTransitioning: bool`、`LastSelectedTargetUnitId: int32`，函数 `IsOpenMenu`、`CanOpen`、`GetCurrentScene`、`GetMainMenuVM`；`CPP_VM_MainMenu`：`SceneDataStack: TArray<GameplayTag>`、`PrevSceneTag` | `WorldSubsystem` + ViewModel，直接反映主菜单是否打开、当前所在子页面栈（道具/装备/队伍/恢复/世界地图/系统等）及是否正在切页 | 高 | 菜单 |
| `CPP_TitleController` + `WBP_TitleLayout_C` / `CPP_VM_Title` | `WBP_TitleLayout_C` 继承 `CPP_WBP_MuchaActivatable`，含 `WidgetSwitcher_0`、`WBP_DirectionLayoutMenu_Difficulty`；入口项 `WBP_Title_ListEntry_C` 含 `EntryType` | 标题画面菜单控制与选项条目，可在标题快照里立刻核对 | 中 | 标题 |

- **数据表印证**：主菜单页面定义对应 `DT_MainMenuScene`（`MainMenuSceneData`）；界面布局对应 `DT_GameLayoutWidget_*`、`DT_GameWidgetAsset`。
- **真机核对重点**：标题画面即可先验证 `CPP_GameWidgetManagerSubsystem.m_GameLayout` → 顶层 `Activatable` → `InputTargetWidget` → `FocuedItemDataMap` 这条通用读光标链路，再在主菜单和二级菜单核对 `CPP_WBP_ButtonPath` / `WBP_ListViewMenu_C` 是否全部走同一机制。

---

## 7. 商店

| 候选类 | 关键属性 / 函数 | 判断依据 | 置信度 | 核对场面 |
| --- | --- | --- | --- | --- |
| `CPP_WBP_ShopLayoutBase` | 继承 `CPP_WBP_MuchaActivatable`；`WidgetStack: CPP_EU_ActivatableWidgetStack*`、`MenuTitle: FText`、`CommandTag: GameplayTag`、`SubheadMain_Title`、`IsHideCurrentMoney`；函数 `GetCommandList`、`SetShopData` | 商店根界面；`WidgetStack` 管理买/卖/数量确认等子页面压栈 | 高 | 商店 |
| `CPP_VM_Shop_TradeWindow` | `IsBuy: bool`、`ShopID: int32`、`CurrentItemList: TArray<int32>`、`ItemInfoList: TArray<SHOP_ItemInfo>`、`SelectItemList: TArray<SHOP_TradeItemData>`、`TradeItemData`、`CurrentTotalAmount: int32`、`ShopEntryInfo`；函数 `GetFocusItemID`、`GetShopID`、`GetIsBuy`、`GetTotalAmount`、`GetItemEnable` | 商店交易核心 ViewModel（蓝图 `VM_Shop_TradeWindow_C`），一次性囊括当前商店 ID、买/卖模式、上架/可卖商品列表、已选清单与总价 | 高 | 商店 |
| `CPP_WBP_Shop_CommandWindow` / `CPP_WBP_Shop_CommandLists` / `CPP_WBP_DialogCustom_ShopBuy` | 均继承 `CPP_EU_WBP_Activatable`；商品项蓝图 `WBP_SHOP_ItemData_C`（父类 `CPP_CommonMenu_ItemData`）与 `WBP_OP_TotalAmount_C` | 商店菜单页面与购买确认弹窗；菜单光标走第 6 节的 `EnhancedUI` 焦点体系 | 中 | 商店 |
| `CPP_MuchaGameSaveData.shop` + `CPP_BFL_Shop` | `shop: TArray<SaveData_ShopItem>`（`ItemId`、`ShopID`、`MaxOrderNum`、`CurrentBuyNum`）；`CPP_BFL_Shop` 含 `CanBuy`、`GetCanBuyItemNum`、`GetOrderMaxNum`、`GetShopData` | 持久化的限购商品已购次数与商店查询逻辑；搭配 `SaveData_Player.currentMoney` 验证买/卖前后的金钱与库存变化 | 高 | 商店、交易前后 |
| `CPP_VM_Colosseum_Shop` | `MVVMViewModelBase` 子类，4 个属性、14 个函数 | 斗技场专用兑换商店 ViewModel，若后续涉及斗技场需与普通城镇商店区分 | 低 | 商店（斗技场） |

- **数据表印证**：普通商店与杂货店清单对应 `DT_ShopList`、`DT_VariousShopsList`（枚举 `eShopType` 含道具/武器/防具/饰品/杂货等）、`DT_ShopEquipItemParameterList`；限购/斗技场商品对应 `Colosseum_ShopItemData`。
- **真机核对重点**：进入城镇商店后核对能否直接从 `CPP_VM_Shop_TradeWindow` 读出 `ShopID`、`IsBuy`、`CurrentItemList` 与当前高亮商品，以及 `WBP_SHOP_ItemData_C` 是否暴露单价和库存。

---

## 8. 剧情进度和旗标

| 候选类 | 关键属性 / 函数 | 判断依据 | 置信度 | 核对场面 |
| --- | --- | --- | --- | --- |
| `CPP_MuchaGameSaveData.Progress`（`SaveData_Progress`）+ `CPP_BFL_SaveDataProgress` | `Progress: TArray<int32>`、`eventFlag: TArray<bool>`、`EventName: TArray<FString>`；函数库 `CPP_BFL_SaveDataProgress` 提供 `GetMainProgress`、`GetFlag`、`SetFlag` | 最直接的剧情进度整数数组、事件旗标布尔数组与已触发事件名列表；`SaveData_InitGameData`（`DT_InitGameSaveData`）中也用 `Progress` 与 `OnFlagList` 初始化开局状态 | 高 | 野外、事件前后 |
| `CPP_ScenarioSubsystem` + `CPP_BFL_Scenario` | `m_CurrentDestinationMapIdList: TArray<int32>`、`m_CurrentMainScenarioMapObjectList: TArray<MAP_OBJECT_TYPE_DATA>`；函数 `GetCurrentChapter`、`GetCurrentMainScenarioSection`、`GetCurrentMainScenarioMapObjectList`、`CPP_BFL_Scenario:GetChapterNumber` | `GameInstanceSubsystem`，把底层 `Progress` / `eventFlag` 结合主线数据表解析成当前章节、当前主线段落与目标地图/目标点列表，对 AI 规划核对当前章进度极其有用 | 高 | 野外、推剧情后 |
| `CPP_QuestSubsystem` + `CPP_MuchaGameSaveData.questBoardList` / `quest` / `request` | `QuestDataMap: TMap<int32, QuestData>`、`MainQuestId: int32`、`PreferredQuestId: int32`；存档中 `questBoardList: TArray<SaveData_QuestBoard>`（`QuestId`、`State: EQuest_State`、`KilledCount`、`BountyData`）及 `request: SaveData_Request`（`DataMap` 含 `State: ERequestState`、`ProgressIndex`） | `GameInstanceSubsystem` + 存档字段，同时覆盖主线/支线任务追踪（`EQuestCategory` / `EQuestProgression`）与委托/任务板状态 | 高 | 野外、菜单（任务） |
| `CPP_ConditionSubsystem` | `GameInstanceSubsystem`，提供条件判定入口（对应 `FlagCondition` 表结构） | 集中评估 `CDT_FlagCondition` 条件表达式（是否满足前置剧情旗标） | 中 | 野外、事件前后 |

- **数据表印证**：章节与主线段落对应 `DT_ChapterData`（`ChapterData`）、`DT_MainScenario`（`ScenarioSection`，含 `Condition`、`DestinationList`）；旗标条件对应 `CDT_FlagCondition`、`DT_FlagCondition_System` 及分章 `DT_FlagCondition_Chapter01..08`；任务与委托对应 `CDT_QuestBoardData`、`DT_RequestData`。
- **真机核对重点**：推进一个早期主线事件前后各取一次野外快照，核对 `SaveData_Progress.Progress[0]` 是否就是 `GetMainProgress()`（以及其余元素含义），并核对 `eventFlag` 的下标与 `DT_FlagCondition_Chapter01` 中旗标编号的映射关系。

---

## 9. 存档和读档

| 候选类 | 关键属性 / 函数 | 判断依据 | 置信度 | 核对场面 |
| --- | --- | --- | --- | --- |
| `CPP_MuchaSaveDataManager` | `m_pGameSaveData: CPP_MuchaGameSaveData*`、`m_pOptionSaveData: CPP_MuchaOptionSaveData*`、`GameSaveDataSummaryMap: TMap<int32, MuchaGameSaveDataSummary>`、`tempGameDifficulty`；函数 `GetSaveSlotNum`、`GetValidSaveDataCount`、`GetValidSaveSlotIndexes`、`GetLatestSaveSlotIndex`、`FindLatestSaveDataSlotIndexByTimestamp`、`FindGameSaveDataSummary`、`IsValidGameSaveData` | `GameInstanceSubsystem`，完整管理槽位索引、摘要映射和读写入口；通过 `GameSaveDataSummaryMap` 就能离线/无侵入查清总槽位数、哪个槽是空的、哪个是自动存档，为 `docs/plan.md` 选定“固定空槽位”提供直接依据 | 高 | 标题、存档界面 |
| `MuchaGameSaveDataSummary`（结构体） | `bExists: bool`、`bIsDemo: bool`、`LoadStatus: ESaveLoadStatus`、`bIsCleared: bool`、`DateTime`、`PlayTimeSec`、`MainProgress: int32`、`CurrentMapId: int32`、`PlayLevel: int32`、`PartyUnitIds: TArray<int32>` | 槽位摘要结构体，可用来在不加载存档的前提下核验槽位是否为空及内容摘要 | 高 | 标题、存档界面 |
| `CPP_VM_SaveData` + `CPP_VM_SaveData_Load` + `CPP_UI_SaveDataSubsystem` | `CPP_VM_SaveData` 含 `SlotIndex: int32`、`SaveDataSummary`、`bSummaryExists: bool`、`bShouldDisplaySaveData`、`bIsAutoSave: bool`；`CPP_VM_SaveData_Load` 含 `SlotIndex`、`TryLoad` | 存/读档界面的槽位条目与读档 ViewModel；其中的 `bIsAutoSave` 与 `SlotIndex` 能明确自动存档槽和手动槽的编号约定 | 高 | 标题（读档）、存档界面 |
| `CPP_VM_SavePoint` + `CPP_MapManager.m_MapCannotSaveAreaStorage` | `ESavePointCommandType`（`SavePoint`/`Tent`/`Teleport`/`Teleport_Exit`/`DLC`）、`WBP_SavePoint_Command_C`；`CPP_MapManager.m_MapCannotSaveAreaStorage` | 野外存档点菜单（存档/帐篷/传送）以及禁存档区域判定，配合 `MAP_LOCATION.bDoAutoSave` 反映自动存档触发时机 | 中 | 野外（存档点） |

- **数据表印证**：新游戏初始化数据对应 `DT_InitGameSaveData`（`SaveData_InitGameData`）；存档点指令与配置对应 `DT_SavePointData`、`DT_SavePointCommand`。
- **真机核对重点**：在标题画面快照（或读档菜单快照）检查 `CPP_MuchaSaveDataManager.GameSaveDataSummaryMap` 的全部键（槽位编号范围、自动存档是否占槽 0）及哪些槽 `bExists == false`，据此请用户拍板固定空槽位。

---

## 10. 输入：Enhanced Input 的映射上下文和动作名（给 R4 用）

| 候选类 / 枚举 / 资产 | 关键属性 / 函数 / 成员 | 判断依据 | 置信度 | 核对场面 |
| --- | --- | --- | --- | --- |
| `CPP_InputManager` | `WorldSubsystem`；属性 `bSwapDecideButton: bool`、`m_pInputRegistrant: CPP_InputRegistrant*`、`ActivateIMC`、`ActivateIMTMap: TMap<eMuchaInputMappingType, ...>`；函数 `GetAllIMCStackList`、`GetInputAction`、`FindInputActionInstance`、`RegisterInputMappingContext`、`GetDataTable`、`IsEnableInput`、`GetCurrentInputModeDebugString`、`FindCurrentMappingForSlot`、`GetKeyListToInputAction` | 游戏统一的 Enhanced Input 封装层；由它维护映射上下文栈（IMC stack）、动态改键和输入使能开关，且内置 `Debug_PrintLogStackMappingContext` / `GetAllIMCStackList` 便于核对当前激活的 IMC | 高 | 标题、野外、战斗、菜单、对话 |
| `eMuchaInputMappingType`（枚举，33 个有效映射类型） | 按场面划分上下文：野外 `MapUnit`、`MapUnitDash`；战斗 `Battle`、`UI_Battle`；标题 `TitleMenu`；商店 `Shop`；菜单 `UI_Common`、`UI_MainMenu`、`UI_Field`、`UI_FieldSideMenu`、`UI_Equip`、`UI_PartyEdit`、`UI_PartyHeal`、`UI_OmakaseCure`、`UI_ActionEquip`、`UI_Item`、`UI_WldMap`、`UI_MapCommon`、`UI_Quest`、`UI_SystemMenu`、`OptionMenu`、`Option_InputSetting`；对话与事件 `UI_Event`、`UI_EventPause`、`UI_Backlog`、`UI_Chat`、`UI_ChatList`；其他 `Race`、`RaceUnit`、`SoundPlayer`、`UI_Info`、`UI_TextEdit`、`UI_Key` | 直接对应 `MUCHA_INPUT_DATA_LIST.inputMappingType` 和 `MUCHA_INPUT_MAPPING_CONTEXT_DATA`（父子上下文关系），基本枚举出了游戏全部输入上下文分组 | 高 | 各场面快照对比 |
| `CPP_DA_MUCHA_INPUT` + `MUCHA_INPUT_DATA_LIST`（表 `/Game/Input/DataAsset/DT_InputData`） | `pInputMappingContextList: TMap<eMuchaInputDeviceType, InputMappingContext*>`（区分 `Gamepad` 与 `KeyboardAndMouse`）、`InputActionList: TMap<FString, InputAction*>` | `DT_InputData`（在 `DT_AppDataTableList` 中注册为 `eDataTableType::InputData`）将每个 `eMuchaInputMappingType` 指向一份 `CPP_DA_MUCHA_INPUT`，内含真实 Enhanced Input `InputMappingContext*` 与按字符串名索引的 `InputAction*` 字典 | 高 | 真机运行时（或解包 `/Game/Input/`） |
| `CPP_GameWidgetInputManager`（父类 `/Script/EnhancedUI.CPP_EU_WidgetInputHandler`） | 属性 `InputBlockSet: UIInputBlockSet`（`TokenSet: TSet<FName>`）；41 个 `HandleInput*` 回调，覆盖通用 UI（`Decide`、`Cancel`、`Direction`、`Detail`、`Escape`、`Scroll`、`TabSwitch`、`SubTabSwitch`、`Switch`、`ShowKey`）、战斗 UI（`DisplayLibra`）、事件（`Event_AutoPlay`、`Event_Backlog`、`Event_Skip`、`Event_TalkSkip`、`Event_SystemMenu`、`EventPause_Resume`、`EventPause_Skip`）、队伍/装备/地图/聊天等 | 挂载在 `CPP_GameWidgetManagerSubsystem.m_GameWidgetInputManager` 下，是 Enhanced Input 驱动 CommonUI/EnhancedUI 界面的总入口；其 `InputBlockSet` 和 `CPP_BFL_UI_Input:IsBlocked` 决定 UI 是否屏蔽输入 | 高 | 标题、菜单、对话、战斗 |
| `CPP_MapUnit_Playable` / `CPP_BtlInput` / `CPP_InputRegistrant` | 野外回调：`InputMove`、`InputDirection`、`InputDash`、`InputInteract`、`InputAirship_Direction` 等；战斗回调：`OnAutoBattle`、`OnChangeChangeGameSpeed`、`OnChangeTimelineAdvance`/`Back`、`OnOpenPauseMenu`、`OnSkipSkill`；`CPP_InputRegistrant.m_BindHandleList: TMap<FString, int32>` | 分别绑定野外角色控制与战斗快捷操作；`CPP_InputRegistrant` 记录当前实际绑定在 `UEnhancedInputComponent` 上的动作名与句柄 | 中 | 野外、战斗 |
| 已知可改键映射名（来自 `CDT_OP_InputSetting_ButtonList`） | 样本包括 `MapUnit_Interact`、`MapUnit_Dash`、`MapUnit_LeftAnalogStickPush`、`Battle_SkillSkip`、`UIBattle_DisplayLibra`、`Chat_Decide`、`Shop_SwichAccessory`、`UIPartyHeal_OmakaseCure`、`UIQuest_Location`、`UIWorldMap_JumpToCurLocation`、`UIWorldMap_MapList` | 来自 `OP_BaseMenu_InputSettingList.MappingNames`，印证了游戏输入动作/映射采用 `<上下文>_<动作>` 的命名规律 | 中 | 真机核对完整 IA 列表 |

- **当前空白（R4 前必须补齐）**：
  - 标题画面的 `.jmap` 中未出现 `/Game/Input/` 下具体的 `InputMappingContext` / `InputAction` 资产实例（仅有引擎 CDO），且 R2 初次导出范围仅在 `Content/Datatable/`，暂未导出 `/Game/Input/DataAsset/DT_InputData` 与其引用的 `CPP_DA_MUCHA_INPUT` 资产。
  - **补齐方式**：（1）在 R1 真机快照里读取 `CPP_InputManager.m_pInputRegistrant.m_BindHandleList` 以及 `LocalPlayer` 的 `UEnhancedInputLocalPlayerSubsystem` 当前已注册的 `InputMappingContext` / `InputAction` 实例名；（2）在 R2/R4 按需导出 `/Game/Input/` 下的 `DT_InputData` 与 `DA_*`，拿到全部 `eMuchaInputMappingType` → `IMC` → `IA` 精确名字，再决定 R4 是向 `UEnhancedInputLocalPlayerSubsystem::InjectInputForAction` 注入还是在 `CPP_InputManager` 层注入。
  - 同时注意 `CPP_InputManager.bSwapDecideButton`（手柄确认/取消对调开关），R4 应尽量注入逻辑 `InputAction`（如确认/取消对应 IA）而非硬编码物理键位。

---

## 下一步真机快照核对清单

按 `docs/r1-dump.md` 的流程，依次在下列场面跑 `dump-run snapshot --label <场面名>` 核对本草稿：

1. **`title`（标题画面）**：核对 `CPP_MuchaSaveDataManager.GameSaveDataSummaryMap`（确定固定空槽位与自动存档槽编号）、`CPP_GameWidgetManagerSubsystem.m_GameLayout` → `CPP_WBP_MenuBase.FocuedItemDataMap` 光标链路、标题下的 `CPP_InputRegistrant.m_BindHandleList`。
2. **`field`（野外可自由移动时，移动前后各一次，靠近 NPC/宝箱一次）**：核对 `CPP_MuchaPlayerController` 的 Pawn 是否为 `CPP_MapUnit_Player`、连续坐标与 `m_Direction`、`m_InteractMapUnitList` / `m_pInteractMapGimmickList`、`CPP_MuchaGameSaveData` 的队伍/金钱/地图/剧情进度字段、`CPP_MapManager` 地图与子关卡状态。
3. **`dialog`（剧情对话中 + 含选项弹窗时）**：核对 `CPP_TalkEventSubsystem.m_pEventManager` → `CPP_WBP_TalkEvent` / `CPP_WBP_TalkWindow` 说话人与文本、`CPP_EU_WBP_DialogChoice.m_ChoiceData` 与焦点下标。
4. **`battle`（轮到我方选指令时 + 选目标时 + 战斗结算时）**：核对 `CPP_UI_BattleSubsystem`、`CPP_BtlUnitManager` 敌我列表、`CPP_VM_BtlTimeLine` 行动顺序、`CPP_VM_BtlCmd` / `CPP_VM_BtlCmdUnitSelect` 指令与目标光标。
5. **`menu`（主菜单及二级子页）**：核对 `CPP_MainMenuSubsystem`、`CPP_VM_MainMenu.SceneDataStack`、`CPP_WBP_MenuBase.FocuedItemDataMap`、`CPP_VM_UnitParam`。
6. **`shop`（城镇商店买入/卖出界面）**：核对 `CPP_WBP_ShopLayoutBase`、`CPP_VM_Shop_TradeWindow` 及买卖前后金钱/背包/限购数变化。
7. **`save`（存档点或读档界面）**：核对 `CPP_VM_SaveData`（`SlotIndex`、`bIsAutoSave`）与 `CPP_VM_SavePoint`。
