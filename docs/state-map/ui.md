# 状态地图：对话、选项、菜单、各类界面与输入

本文基于 R1 真机快照（`.deps/r1/` 下序章至第 1 章前期的 98 份快照，含成对对照与 `auto-01..43` 自动快照）、用户时间标记（`markers.log`）、反射数据（`20261124-125820-title-jmap` 与 `20261125-battle-jmap`）及 R2 数据表（`docs/data-tables.md`），定稿《FINAL FANTASY RESONANCE》UI 与输入主题的状态读取规范。

> [!NOTE]
> 置信度约定：**高**＝经至少两组对照快照（状态/光标/页签前后变化）实证；**中**＝仅单份快照出现无对照，或结构已由真机快照部分验证但核心属性位于 `FfrDump` 未导出的引擎/插件包（见 §0.3）；**低**＝仅来自静态反射或数据表推断，尚未抓到对应真机场面。

---

## 0. 全局 UI 架构与两个关键避坑点

### 0.1 CommonUI 12 层界面栈（`WBP_OverAllLayout_C`）

整个游戏的 UI 均挂载在单一视口根控件 `WBP_OverAllLayout_C`（C++ 类 `CPP_WBP_PrimaryGameLayout`，继承 `/Script/EnhancedUI.CPP_EU_WBP_GameLayout`，由 `CPP_GameWidgetManagerSubsystem.m_GameLayout` 持有）之下。结合 `DT_GameLayoutWidget_*.json` 与快照对象树，该根控件包含 12 个 `CPP_EU_ActivatableWidgetStack`（继承 `/Script/CommonUI.CommonActivatableWidgetContainerBase`）图层栈：

| 图层 `GameplayTag`（`DT_GameLayoutWidget_*`） | `WBP_OverAllLayout_C` 槽位属性 | 承载的代表性界面（`WBP_*`） |
| --- | --- | --- |
| `UI.Layer.Game` | `Stack_Game` | `WBP_TitleLayout_C`、`WBP_FieldLayout_C`、`WBP_BattleLayout_C`、`WBP_Btl_ResultLayout_C`、`WBP_Btl_Gameover_C` |
| `UI.Layer.Game.Front` | `Stack_Game_Front` | `WBP_BtlCinematicLayout_C`（战中演出叠层） |
| `UI.Layer.Event` | `Stack_Event` | `WBP_TalkEventLayout_C`（含事件对话、字幕与事件选项）、`WBP_Backlog_C`、`WBP_TalkEvent_Pause_C` |
| `UI.Layer.System` | `Stack_System` | `WBP_MainMenuLayout_C`、`WBP_SystemMenuLayout_C`、`WBP_SHOP_Layout_C`、`WBP_SavePointLayout_C`、`WBP_INNLayout_C` |
| `UI.Layer.System.Front1` | `Stack_System_Front1` | `WBP_SaveLayout_C`、`WBP_LoadLayout_C`、`WBP_OptionLayout_C`、`WBP_HelpLayout_C`、`WBP_SHOP_CommandWindow_C`、`WBP_OmakaseCureLayout_C` |
| `UI.Layer.System.Front2` | `Stack_System_Front2` | `WBP_BattleSequencerLay_C`、`WBP_Option_Dialog_C` |
| `UI.Layer.Modal` | `Stack_Modal` | `WBP_DialogLayout_C`、`WBP_ChoiceDialogLayout_C`、`WBP_TutorialLayout_C`、`WBP_Btl_LibraInfoDetailLayout_C` |
| `UI.Layer.Save` / `Chat` / `ActionGuide` / `Tooltip` / `Timer` | `Stack_Save` / `Stack_Chat` / `Stack_ActionGuide` / `Stack_Tooltip` / `Stack_Timer` | 自动存档提示、Active Time Chat、底部按键提示条、悬停提示框、限时计时器 |

除 `WBP_OverAllLayout_C` 外，仅有少量全屏覆盖控件直接 `AddToViewport()`（`IsInViewport() == true`）：常驻的 `WBP_FadeScreen_C`、`WBP_DBG_OverAllLayout_C`，以及按需创建的 `WBP_MovieScreen_C`（过场影片）、`WBP_BtlChangeSubSpace_C`、`WBP_Btl_Logo_FullBreak_C`。

### 0.2 避坑点 1：CommonUI 控件对象池残留（绝不可靠“对象存在”或 `Visibility` 判开关）

> [!WARNING]
> **经 98 份快照实证（高）**：绝大多数页面控件（`WBP_DialogLayout_C`、`WBP_ChoiceDialogLayout_C`、`WBP_TutorialLayout_C`、`WBP_SystemMenuLayout_C`、`WBP_SHOP_Layout_C`、`WBP_SavePointLayout_C`、`WBP_MainMenuLayout_C`、`WBP_Btl_ResultLayout_C`）**关闭后不会销毁，而是留在 `WBP_OverAllLayout_C` 的 `CommonActivatableWidgetContainerBase` 池中，且 `Visibility` 仍维持 `4 (SelfHitTestInvisible)`，并保留上次打开的文本与光标控件**！
>
> 实例证据：
> - `WBP_ChoiceDialogLayout_C` 在 `choice-strategy` 关闭后，在后续全部 62 份快照（直至 `boss-defeated`）中仍保持 `vis=4` 与 3 条策略选项原文；
> - `WBP_Tutorial_C` 在 `tutorial-break` / `tutorial-charge` / `boss-break` 中已关闭，却仍保留上一次教程的标题/正文与 `PageIndex`，而 `CPP_TutorialSubsystem.m_TutorialWidget` 已精确清为 `null`；
> - `WBP_BattleSequencerLay_C` 的 `WBP_TalkBattle_Caption_C` 池化且 `vis=2 (Hidden)`，内部常驻日文设计占位文本；
> - 仅 `WBP_TalkEventLayout_C` 会在非事件期把自身 `Visibility` 从 `4` 切为 `1 (Collapsed)`。

因此，判断界面开闭必须遵循**权威信号源**：
1. **运行时桥首选**：`WBP_OverAllLayout_C` 各图层栈的 `GetActiveWidget()` / `DisplayedWidget`、`GetInputTargetLayer()` 及控件的 `IsActivated()`（`bIsActive`）；
2. **专用子系统/ViewModel 指针（全部已在 98 份快照中实证）**：
   - 过场影片：`CPP_MovieSubsystem.m_pMovieScreen ~= nil`
   - 教学弹窗：`CPP_TutorialSubsystem.m_TutorialWidget ~= nil`
   - 系统确认/提示弹窗：`CPP_DialogManager.DialogWidget ~= nil`
   - 主菜单及子页面：`CPP_VM_MainMenu.SceneDataStack` 非空（且 `WBP_MainMenuLayout_C.CPP_VM_MainMenu ~= nil`）
   - 剧情/设施事件总控：`CPP_TalkEventSubsystem.m_pEventManager ~= nil`
   - 底部按键提示上下文（极佳的辅助交叉印证）：`CPP_ActionGuideSubsystem.GuideTag`（在 98 份快照中随活跃界面实时切换，见各节）

### 0.3 避坑点 2：`FfrDump` 导出边界（`/Script/EnhancedUI` 与 `/Script/CommonUI` 未进快照）

`mod/FfrDump/Scripts/ffrdump/snapshot.lua` 仅导出声明在 `/Script/mucha` 与 `/Game/` 下的 `UProperty`，且 `widgets.jsonl` 单次上限 2000 条（后半段迷宫快照触顶）。因此，声明在 `/Script/EnhancedUI`（`CPP_EU_WBP_GameLayout.CurrentInputTargetLayer`/`Layers`、`CPP_EU_WBP_Activatable.IsInputTarget`/`InputTargetWidget`/`IsOpeningProcess`/`IsClosingProcess`、`CPP_WBP_MenuBase.FocuedItemDataMap`、`CPP_EU_WBP_DialogChoice.m_ChoiceData`）和 `/Script/CommonUI`（`CommonActivatableWidgetContainerBase.DisplayedWidget`/`WidgetList`、`CommonActivatableWidget.bIsActive`）的属性**在 `.jsonl` 快照中不显示**，但已由 `20261125-battle-jmap` 证实存在于运行时类布局中，UE4SS Lua 在 FfrBridge v1 中可直接读写。下文凡依赖此类属性的结论均标注“快照间接实证 + jmap 反射确认，待 Bridge v1 首次真机核验”。

---

## 1. 对话、无框演出与剧情动画

### 1.1 事件与演出的统一判定（推翻草稿的“单指针等价”假设）

**重要修正（置信度：高）**：`CPP_TalkEventSubsystem.m_pEventManager` 并非只用于对话框——在 98 份快照中，**剧情动画、无框字幕演出、对话、剧情选项、商店、存档点、存档界面、帐篷确认框、宝箱获得提示与事件触发教学均运行在 `TalkEvent` 管线之内**（`CPP_TalkEventSequenceDirector` 内含 `ShopWait`、`SaveDataWait`、`TutorialWait`、`DialogWait` 等节点），故 `m_pEventManager ~= nil` 的准确含义是**“当前处于事件/设施交互流程中（玩家无法在野外自由移动）”**，而非“对话框开着”。

按以下分层字段可无歧义区分全部“演出/对话”形态（经 `cutscene`、`auto-01..24`、`dialog-hero`、`talk-npc`、`battle-dialog`、`before-earth-crystal`、`boss-start`、`boss-closeup-talk`、`boss-dialog`、`boss-break` 等快照实证）：

| 形态 | 判定条件 | 置信度 | 快照证据 |
| --- | --- | --- | --- |
| **FMV 过场动画**（可能带底部字幕） | `CPP_MovieSubsystem.m_pMovieScreen ~= nil`（`WBP_MovieScreen_C` 在视口内，持有 `m_pManaPlayer = MovieManaPlayer`） | 高 | `cutscene`、`auto-01..11` 全程非空；其余 86 份快照均为 `null`。其中 `auto-07` / `auto-08` 同时伴随 `WindowManager.m_pWindow` 指向 `WBP_TalkEvent_Caption_C`（影片字幕“是你们输了。”、“对不起……”） |
| **标准对话框（有框对话）** | `m_pEventManager ~= nil` 且 `WBP_TalkEventLayout_C.WindowManager.m_pWindow` 类为 **`WBP_TalkWindow_C`**（且 `WBP_TalkEventLayout_C` 可见性为 `4`） | 高 | `auto-14`、`dialog-hero`、`auto-16`、`auto-19`、`auto-24`、`boss-dialog`（战中对话同样走 `WBP_TalkEventLayout_C` + `WBP_TalkWindow_C`，不走 `WBP_BattleSequencerLay_C`） |
| **无对话框演出（镜头特写 / 字幕旁白）** | `m_pEventManager ~= nil`、`m_pMovieScreen == nil`，且 `WindowManager.m_pWindow` 类为 **`WBP_TalkEvent_Caption_telop_C`**（或 `Caption*` 家族：`Caption` / `Caption_OP` / `Caption_telopB..D`） | 高 | `boss-start` 与 `boss-closeup-talk`（用户记“无对话框，镜头拉近”）：`CPP_UI_BattleSubsystem.bDuringBattle=false`、`CPP_TaskManager.m_pTask=CPP_MapTaskMain`、`m_pEventManager` 非空、`m_pEventPlayer=CPP_TalkEventSequence`、`m_pWindow=WBP_TalkEvent_Caption_telop_C`（文本“嘎喔喔喔喔喔喔！！！”）；`auto-12`（世界撕裂旁白）同理 |
| **纯镜头 / 静默过渡演出** | `m_pEventManager ~= nil`、`m_pMovieScreen == nil`、`m_TutorialWidget == nil`、`DialogWidget == nil`、`WindowManager.m_pWindow == nil`，且**未打开商店/存档点/存档/选项**（`GuideTag == UI.System.ActionGuide.Declare.Event` 或 `CPP_TalkEventSequence.m_pSequencePlayer ~= nil`） | 高 | `auto-13`、`auto-15`、`talk-npc`（刚按交互切入镜头瞬间）、`auto-20..22`、`before-earth-crystal` |
| **战中无对话纯演出**（破防特写 / 极限技 / 召唤） | `CPP_UI_BattleSubsystem.bDuringBattle == true` 且 (`CPP_VM_Battle.bIsEvent == true` 或 `bIsDisplayBattleWidget == false` 或 `m_WorkingJobList` 非空) | 高 | `boss-break`（`m_pEventManager=nil`，但 `bIsEvent=true, bIsDisplayBattleWidget=false`）、`battle-limit`（与 `battle.md` §8 一致） |

> 注：`battle-dialog`（16:19:25）抓拍瞬间刚好叠加了战斗教学弹窗（`魔法全体化`，`CPP_TutorialSubsystem.m_TutorialWidget ~= nil`，`GuideTag = Common.Close`），此时 `WindowManager.m_pWindow == nil`，前一句雷因/拉斯韦尔对话已收起并残留在池化的旧 `WBP_TalkWindow_C` 中。这也验证了必须把 `m_TutorialWidget` 检查排在普通对话之前。

### 1.2 说话人、当前台词与“是否等待玩家按键”

在标准对话框（`WindowManager.m_pWindow` 为 `WBP_TalkWindow_C`）与字幕控件（`WBP_TalkEvent_Caption*`）上：

| 信息 | 来源对象与属性 / Getter | 置信度 | 实证与说明 |
| --- | --- | --- | --- |
| **窗口类型** | `WBP_TalkEventLayout_C.WindowManager.m_pWindow` 的运行时短类名（映射自 `WBP_TalkEventLayout_C.m_WindowClassMap`，对应 `ETalkEventWindowType` 0..6） | 高 | `0=WBP_TalkWindow_C`（标准对话框），`1=WBP_TalkEvent_Caption_C`（影片字幕），`2=WBP_TalkEvent_Caption_telop_C`（无框旁白/喊话），`3..6=Caption_OP / telopB..D` |
| **说话人** | `WBP_TalkWindow_C.Text_Name:GetText()`（`CommonTextBlock`）；或 `WBP_TalkEventLayout_C.m_pCurrentTalkData:GetSpeaker()` | 高 | `auto-14`/`auto-16`/`auto-24`/`boss-dialog` 为 `'雷因'`，`dialog-hero` 为 `'格兰谢尔特王'`，`auto-19` 为 `'格兰谢尔特士兵'`；`Caption*` 字幕窗口无说话人（空串） |
| **当前台词** | `WBP_TalkWindow_C.m_pTextBlock`（`CPP_GradualTextBlock`）渲染文本；或 `m_pCurrentTalkData:GetText()` | 高 | 全部 10 份有文本事件快照 100% 匹配游戏内显示台词；`m_pCurrentTalkData`（`CPP_TalkEventTalkData`，父 `CPP_TalkData`）无反射 UProperty，但 jmap 确认暴露 `GetSpeaker()` / `GetText()` / `GetEmotion()` |
| **是否在等玩家按键** | **`WBP_TalkWindow_C.WBP_TalkKeyWait`** 的 `Visibility`（`0 = Visible` 等按键 vs `1 = Collapsed` 未等/打字中/自动切句）+ `m_pTextBlock:IsTextFinished()` | 高 | **成对实证**：同一 `WBP_TalkWindow_C` 实例上，`auto-14` / `auto-16` / `auto-24` 为 **`0 (Visible)`**，而 `dialog-hero` / `auto-19` 为 **`1 (Collapsed)`**；`Caption*` 字幕窗口不含 `WBP_TalkKeyWait`（纯自动推进，不需按键）。`CPP_GradualTextBlock:IsTextFinished()` 已在 jmap 确认，可在 Bridge v1 复核“逐字打印完毕” |
| **自动播放 / 跳过开关** | `WBP_TalkEventLayout_C.{IsAutoPlay, CanSkip, CanTalkSkip}` 及 `WBP_TalkWindow_C.WBP_TalkKeyAutoPlay` | 中 | `auto-14..24` 中 `WBP_TalkKeyAutoPlay` 均存在；具体控制模式对应枚举 `ETalkEventControlMode`（`User` / `Program` / `ProgramSkippable`） |
| **当前事件 ID** | `CPP_BFL_TalkEvent:GetCurrentEventId()`（静态蓝图库函数）或 `CPP_TalkEventSequence.m_pSequenceActor` 资产名 | 中 | `m_pSequenceActor` / `DirectorBP` 实证可区分事件（如 `SEQ_OP_MV`、`SEQ_Ev_C01_008_02`、`SEQ_SubEv_C01_008_Battle`）；`GetCurrentEventId()` 已在 jmap 确认签名，待 Bridge v1 验收 |

### 1.3 运行时台词与 R2 数据表的完整对应链（置信度：高）

全部抓拍台词均 100% 命中离线导出数据表，链路如下：

```
运行时 EventId / SequenceDirector（如 C01_003_01_B / SubEv_C01_008_Battle）
  → Datatable/Event/TalkEvent/CDT_TalkEventData_Prod.json[EventId].TalkEventDataTable
  → Datatable/Event/Text/<章>/<DT_*_TalkEvent>.json[RowKey]
      (含 WindowType, Speaker, Emotion, WindowPosition, ContentText: {TableId, Key})
  → Datatable/Strings/Event/.../<ST_*>.json (源语言 ja)
  → Localization/zh-Hans.json[TableNamespace][Key] (简体中文译文，与快照文本 100% 一致)
```

实证抽样核对：
- `auto-14`（“格兰谢尔特骑士团，飞空艇部队队长，雷因参见陛下。”）→ `CDT_TalkEventData_Prod["C01_003_01_B"]` → `DT_C01_003_01_B_TalkEvent["003_01_B_001"]` → `ST_C01_003_01_B:003_01_B_001`（`WindowType = Normal`）；
- `auto-12`（“就这样，世界被撕裂……”）→ `ST_C01_002_MV:002_MV_001`（`WBP_TalkEvent_Caption_telop_C`）；
- `boss-start` / `boss-closeup-talk`（“嘎喔喔喔喔喔喔！！！”）→ `CDT_TalkEventData_Prod["C01_008_02"]` → `ST_C01_008_02:008_02_001`（`WBP_TalkEvent_Caption_telop_C`）；
- `boss-dialog`（“表皮很厚呢……一般的斩击恐怕对付不了。”）→ `CDT_TalkEventData_Prod["SubEv_C01_008_Battle"]` → `ST_SubEv_C01_008_Battle`（`WBP_TalkWindow_C`）；
- `battle-dialog` 残留前句（“用雷之魔法来对付机械……”）→ `CDT_TalkEventData_Prod["C01_001D_01"]` → `DT_C01_001_05_TalkEvent` → `ST_C01_001_05:001_05_002`。

---

## 2. 选项（剧情选项 vs 系统确认弹窗）

草稿 §5 留下悬念：“对话选项出现时是挂在 `CPP_WBP_TalkEvent.WidgetStack` 还是 `CPP_DialogManager.DialogWidget`？”真机快照证实：**两者并存，分属两套清晰路径，但底层选项列表共用同一个 `WBP_DialogChoice_C`（`/Script/EnhancedUI.CPP_EU_WBP_DialogChoice` → `CPP_WBP_MenuBase`）**。

### 2.1 两种选项容器的精确区分

| 维度 | 剧情事件选项（`choice-strategy`） | 系统/道具确认弹窗（`tent-confirm-no` / `tent-confirm-yes`、`got-equipment`） |
| --- | --- | --- |
| **挂载位置** | `WBP_TalkEventLayout_C.WidgetStack`（`Stack_Event` 内部栈） | `CPP_DialogManager.DialogWidget`（`Stack_Modal` 图层，由 `CPP_GameWidgetManagerSubsystem.m_DialogManager` 持有） |
| **开闭判定** | `m_pEventManager ~= nil` 且 `WBP_TalkEventLayout_C.WidgetStack:GetActiveWidget()` 非空（类为 `WBP_ChoiceDialogLayout_C` / `WBP_TalkEvent_ChoiceDialogLayout_C` / `WBP_TalkEvent_ChoiceDialogBottomLayout_C`，见 `m_ChoiceDialogClass` 映射 0/1/2） | **`CPP_DialogManager.DialogWidget ~= nil`**（98 份快照严格仅在 `chest2-after`、`got-equipment`、`tent-confirm-no`、`tent-confirm-yes` 四份弹窗中非空，其余 94 份均为 `null`）；同时 `DialogDelData.IsDecidedItem == false, DecidedItemIndex == -1`（关闭后翻为 `true` 与选中下标） |
| **窗口与标题信息** | `WBP_ChoiceDialogLayout_C`（`DialogBoxInfo.Message` 为空，题干在上一句 `WBP_TalkWindow_C`） | `WBP_DialogLayout_C.DialogBoxInfo`：`DialogTag`（如 `UI.Dialog.Box.Item.Use` / `SaveData.ConfirmSave` / `Item.Obtain`）、`ChoiceTag`（如 `UI.Dialog.Choice.YesNo` / `OK`）、`Message`（如“确定要使用吗？”）；若有自定义体则挂在 `WBP_DialogBox_C` 下（如 `WBP_DialogCustomContent_UseItem_C` 显示“帐篷”、持有 `3`、消耗 `1`） |
| **选项条目位置** | `WBP_ChoiceDialogBox_C` → `WBP_DialogChoice_C` → `DirectionGridPanel` → 3× `WBP_DialogChoiceEntry_C`（“我来打头阵…”/“以调查为优先…”/“合作最重要…”，对应 `DT_DIalogChoice[UI_DialogChoice_C01_006_01]`） | `WBP_DialogBox_C` → `PathMenu (WBP_DialogChoice_C)` → `DirectionGridPanel` → 2× `WBP_DialogChoiceEntry_C`（`0="是"`、`1="否"`，对应 `DT_DIalogChoice[UI_DialogChoice_YesNo]`） |
| **ActionGuideTag** | `UI.System.ActionGuide.Declare.Event` | `UI.System.ActionGuide.Declare.Common.Cancel` |

> **关键数据表印证**：`DT_DIalogChoice.json` 中 `UI_DialogChoice_YesNo` 明确配置 `ChoiceTextList = ["是", "否"]`、**`DefaultIndex = 1`、`CancelIndex = 1`**——这直接解释了为什么 `tent-confirm-no` 打开时默认光标在“否”（下标 `1`）；而 `UI_DialogChoice_C01_006_01` 配置 `DefaultIndex = 0, CancelIndex = -1`（不可取消）。

### 2.2 选项光标的读取方式（`tent-confirm-no` vs `tent-confirm-yes` 差异说明）

- **快照现象**：比较 `tent-confirm-no`（光标在“否”）与 `tent-confirm-yes`（光标移到“是”），在 `objects.jsonl` 和 `texts.jsonl` 中**无任何 `/Script/mucha` 属性差异**。原因已定位（§0.3）：
  1. `WBP_DialogChoice_C` 继承 `/Script/EnhancedUI.CPP_EU_WBP_DialogChoice` → `/Script/EnhancedUI.CPP_WBP_MenuBase`，其光标状态 **`FocuedItemDataMap`** 与选项数组 **`m_ChoiceData`** 均声明在 `/Script/EnhancedUI` 下，被 `FfrDump` 过滤；
  2. 每个 `WBP_DialogChoiceEntry_C` 的高亮切换（`BP_OnIsFocused`）操作的是 `Image_Select` / `Image_UnSelect`（`UMG.Image`，不进 `widgets.jsonl`）。
- **Bridge v1 读法（置信度：中，jmap 签名已确认）**：对活跃的 `WBP_DialogChoice_C` 直接读取 **`m_ChoiceData.ChoiceList`**（全部选项文本数组），并调用接口 **`CPP_BI_MenuBase:GetFocusedItemIndex(0)`**（或直读 `FocuedItemDataMap[0].Index`），兜底检查各 `WBP_DialogChoiceEntry_C` 下 `Image_Select:GetVisibility()`。

---

## 3. 系统菜单与主菜单

### 3.1 必须严格区分的两套菜单（快照实证纠正命名）

用户报的“系统菜单”（`main-menu`、`main-menu-2`）对应的是 **`WBP_SystemMenuLayout_C`**（保存/读取/帮助/选项/返回标题界面/结束游戏），而游戏内“道具/装备/恢复/任务/世界地图”归属另一套 **`WBP_MainMenuLayout_C`**（主菜单）。两者均压入 `Stack_System`（`UI.Layer.System`），但控制器完全独立：

| 维度 | 系统菜单（`main-menu` / `main-menu-2`） | 主菜单及其子页面（`quest-list` / `item-use-*` / `world-map-m` / `quick-heal-h`） |
| --- | --- | --- |
| **根控件** | `WBP_SystemMenuLayout_C`（含 `WBP_SystemMenu_C`） | `WBP_MainMenuLayout_C`（内部持有子页面栈 `CPP_EU_ActivatableWidgetStack_51`） |
| **控制器 / VM** | `VM_SystemMenu_C`（`EnableEntryList` 6 项：`Save`/`Load`/`Help`/`Option`/`Title`/`QuitGame`；`ExecuteEntryTag`） | `CPP_MainMenuSubsystem`（`WorldSubsystem`：`IsOpenMenu()`、`GetCurrentScene()`、`bIsTransitioning`）+ `CPP_VM_MainMenu`（`SceneDataStack: TArray<GameplayTag>`、`PrevSceneTag`） |
| **开闭判定（高）** | `Stack_System:GetActiveWidget()` 类为 `WBP_SystemMenuLayout_C`（注：`VM_SystemMenu_C` 在打开后仅活跃于系统菜单期间，但在 `auto-41`/`battle2-start`/`quest-list` 也曾短暂存活于内存，切勿只看对象存在） | **`CPP_VM_MainMenu.SceneDataStack` 长度 > 0**（98 份快照严格仅在 `quest-list`=`[Journal]`、`item-use-before/after`=`[Item]`、`world-map-m`=`[Map]`、`quick-heal-h`=`[Heal]` 4 组中非空；关闭后立即清空为 `[]` 且 `WBP_MainMenuLayout_C.CPP_VM_MainMenu` 置 `null`） |
| **ActionGuideTag** | `UI.System.ActionGuide.Declare.Common.Cancel` | 随子页变化：`MainMenu.Journal` / `MainMenu.Item` / `MainMenu.WldMap` / `MainMenu.Party.Heal` |

### 3.2 `main-menu` vs `main-menu-2` 光标下移实锤（置信度：高）

对比 `main-menu`（光标在索引 0“保存”）与 `main-menu-2`（光标下移一格到索引 1“读取”），快照捕捉到了完整的焦点变化证据：

1. **旧焦点项 `Save`**（`WBP_SystemList_Item_C` 含 `EntryType = UI.System.SystemMenu.Entry.Save`）：其下的手指光标控件 **`WBP_FingerCursor_C_2147464047` 的 `Visibility` 从 `3 (HitTestInvisible)` 变为 `1 (Collapsed)`**；
2. **新焦点项 `Load`**（`EntryType = UI.System.SystemMenu.Entry.Load`）：
   - `WBP_LeftLineButton_C.FocusImage` 从 `null` 变为 `Image_LeftLine_Focus_C`；
   - `WBP_ListEntryButtonBase_C.CursorWidget` 从 `null` 变为新建的 `WBP_FingerCursor_C_2147427194`，且该光标 `Visibility == 3 (HitTestInvisible)`。
3. **避坑细节**：旧焦点项 `Save` 上的 `CursorWidget` 与 `FocusImage` 指针在失焦后**不会清回 `null`**，仅通过将 `WBP_FingerCursor_C` 设为 `Collapsed (1)` 来隐藏。因此若经子控件兜底找光标，必须判 **`CursorWidget:IsValid() and CursorWidget:GetVisibility() ~= Collapsed`**，而首选仍是读其父菜单（`CPP_WBP_MenuBase`）的 `GetFocusedItemIndex(0)` / `FocuedItemDataMap`。

### 3.3 菜单栈与全局焦点链路（jmap 确认，置信度：中）

通用读栈/焦点链路（覆盖标题、系统菜单、主菜单、确认框等几乎全部 `EnhancedUI` 界面）：
1. **顶层活跃图层**：`layout = CPP_GameWidgetManagerSubsystem.m_GameLayout` → `layer = layout:GetInputTargetLayer()`（或读 `layout.CurrentInputTargetLayer`）；
2. **层内栈顶界面**：`top = layer:GetActiveWidget()`（或 `layer.DisplayedWidget`），确认 `top:IsActivated()` 且 `not top.IsOpeningProcess` 且 `not top.IsClosingProcess`；
3. **当前焦点子控件**：沿 `top.InputTargetWidget`（`TWeakObjectPtr<UWidget>`）向下解引用，到达承载列表的 `CPP_WBP_MenuBase`（或经 `PathMenu`）；
4. **光标下标**：调用 `menu:GetFocusedItemIndex(0)`（或读 `menu.FocuedItemDataMap[0].Index`）。

> 尚缺快照：主菜单根目录页面（`UI.MainMenu.Scene.Root`，即按主菜单键打开的 6 宫格/侧栏总菜单，而不是快捷键直达子页）在本次 98 份快照中未出现。

---

## 4. 商店

### 4.1 快照时间线勘误与对照依据

商店快照覆盖了土之神殿入口道具店（`ShopID = 2`，对应 `DT_ShopList` 货单 `[1 恢复药, 24 烟雾弹(限购3), 500 帐篷]`）的 8 份连续快照。**需特别说明**：用户于 01:38:46 报“购买界面”（`shop-buy`），但抓拍瞬间游戏实际已切入**出售模式**（`IsBuy=false`、`Text_Title='出售'`、`CurrentItemList=[1,3,500,4]` 为玩家背包消耗品、`Price=sellingPrice`）；真正的**购买模式**被 24 秒前的 `rest-shop`（16:38:22）完整捕获。完整对照如下：

| 快照 | `VM_Shop_TradeWindow.IsBuy` | `Text_Title` | `CPP_VM_Colosseum_Shop.SceneTypeStack` | `CategoryTab.CurSelectedIdx` | `CurrentItemList` | `CmdList.FocuedIndex` | `TradeList.FocuedIndex` | `VM_Tooltip.ActionKey.ID` |
| --- | --- | --- | --- | ---: | --- | ---: | ---: | ---: |
| `rest-shop` | **`true`** | `'购买'` | `[Command, Shop]` | `0`（常用） | `[1, 24, 500]` | **`0`**（购买） | **`0`** | `1`（恢复药） |
| `auto-27` | `false` | `'出售'` | `[Command, Shop]` | `0` | `[12000]` | `1`（出售） | `3` | `12000` |
| `shop-buy` | `false` | `'出售'` | `[Command, Shop]` | **`1`**（消耗品） | `[1, 3, 500, 4]` | `1` | **`3`** | `4`（不死鸟之尾） |
| `auto-28` | `false` | `'出售'` | `[Command, Shop]` | `1` | `[1, 3, 500, 4]` | `1` | `3` | `4` |
| `shop-sell` | `false` | `'出售'` | `[Command, Shop]` | **`4`**（饰品） | `[12000]` | `1` | **`0`** | `12000`（体力耳环） |
| `auto-29` | `false` | `'出售'` | `[Command, Shop]` | **`0`**（常用） | `[12000, 11000]` | `1` | **`3`** | `11000`（防具） |
| `shop-sell-2` | `false` | `'出售'` | `[Command, Shop]` | `0` | `[12000, 11000]` | `1` | **`0`** | `12000`（体力耳环） |
| `auto-31` | `true`（退回/关闭） | `'购买'` | **`[Command]`** | `0` | `[1,3,500,4,11000,12000]` | `0` | `0` | `1` |

### 4.2 确认的高置信度商店属性

1. **是否处于商店 & 所处层级（高）**：
   - 根界面在 `Stack_System` 栈顶为 `WBP_SHOP_Layout_C`（退出时 `WBP_SHOP_Layout_C.CPP_VM_Colosseum_Shop` 立即解绑为 `null`，实证见 `auto-31`；在交易窗口内 `GuideTag` 为 `UI.System.ActionGuide.Declare.Shop` 或 `.Shop.Trade`）。
   - **推翻草稿 §7 判断**：**`CPP_VM_Colosseum_Shop` 并非斗技场专用，普通城镇/迷宫道具店也在用它管理外层场景栈！**（实证 `ShopID=2, ShopType=eShopType::ToolShop`、`SceneTypeStack`：`[Command]` 表示买/卖一级命令菜单，`[Command, Shop]` 表示已进入商品列表交易窗）。注意其自身的 `IsBuy` 字段始终为 `false`，买/卖模式**不要读它的 `IsBuy`**。
2. **买/卖模式与商店 ID（高）**：
   - 读取 `MVVMViewModelCollectionObject` 下的蓝图实例 **`VM_Shop_TradeWindow_C`**（名 `VM_Shop_TradeWindow`）：`IsBuy: bool`（`true` 买 / `false` 卖）、`ShopID: int32`、`CurrentTotalAmount: int32`、`SelectItemList`（已加购/待售清单）、`TradeItemData`。
   - **避坑**：`WBP_SHOP_Layout_C` 下另有一个 C++ 默认实例 `CPP_VM_Shop_TradeWindow`（`ShopID=-1, CurrentItemList=[]`，仅 `ShopEntryInfo.ShopID=2`），切勿找错实例，应认准蓝图类 `VM_Shop_TradeWindow_C`。
3. **当前页商品列表与价格/限购（高）**：
   - `VM_Shop_TradeWindow_C.CurrentItemList: TArray<int32>` 给出当前分类页的道具 ID 列表；
   - `VM_Shop_TradeWindow_C.ItemInfoList: TArray<SHOP_ItemInfo>` 给出每件商品的完整运行时数据：`ID`、`ItemName`、`Description`、`ItemNum`（当前持有数）、`buyingPrice`、`sellingPrice`、`BuyCountLimit`（`-1` 不限购，烟雾弹为 `3`）、`Stock`（剩余可购数，`-1` 无限，烟雾弹为 `3`）。列表项控件 `WBP_SHOP_ItemData_C` 亦镜像 `Price`（自动按买/卖填入单价）与 `ItemData`。
4. **分类/翻页（用户 Q/E 键）（高）**：
   - `WBP_SHOP_TradeWindow_C` 下的 **`WBP_InventoryCategoryTab_C`**：`CategoryList` 固定 5 项（`0=Procedure` 常用/全部、`1=Consumable` 消耗品、`2=Weapon` 武器、`3=Armor` 防具、`4=Accessory` 饰品），**`CurSelectedIdx: int32`** 在成对快照中精确随 Q/E 翻页变化（`0 → 1 → 4 → 0`）。
5. **光标位置（用户上下键）（高）**：
   - 一级买/卖菜单：`WBP_SHOP_CommandWindow_C`（根层那个）→ `WBP_SHOP_CommonLists` → `WBP_SHOP_MainList` → **`WBP_OptionList_C.FocuedIndex`**（注意源码拼写缺少 `s`：`0`=购买、`1`=出售）；
   - 商品列表光标：`WBP_SHOP_TradeWindow_C` → `WBP_SHOP_CommandWindow` → `WBP_SHOP_CommonLists` → `WBP_SHOP_MainList` → **`WBP_OptionList_C.FocuedIndex`**（实证 `0 → 3 → 0 → 3 → 0`），且与单例 **`VM_Tooltip_C.ActionKey.ID`** 同步追踪当前高亮道具 ID（注：列表切换分类后若新列表较短，旧 `FocuedIndex` 可能短暂大于 `len-1` 如 `auto-27` / `auto-29`，此时以 `VM_Tooltip_C.ActionKey.ID` 或 `VM_Shop_TradeWindow_C:GetFocusItemID(idx)` 互验更稳）。

---

## 5. 其他界面

### 5.1 教学弹窗（`chest-after`、`tutorial-p2`、`battle-dialog`、`tutorial-break`、`tutorial-charge`、`tutorial-break-bonus`）

- **识别方式（高）**：**唯一权威判据为 `CPP_TutorialSubsystem.m_TutorialWidget ~= nil`**（或调 `CPP_BFL_Tutorial:IsWidgetActive()`）。6 份快照严格对照：`chest-after` / `tutorial-p2` / `battle-dialog` / `tutorial-break-bonus` 非空；刚关闭的 `tutorial-break` / `tutorial-charge` / `boss-break` 均为 `null`（尽管池化的 `WBP_Tutorial_C` 仍保留上一条文案）。底部 `GuideTag` 在单页或末页时为 `UI.System.ActionGuide.Declare.Common.Close` 或 `.Common.Tutorial.FinalPage`。
- **关键内容（高）**：沿 `m_TutorialWidget`（`WBP_TutorialLayout_C`）→ `WBP_Tutorial`（`WBP_Tutorial_C`）：
  - `Data.Name: FText`（主题名，如“主菜单”、“魔法全体化”、“全面破防”）；
  - `Data.Pages: TArray<TutorialPage>`（每页含 `Caption`、`Format`、`Text`），配合 `Data.ReleaseFlagId` 对应 `DT_TutorialAsset.json`；
  - **`PageIndex: int32`**（0-based 当前页码，成对实证：`battle-dialog`/`tutorial-break-bonus`=`0` 对应 `'1/1'`，`chest-after`/`tutorial-p2`=`1` 对应 `'2/2'`，`tutorial-break`=`2` 对应 `'3/3'`）；当前页即 `Data.Pages[PageIndex + 1]`，总页数为 `#Data.Pages`（`PageIndex + 1 == #Data.Pages` 即处于可关闭的末页）。

### 5.2 任务列表（`quest-list`）

- **识别方式（高）**：`CPP_VM_MainMenu.SceneDataStack` 栈顶为 **`UI.MainMenu.Scene.Journal`**（且 `GuideTag == UI.System.ActionGuide.Declare.MainMenu.Journal`）。
- **关键内容（中，单份快照 + `field-idle` 对照）**：
  - ViewModel **`VM_Journal_C`**（单例）：`DisplayQuestCategory`（`EQuestCategory::Main(1)` / `Sub(2)` / `Mini(3)`）、`TabQuestCategoryList`、**`FocusQuestId`**（`1001`）、`FirstFocusIndex`（`0`）、**`DisplayQuestDataList`**（每项含 `ID`、`Name` 如“第1章探究的开端”、`Subtitle` 如“调查土之神殿”、`DestinationTitle` 如“土之神殿”、`Description`、`Progression`、`bIsPreferred`）；
  - 控件 `WBP_JournalLayout_C`：`MainIndex`、`QuestList`；
  - 持久化底层源（不开菜单亦可读）：`CPP_QuestSubsystem.{MainQuestId, PreferredQuestId, QuestDataMap}`。

### 5.3 H 键快速回复（`quick-heal-h`）

- **识别方式（高）**：按 H 键并非独立弹窗，而是直达主菜单的恢复子页——`CPP_VM_MainMenu.SceneDataStack` 栈顶为 **`UI.MainMenu.Scene.Heal`**（`GuideTag == UI.System.ActionGuide.Declare.MainMenu.Party.Heal`）。
- **关键内容（中，单份快照）**：
  - 控件 **`WBP_HealLayout_C`**：**`CurrentState: int32`** 与 `StateWidgetMap` 精确刻画 4 级操作子状态：`0 = WBP_CategoryList`（选分类：道具/各角色魔法）、`1 = WBP_SelectItemList`（选恢复道具）、`2 = WBP_UnitSkillList`（选恢复技能）、`3 = WBP_TargetUnitList`（`WBP_PartySelecter_C` 选目标队员），另含 `bIsAoeMode`；
  - ViewModel **`VM_HealLayout_C`**（单例）：`DisplayActionList` / `ItemActionDataList`（可用恢复道具列表，每项 `{ActionKey:{Category, ID}, TmpNum, UserUnitId}`，快照中列出恢复药 5、以太 3 等 3 项）、`UnitActionList`（可用回复魔法列表）、`UnitId`（施法者）、`TargetUnitId`、`bCanAoe`、`UseActionData`。

### 5.4 存档界面（`save-ui` 与 `title` 对照）

- **识别方式（高）**：`Stack_System_Front1` 栈顶为 **`WBP_SaveLayout_C`**（存盘，位于 `UI.Layout.SaveLoad.Save`）或 `WBP_LoadLayout_C`（读盘，`UI.Layout.SaveLoad.Load`），`GuideTag == UI.System.ActionGuide.Declare.Common.Cancel`。
- **关键内容（高）**：
  - `WBP_SaveLayout_C.PathMenu` → `WBP_SaveDataList_C`：**`IsSaveLayout: bool`**（`true`=保存界面，隐藏自动存档槽 0）、**`SlotIndexList: [1..10]`**（10 个手动槽，对应 10 个 `BP_SaveDataListItem_C`）；
  - 视口可见的 4 行 `WBP_SaveData_Entry_C` 各自绑定一个 **`VM_SaveData_C`**：含 `SlotIndex`（`1..4`）、`bIsAutoSave`、`bShouldDisplaySaveData`、`LoadStatusProperty.{bIsEmpty, bIsLoadable}`、`SaveDataSummary.{bExists, LoadStatus, CurrentMapId, MainProgress, PlayLevel, PlayTimeSec, PartyUnitIds, DateTime}` 与 `ViewData.{SlotName, ChapterName, PlaceName, DifficultyName}`（`save-ui` 实证：槽 1 有用户存档 `MapId=3001, Progress=1030`，槽 2–4 为空 `bExists=false`）；
  - **全量 11 槽权威底层源（高）**：`CPP_MuchaSaveDataManager.GameSaveDataSummaryMap` 恒含键 **`0..10`**（槽 `0` 为自动存档，槽 `1..10` 为手动槽），运行时用 `FindGameSaveDataSummary(slot, out)` / `IsValidGameSaveData(slot)` 无需打开存档界面即可读取全部槽位。当前光标经 `WBP_SaveDataList_C` 内部 `PathMenu`（`CPP_WBP_MenuBase`）读 `GetFocusedItemIndex(0)`。

### 5.5 战斗结算与成长页面（`battle-result`、`growth-page`、`level-up`、`learn-skill`、`learn-skill-2`、`boss-defeated`）

- **识别方式（高）**：`CPP_TaskManager.m_pTask` 类为 `CPP_BtlTaskWin` 且 `CPP_UI_BattleSubsystem.ResultWidget`（`WBP_Btl_ResultLayout_C`）有效且 **`ResultWidget.IsFinishPage == false`**（同时 `GuideTag == UI.System.ActionGuide.Declare.Battle.Result`）。退出结算后 `ResultWidget` 仍池化非空，但 `IsFinishPage` 变为 `true`（与 `battle.md` §2 一致）。
- **页面与子阶段区分（高）**：
  - **`WBP_Btl_ResultLayout_C.WBP_BtlResultPage_Layout.CurrentPageIndex: int32`**：
    - **`0` = 报酬页**（`WBP_BtlResultPage_RewardWindow_C`，`battle-result` 实证）；
    - **`1` = 成长/升级/习得技能页**（`WBP_BtlResultPage_GrowthWindow_C`，`growth-page` / `level-up` / `learn-skill` / `learn-skill-2` / `boss-defeated` 全部实证）；
  - **是否有升级/习得技能详情窗（高）**：`WBP_BtlResultPage_LevelUpWindow_C.IsShouldDisplay: bool` 在无升级的 `battle-result`/`growth-page` 为 **`false`**，在升级并展示新技能的 `learn-skill`/`learn-skill-2`/`boss-defeated` 为 **`true`**（此时 `WBP_Btl_Result_UnitAbilityList_C` 动态生成当前升级角色的技能表：`learn-skill` 展示雷因技能，`learn-skill-2` 切换为拉斯韦尔技能）；
  - **结算数值来源（高）**：单例 **`CPP_VM_BattleResult.BtlResultData`**（`baseExperience`/`totalExperience`、`baseMoney`/`totalMoney`、`baseAp`/`totalAp`、`enemyDataList`、`itemDataList`、`unitDataList`）。
- **避坑（高）**：`WBP_Btl_ResultLayout_C.State`、`WBP_BtlResultPage_Layout_C.CurrentPageData`、`m_PageIndex` 和 `CPP_VM_BattleResult.SceneTypeStack` 在全部 6 份结算快照中均保持默认空值（`None`/`-1`/`0`/`[]`），**不可使用**；同时未激活子控件保留设计期占位文本（如 `'EXp erhaltenaaa'`、`'99999999'`、`'あああ…'`），不可盲扫 `texts.jsonl`。

### 5.6 其他已抓拍界面速查（防止误判）

| 界面（快照） | 识别判据 | 关键数据来源 | 置信度 |
| --- | --- | --- | --- |
| **标题画面**（`title`） | `CPP_TaskManager.m_pTask == CPP_TitleTaskMain`；`Stack_Game` 栈顶 `WBP_TitleLayout_C` | `WBP_Title_ListEntry_C`（菜单项）+ `CPP_WBP_MenuBase` 光标 | 高 |
| **存档点与迷宫传送**（`savepoint`、`teleport-ui`） | `m_pEventManager ~= nil`；`Stack_System` 栈顶 `WBP_SavePointLayout_C`；`GuideTag == UI.System.ActionGuide.Declare.Shop.SavePoint` | 复用商店菜单骨架：`WBP_OptionList_C.FocuedIndex`（`savepoint`=`0`“保存”，`teleport-ui`=`2`“传送”）；项类 `WBP_SavePoint_Command_C` | 高 |
| **获得道具/装备弹窗**（`got-equipment`、`chest2-after`） | `CPP_DialogManager.DialogWidget ~= nil` 且 `DialogBoxInfo.DialogTag` 为 `UI.Dialog.Box.Item.Obtain` 等 | `WBP_DialogLayout_C.DialogBoxInfo` 及嵌入的获得物品控件文本（如“野太刀 × 1”） | 高 |
| **世界地图**（`world-map-m`，M 键） | `CPP_VM_MainMenu.SceneDataStack` 栈顶 `UI.MainMenu.Scene.Map`；`GuideTag == MainMenu.WldMap` | `VM_WldMap_C`（`CurrentIslandType`/`CurrentAreaId`/`CameraAlignment`）、`VM_WldMapLandmarkList_C.AllLandmarkIdList` | 高 |
| **野外道具页**（`item-use-before/after`） | `CPP_VM_MainMenu.SceneDataStack` 栈顶 `UI.MainMenu.Scene.Item`；`GuideTag == MainMenu.Item` | `WBP_ItemLayout_C.CurrentState`（`1`=列表、`3`=选目标队友）、`VM_ItemLayout_C.DisplayActionList` | 高 |

---

## 6. 输入：Enhanced Input 上下文、动作与键位映射（供 R4 用）

### 6.1 运行时输入管线与两个关键发现

1. **推翻草稿 §10 对 `CPP_InputManager.ActivateIMTMap` 的假设（高）**：
   - 真机快照与 jmap 证实：`CPP_InputManager.ActivateIMTMap` 的值类型是 `MulticastInlineDelegateProperty`（各映射类型激活时的事件委托回调，如键 `12`），**不是当前激活的 IMC 列表**；
   - 实时查询当前激活的 `InputMappingContext` 栈应调用 **`CPP_InputManager:GetAllIMCStackList()`**（jmap 确认无参返回 `TArray`）或读取 `LocalPlayer.PlayerController.PlayerInput`（`EnhancedPlayerInput`）上的 **`AppliedInputContexts`**。
2. **输入屏蔽门控 `WindowDeactivate`（高，对 R4 极其关键）**：
   - `CPP_GameWidgetInputManager.InputBlockSet.TokenSet` 在用户切出游戏窗口的快照（`main-menu`、`tutorial-p2`、`cmd-cursor-1`、`quick-heal-h`、`world-map-m`、`tent-confirm-*` 等）中均出现 **`"WindowDeactivate"`**，回到窗口前景时清空为空集 `[]`（`CPP_BFL_UI_Input:IsBlocked()` 可直接查询）；
   - **对 R4 的含义**：当游戏窗口失焦时，游戏层会主动屏蔽全部 UI 控件输入！R4 无论采用 OS 级按键还是 `EnhancedInput` 注入，都**必须确保游戏窗口处于激活/前景状态**，或在派发前检查 `CPP_BFL_UI_Input:IsBlocked() == false`。
3. **`CPP_InputRegistrant.m_BindHandleList`（高）**：
   - `CPP_InputManager.m_pInputRegistrant.m_BindHandleList`（`TMap<FString, int32>`）记录已绑定到 `UEnhancedInputComponent` 的动作键。快照对比发现它会随场景累加（标题 8 项 → 野外 30–38 项 → 进战斗/菜单后最多 51 项，合计观测到 42 个不同动作键），因此它适合校验“某动作是否已注册”，**不能用来判断当前激活的上下文**。
4. **手柄确认/取消互换**：`CPP_InputManager.bSwapDecideButton`（全部快照为 `false`）仅影响手柄确认/取消键位；R4 若走逻辑 `MappingName` / `InputAction` 注入则自动免疫物理键位差异。

### 6.2 `eMuchaInputMappingType` 上下文分组（33 项，jmap 确认）

- **野外探索**：`MapUnit`(1)、`MapUnitDash`(2)、`UI_Field`(12)、`UI_FieldSideMenu`(13)、`UI_MapCommon`(26)
- **通用 UI / 系统与主菜单**：`TitleMenu`(5)、`UI_Common`(15)、`UI_MainMenu`(14)、`UI_SystemMenu`(29)、`OptionMenu`(7)、`Option_InputSetting`(10)、`UI_Info`(22)、`UI_Key`(32)、`UI_TextEdit`(31)
- **主菜单各子页与设施**：`Shop`(8)、`UI_Item`(24)、`UI_Equip`(19)、`UI_PartyEdit`(20)、`UI_PartyHeal`(21)、`UI_OmakaseCure`(27)、`UI_ActionEquip`(23)、`UI_WldMap`(25)、`UI_Quest`(28)
- **战斗**：`Battle`(6)、`UI_Battle`(11)
- **事件 / 对话 / 闲聊**：`UI_Event`(16)、`UI_EventPause`(17)、`UI_Backlog`(18)、`UI_Chat`(30)、`UI_ChatList`(33)
- **小游戏 / 附加**：`Race`(3)、`RaceUnit`(4)、`SoundPlayer`(9)

### 6.3 用户实测键位与核心动作映射总表

综合 `DT_OP_KeyboardMouseButtonList.json`（46 行）、`DT_OP_KeyboardMouseSetting.json`（74 行）、`CDT_TagKeyGuide.json`（83 行，含 44 条精确 `IA_*` 资产路径）与 `CPP_InputRegistrant.m_BindHandleList`（42 键）：

| 用途 / 用户实测键 | `MappingName`（可改键映射名） | `InputAction` 资产路径（`CDT_TagKeyGuide` 实证） | `BindHandle` 键名（快照实证） | 适用上下文（`eMuchaInputMappingType`） |
| --- | --- | --- | --- | --- |
| **M — 打开世界地图** | `Map_OpenMainMenu_Map` | `/Game/Input/UI/MainMenu/InputAction/IA_Map_OpenMainMenu_Map` | `Scene_Map` | `UI_Field` / `UI_MainMenu` |
| **H — 快速回复画面** | `Map_OpenMainMenu_Heal` | *未见于 KeyGuide 表*（按同目录规约推测 `/Game/Input/UI/MainMenu/InputAction/IA_Map_OpenMainMenu_Heal`，低） | `Scene_Heal` | `UI_Field` / `UI_MainMenu` |
| **Q / E — 翻页 / 切标签**（商店分类、教学翻页） | `UICommon_PreviewPage` / `UICommon_NextPage` | `/Game/Input/UI/Common/InputAction/IA_UI_Common_TabSwitch` | `UI_Common_TabSwitch` / `UI_Common_SubTabSwitch` | `UI_Common` / `Shop` / 教学与菜单页 |
| **W / S、A / D — UI 光标与战斗目标切换** | `UICommon_MoveForwards` / `Backwards` / `Left` / `Right` | *未见于 KeyGuide 表*（2D 轴向 `IA_UI_Common_Direction`） | `UI_Common_Direction` | `UI_Common` / `UI_Battle` / `Shop` / `UI_Event` |
| **W / S、A / D — 野外角色移动** | `MapUnit_MoveForward` / `Backwards` / `Left` / `Right` | `/Game/Input/Action/Map/IA_Map_UnitMove` | `UnitMove` | `MapUnit` |
| **确认 / 调查**（`IsMappingCopy` 联动） | `UICommon_Decide` / `MapUnit_Interact` | `/Game/Input/UI/Common/InputAction/IA_UI_Common_Decide` | `UI_Common_Decide` / `UnitInteract` | `UI_Common` / `MapUnit` / `UI_Event` |
| **取消 / 返回** | `UICommon_Cancel` | `/Game/Input/UI/Common/InputAction/IA_UI_Common_Cancel` | `UI_Common_Cancel` / `UI_Common_Escape` | `UI_Common` / `UI_Battle` / `UI_Event` |
| **打开主菜单 / 任务 / 道具 / 装备 / 队伍** | `Map_OpenMainMenu_{Root,Journal,Item,Equip,Party}` | `IA_Map_OpenMainMenu_Root`、`IA_Map_OpenMainMenu_Journal`（其余同目录） | `Scene_{Root,Journal,Item,Equipment,Party}` | `UI_Field` / `UI_MainMenu` |
| **打开系统菜单** | `UICommon_SystemMenu` | `/Game/Input/UI/SystemMenu/InputAction/IA_UI_SystemMenu` | `SystemMenu_Open` / `UI_Event_SystemMenu` | `UI_Common` / `UI_Field` / `UI_Event` |
| **事件自动 / 回看 / 跳过** | `UIEvent_AutoPlay` / `UIEvent_BackLog` / `UIEvent_Skip` | `/Game/Input/UI/Event/Event/InputAction/IA_UI_Event_{AutoPlay,Backlog,Skip}` | `UI_Event_{AutoPlay,Backlog,Skip}` | `UI_Event` |
| **战斗 LB / 全体化 / 侦测 / 加速 / 跳过** | `UIBattle_Limit` / `UIBattle_AoE` / `UIBattle_DisplayLibra` / `Battle_ChangeBattleSpeed` / `Battle_SkillSkip` | `IA_UI_Battle_{Limit,AoE,DisplayLibra,DetailLibra}`、`IA_Battle_{ChangeGameSpeed,SkillSkip}` | `UI_Battle_{Limit,DisplayLibra,DetailLibra}`、`Battle_{ChangeGameSpeed,SkillSkip}` | `Battle` / `UI_Battle` |
| **商店/菜单滚动、提示、详细、一键回复** | `UICommon_ScrollUp/Down`、`UICommon_ToolTip`、`UIInfo_Detail`、`UIPartyHeal_OmakaseCure` | `IA_UI_Common_{Scroll,Tooltip}`、`IA_UI_Info_Detail`、`IA_UI_PartyHeal_OmakaseCure` | `UI_Common_{Scroll,Tooltip}`、`UI_Info_Detail` | `UI_Common` / `UI_PartyHeal` / `Shop` |

> **R4 落地提示**：
> 1. 导出目录 `Datatable/` 仍未包含 `/Game/Input/DataAsset/DT_InputData` 与 `IMC_*` 资产（所以尚未离线拿到物理默认键码表；用户按键配置在 `CPP_MuchaOptionSaveData.KBMKeyConfigData.KBMKeyConfig` 中仅存改键覆盖项，默认为空）。
> 2. R4 可选两条路线：（a）OS 级向激活的游戏窗口发键盘事件（默认键位可由运行时 `CPP_InputManager:FindCurrentMappingForSlot(MappingName, 0)` 或 `GetKeyListToInputAction(IA)` 精确反查，绝不硬编码）；（b）若需要纯引擎内动作注入，`EnhancedInputLocalPlayerSubsystem` 暴露了 `InjectInputForPlayerMapping(MappingName, RawValue, ...)` 与 `InjectInputForAction(Action, ...)`（`Action` 可由 `CPP_InputManager:GetInputAction(MappingType, ActionName)` 取得）——注意遵守硬规矩，R4 需评估是否采用 OS 级模拟按键以严格符合“模组只读 + 模拟按键”。

---

## 7. 与草稿 `docs/state-map-draft.md` 的核对汇总

| 草稿章节与候选 | 核对结论 | 实机依据与修正说明 |
| --- | --- | --- |
| §5 `CPP_TalkEventSubsystem.m_pEventManager`（单凭非空判“在对话”） | **部分推翻/修正** | 98 份快照证实商店、存档点、存档 UI、帐篷确认、开箱提示、事件教学均在 `TalkEvent` 内运行；必须再结合 `m_pMovieScreen`、`m_TutorialWidget`、`DialogWidget`、`WindowManager.m_pWindow` 细分 |
| §5 `CPP_WBP_TalkEvent` / `WBP_TalkWindow_C`（说话人、`m_pTextBlock`、等按键） | **证实并补全** | `Text_Name` + `m_pTextBlock` 实证匹配数据表；新增实证 **`WBP_TalkKeyWait` 可见性（`0` vs `1`）**判等按键，以及 **`WBP_TalkEvent_Caption_telop_C`** 识别无框演出 |
| §5 `CPP_DialogManager.DialogWidget` 与 `WBP_TalkEventLayout_C.WidgetStack` 归属 | **证实并厘清** | 两者分别承载系统确认弹窗（`WBP_DialogLayout_C`）与剧情分支选项（`WBP_ChoiceDialogLayout_C`），底层选项列表均用 `WBP_DialogChoice_C` |
| §5 `CPP_ChatSubsystem` / `CPP_BacklogSubsystem` | **尚缺快照** | 现有快照未触发野外 Active Time Chat 或打开对话回看界面 |
| §6 `CPP_GameWidgetManagerSubsystem.m_GameLayout` / `CPP_EU_WBP_Activatable` / `CPP_WBP_MenuBase.FocuedItemDataMap` | **结构证实（待 Bridge v1 直读）** | 12 层 `Stack_*` 实证存在且发现池化残留陷阱；`/Script/EnhancedUI` 与 `/Script/CommonUI` 属性被 `FfrDump` 过滤，需 Bridge v1 首次真机核对；同时通过 `main-menu` vs `main-menu-2` 用 `WBP_FingerCursor_C` 实证光标迁移 |
| §6 `CPP_MainMenuSubsystem` + `CPP_VM_MainMenu.SceneDataStack` | **证实（并纠正菜单边界）** | `SceneDataStack` 精确追踪主菜单子页（`Journal`/`Item`/`Map`/`Heal`）；但“系统菜单”走独立的 `WBP_SystemMenuLayout_C` + `VM_SystemMenu_C` |
| §7 `CPP_VM_Shop_TradeWindow`（`VM_Shop_TradeWindow_C`） | **证实** | `ShopID`、`IsBuy`、`CurrentItemList`、`ItemInfoList`（含 `buyingPrice`/`sellingPrice`/`Stock`/`BuyCountLimit`）全量实证；新增实证 `WBP_InventoryCategoryTab_C.CurSelectedIdx` 与 `WBP_OptionList_C.FocuedIndex` |
| §7 `CPP_VM_Colosseum_Shop`（草稿标低、猜仅限斗技场） | **推翻** | 普通道具店（`ShopID=2, eShopType::ToolShop`）同样用它维护 `SceneTypeStack`（`[Command]` / `[Command, Shop]`） |
| §7 `CPP_WBP_ShopLayoutBase` | **证实** | 蓝图实现 `WBP_SHOP_Layout_C`，退出时解绑 `CPP_VM_Colosseum_Shop` 为 `null` |
| §9 `CPP_MuchaSaveDataManager.GameSaveDataSummaryMap` & `CPP_VM_SaveData` | **证实** | 槽位键 `0..10`（0 自动、1–10 手动）实证；`save-ui` 实证 `WBP_SaveDataList_C.SlotIndexList=[1..10]` 与 `VM_SaveData_C` 完整摘要字段 |
| §10 `CPP_InputManager.ActivateIMTMap`（猜为激活的 IMC 列表） | **推翻** | 实为 `MulticastInlineDelegateProperty` 回调字典；激活上下文栈必须改调 `GetAllIMCStackList()` 或读 `EnhancedPlayerInput.AppliedInputContexts` |
| §10 `CPP_GameWidgetInputManager.InputBlockSet` | **证实（关键发现）** | 实测窗口失焦即插入 `"WindowDeactivate"` 屏蔽全部 UI 输入 |
| §10 `CPP_InputRegistrant.m_BindHandleList` | **证实并修正** | 实测会跨场景累加至 51 项，只能验证已注册动作名，不可作当前上下文判据 |

### 尚缺的真机快照

1. **主菜单根目录页（`UI.MainMenu.Scene.Root`）**：尚未抓到未进子页时的主菜单根视图与光标切换。
2. **读档界面（`WBP_LoadLayout_C`）与存档确认覆盖弹窗**：验证槽 0 自动存档在 UI 层的展示（`bIsAutoSave == true`）。
3. **教学弹窗第 1 页 vs 第 2 页成对快照**（`chest-after` 与 `tutorial-p2` 都在第 2 页）及教学中途 `ActionGuideTag`（非末页时）。
4. **剧情选项光标移动成对快照**（如 `choice-strategy` 光标 0 vs 1）与 **Bridge v1 直读 `/Script/EnhancedUI` 属性**（`CurrentInputTargetLayer`、`DisplayedWidget`、`FocuedItemDataMap`、`m_ChoiceData`）的真机核验。
5. **商店下单数量选择弹窗（`WBP_DialogCustom_ShopBuy`）与实际成交前后对比**。
6. **野外 Active Time Chat（`WBP_Chat_C`）与对话回看（`WBP_Backlog_C`）**，确认不误判为阻塞对话。

---

## 8. FfrBridge v1 建议字段

沿用 `mod/FfrBridge/Scripts/ffrbridge/state.lua` 的只读 0.5 秒轮询规范（`pcall` + `IsValid()`，绝不跨换图缓存裸 `UObject` 指针）。为避免与 `battle.md` / `field.md` / `party.md` 冲突，本主题字段统一收口在顶层 **`ui`** 命名空间（与 `battle.md` 共享的 `task_class` / `in_talk_event` / `in_tutorial` 由桥在一次 tick 内复用同一批 `FindFirstOf` 结果，不重复扫描）。

本主题共建议 **`13` 个 `ui` 字段/字段组**（5 个常驻轻量路由字段 + 8 个仅在对应界面活跃时填充的按需子结构），外加 **1 个按需只读查询命令**（供 R4 查输入上下文与物理键码）。

### 8.1 常驻轻量路由字段（每 0.5s 必读，共 5 项）

| # | 输出字段 | 类型 | 实例查找方式与属性 / Getter 读法 | 说明 |
| ---: | --- | --- | --- | --- |
| 1 | `ui.screen_kind` | `string` | 桥按固定优先级汇总派生：`"loading"` → `"movie"` → `"tutorial"` → `"dialog"` → `"choice"` → `"talk"` → `"cutscene"` → `"save_ui"` → `"system_menu"` → `"shop"` → `"savepoint"` → `"main_menu"` → `"battle_result"` → `"battle"` → `"title"` → `"field"` | 规划层与执行器的一级画面仲裁字段；优先级严格消除下层控件（如教学压在对话/战斗上、确认框压在存档点上）的歧义 |
| 2 | `ui.active_layer` | `string \| null` | `layout = FindFirstOf("CPP_GameWidgetManagerSubsystem").m_GameLayout` → `layout:GetInputTargetLayer()`（或 `layout.CurrentInputTargetLayer`）在 `layout.Layers` 中对应的 `GameplayTag`（回落取 `Stack_*` 槽名） | 当前实际接收 UI 输入的图层栈（需 v1 真机首验 `/Script/EnhancedUI` 可读性） |
| 3 | `ui.active_widget` | `string \| null` | 上述活跃层调用 `:GetActiveWidget()`（或 `.DisplayedWidget`）后的短类名（如 `"WBP_SystemMenuLayout_C"`）；附带检查 `IsActivated()` 且非开闭动画中 | 精确识破 CommonUI 控件池化残留 |
| 4 | `ui.action_guide_tag` | `string \| null` | `FindFirstOf("CPP_ActionGuideSubsystem").GuideTag.TagName`（`"None"` 输出 `null`） | 98/98 实证与当前活跃界面同步的高可靠轻量辅助信号 |
| 5 | `ui.input_blocked` | `string[]` | `FindFirstOf("CPP_GameWidgetInputManager").InputBlockSet.TokenSet`（兜底 `CPP_BFL_UI_Input:IsBlocked()`）；无屏蔽为空数组 `[]` | 包含 `"WindowDeactivate"` 时执行器必须先恢复游戏窗口焦点再发键 |

### 8.2 按需展开的界面子结构（仅对应界面活跃时非空，其余为 `null`，共 8 项）

| # | 输出字段 | 触发采集条件 | 子字段、实例路径与属性读法 |
| ---: | --- | --- | --- |
| 6 | `ui.movie` | `FindFirstOf("CPP_MovieSubsystem").m_pMovieScreen:IsValid()` | `{ active: true, subtitle: string \| null }`；字幕仅在 `WBP_TalkEventLayout_C.WindowManager.m_pWindow` 为 `WBP_TalkEvent_Caption_C` 时读其文本块 |
| 7 | `ui.talk` | `talkSub.m_pEventManager:IsValid()` 且非 `movie`/`shop`/`savepoint`/`save_ui` | `{ event_id: CPP_BFL_TalkEvent:GetCurrentEventId(), window_class: shortClass(win), is_cutscene: (win == nil or Caption*), speaker: string \| null, text: string \| null, waiting_key: boolean, text_finished: boolean \| null, can_skip: boolean, is_auto: boolean }`。`win = evMgr.m_pWidget.WindowManager.m_pWindow`；`waiting_key` 读 `win.WBP_TalkKeyWait:GetVisibility() == 0`（`Caption*` 为 `false`）；`text_finished` 调 `win.m_pTextBlock:IsTextFinished()`；文本优先读 `win.Text_Name` / `win.m_pTextBlock`（兜底 `m_pCurrentTalkData:GetSpeaker()/GetText()`） |
| 8 | `ui.choice` | `dlgMgr.DialogWidget:IsValid()` 或 `evMgr.m_pWidget.WidgetStack:GetActiveWidget()` 为 `*ChoiceDialog*` | `{ source: "dialog" \| "event_choice", dialog_tag: string \| null, choice_tag: string \| null, message: string, options: string[], focus_index: number \| null }`。`dialog` 从 `dlgMgr.DialogWidget.DialogBoxInfo` 读标签与 `Message`；选项列表与光标均从下级 `WBP_DialogChoice_C` 读 `m_ChoiceData.ChoiceList` 与 `GetFocusedItemIndex(0)`（兜底遍历 `WBP_DialogChoiceEntry_C` 文本与 `Image_Select` 可见性） |
| 9 | `ui.tutorial` | `FindFirstOf("CPP_TutorialSubsystem").m_TutorialWidget:IsValid()` | `{ name: string, page_index: number, page_count: number, caption: string, text: string, can_close: boolean }`。从 `m_TutorialWidget.WBP_Tutorial` 读 `PageIndex`、`Data.Name`、`Data.Pages[PageIndex+1].{Caption, Text}`；`can_close = (PageIndex + 1 >= #Data.Pages)` |
| 10 | `ui.menu` | `Stack_System:GetActiveWidget()` 为 `WBP_SystemMenuLayout_C` / `WBP_SavePointLayout_C` / `WBP_TitleLayout_C`，或 `CPP_VM_MainMenu.SceneDataStack` 非空 | `{ kind: "system_menu" \| "main_menu" \| "savepoint" \| "title", scene_stack: string[], entries: string[], focus_index: number \| null, sub_state: object \| null }`。系统菜单读 `VM_SystemMenu_C.EnableEntryList` + `GetFocusedItemIndex(0)`（兜底 `WBP_FingerCursor_C` 可见性）；主菜单读 `CPP_VM_MainMenu.SceneDataStack`，并按栈顶附带 `sub_state`：`Journal` 读 `VM_Journal_C.{DisplayQuestCategory, FocusQuestId, DisplayQuestDataList[*].{ID,Name,Subtitle}}`；`Heal` 读 `WBP_HealLayout_C.CurrentState` + `VM_HealLayout_C.{DisplayActionList, UnitId, TargetUnitId, bCanAoe}`；`Item` 读 `WBP_ItemLayout_C.CurrentState`；`Map` 读 `VM_WldMap_C.{CurrentIslandType, CurrentAreaId}`；存档点读 `WBP_OptionList_C.FocuedIndex` |
| 11 | `ui.shop` | `Stack_System:GetActiveWidget()` 为 `WBP_SHOP_Layout_C`（且其 `CPP_VM_Colosseum_Shop:IsValid()`） | `{ shop_id, shop_type, scene_stack, is_buy, category_index, categories, cmd_focus_index, item_focus_index, tooltip_item_id, total_amount, items: [{id, name, price, owned, stock, limit}] }`。分别取自 `VM_Shop_TradeWindow_C`、`CPP_VM_Colosseum_Shop`、`WBP_InventoryCategoryTab_C`、两级 `WBP_OptionList_C.FocuedIndex`、`VM_Tooltip_C.ActionKey.ID` |
| 12 | `ui.save_ui` | `Stack_System_Front1:GetActiveWidget()` 为 `WBP_SaveLayout_C` 或 `WBP_LoadLayout_C` | `{ mode: "save" \| "load", slot_indices: number[], focus_index: number \| null }`。取自 `PathMenu`（`WBP_SaveDataList_C`）的 `IsSaveLayout`、`SlotIndexList` 及内层 `CPP_WBP_MenuBase:GetFocusedItemIndex(0)`（全量 0–10 槽位摘要由 `party.md` 的按需 `save_slots` 命令提供，不进 0.5s 轮询） |
| 13 | `ui.battle_result` | `task_class == "CPP_BtlTaskWin"` 且 `uiBattle.ResultWidget:IsValid()` 且 `not ResultWidget.IsFinishPage` | `{ page_index: number, has_level_up_window: boolean }`（与 `battle.md` 的 `battle.result` 共享同一实现入口，补充 `WBP_BtlResultPage_LevelUpWindow_C.IsShouldDisplay` 区分“普通成长页”与“升级/习得技能页”） |

### 8.3 按需只读查询命令（不进 0.5s 轮询，供 R4 使用）

- **`input_state`**：仅在启动自检、R4 调试或映射异常时由宿主机主动请求，返回：
  - `active_imcs`: `CPP_InputManager:GetAllIMCStackList()`（字符串/资产名数组）
  - `swap_decide`: `CPP_InputManager.bSwapDecideButton`
  - `input_enabled`: `CPP_InputManager:IsEnableInput()`
  - `input_blocked`: `CPP_GameWidgetInputManager.InputBlockSet.TokenSet`
  - `key_bindings`: 对 R4 所需的一组 `MappingName`（如 `UICommon_Decide`/`Cancel`/`PreviewPage`/`NextPage`/`Move*`、`Map_OpenMainMenu_{Root,Map,Heal,Journal}`）调用 `CPP_InputManager:FindCurrentMappingForSlot(name, 0)` 返回当前生效的物理键名

### 8.4 更新频率、性能与对象失效防护

1. **严格按需门控**：自由野外（`ui.screen_kind == "field"`）与标题画面下，每 0.5s 仅执行 5 个常驻单例的 `O(1)` 读取，8 个子结构全部短路返回 `null`，单拍 Lua 耗时 `< 0.5 ms`。
2. **禁止 `FindAllOf` 扫控件树**：由于 CommonUI 控件池化和旧实例残留（如多份 `WBP_DialogLayout_C`、`WBP_OptionList_C`），所有子控件一律从当前活跃图层栈 `GetActiveWidget()` 或子系统单例指针链向下访问；唯一需要扫描短数组的是 `ui.menu` 系统菜单光标兜底（仅 6 项 `WBP_SystemList_Item_C`）。
3. **文本与列表体积控制**：`ui.shop.items` 仅输出当前分页（通常 1–20 项）；`ui.talk.text` 与 `ui.tutorial.text` 原样短串输出；换图检测到 `World` 变更时丢弃所有 `WorldSubsystem`（`CPP_MainMenuSubsystem`、`CPP_UI_BattleSubsystem`、`CPP_InputManager`）缓存引用，每次 tick 重新 `FindFirstOf` 并做 `IsValid()` 检查。
