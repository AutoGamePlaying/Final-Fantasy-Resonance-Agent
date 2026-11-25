# 状态地图：野外、地图和移动（R1 实机定稿）

本文基于序章至第 1 章土之神殿 Boss 战期间采集的 **98 份真机快照**、**58 份 FfrBridge v0 轨迹（共 6442 行）**、战斗场景 `.jmap` 反射数据及 R2 导出数据表，定稿野外、地图、交互、传送、大地图与寻路六大子主题的状态映射。

> [!NOTE]
> **证据与置信度约定**
> - **高**：成对快照前后变化直接证实，或跨多场景快照 / 轨迹 / 数据表一致互证。
> - **中**：仅在单份或少量快照中观测到，缺少前后对照。
> - **低**：仅见于静态反射（`.jmap`）或离线数据表，尚缺真机快照验证。
> - 本文只记录类名、属性/函数名与少量示例值；采样局限：`objects.jsonl` 仅展开 `/Script/mucha` 与 `/Game/` 类属性（引擎基类与 `/Script/EnhancedUI` 基类属性未序列化），且 `TMap<int32, Struct>` 值未展开字段。

---

## 1. 玩家：位置、朝向、移动方式、操控状态与速度

### 1.1 实机结论一览

| 状态项 | 实机读取源（类 / 属性或函数） | 示例值 / 变化规律 | 置信度 | 快照与轨迹依据 |
| --- | --- | --- | --- | --- |
| 实时 3D 位置 | `PlayerController.Pawn:K2_GetActorLocation()`（等价 `CPP_MiniMapSubsystem.PlayerLocation` / `VM_MiniMap_C.PlayerLocation`） | `world-map-m`: `(2215.7, 3968.5, -1697.8)`，三处完全一致 | 高 | 全 95 份有 Pawn 快照与 6425 行轨迹；反证：`SaveData.Map.unitLocation` 仅存盘刷新（`auto-43` 进 `Dng_01Gra_40_PL` 后仍停在旧图 `(-3862,-551,-98)`） |
| 实时朝向（8 方向与角度） | `VM_MiniMap_C.PlayerDirection`（`eMapDirectionType`）与 `PlayerDirectionDegree`（`float`）；打开大地图时 `VM_WldMap_C` 同名字段同步一致 | `field-walk`: `South / 180.0`；`rest-shop`: `West / -90.0`；`world-map-m`: `East / 90.0`；全集观测到 6 方向（`0/±45/±90/180`） | 高 | 跨 94 份含 `VM_MiniMap_C` 快照变化 + `WBP_MiniMapPlayerIcon_C.NextAngle` 同步印证；**推翻** `BP_MapUnit_Player_C.m_Direction`（95/95 恒为 `North`） |
| 最近移动输入向量 | `BP_MapUnit_Player_C.m_OldInputActionValue`（`FVector2D`） | `field-walk`: `(0,-1)`；`chest2-before`: `(0,1)`；`world-map-m`: `(1,0)`；`field-idle`: `(-0.707,0.707)` | 高 | 跨多份野外快照随上次移动键更新；但不反映原地事件转向（如 `savepoint` 为 `(-1,0)` 而角色实际朝 `South`） |
| 当前操控角色 ID | `BP_MapUnit_Player_C.m_MapUnitId` | 序章 `60`（魔人菲娜）→ 第 1 章 `10`（雷音） | 高 | `Com_00Com_45_PL`~`Event_00Eve_87_PL` 为 `60`；`auto-12`（`Town_01Gra_03_PL`）起变 `10`，吻合 `DT_MapUnitPlayableData` 条件 `{progress:0}<=1010` / `>=1010` |
| 移动载体类别 | `BP_MapUnit_Player_C.m_MapMovementMethod`（`eMapMovementMethod`） | 全 95 份快照为 `Walking (1)`；表/枚举还含 `AirShip`/`Chocobo`/`Ship`/`MagickArmor` | 中 | 城镇/迷宫实测为 `Walking`；载具与大地图移动尚缺真机快照 |
| 速度上限配置 | `BP_MapUnit_Player_C`: `m_MaxWalkingSpeed` / `m_MaxJogSpeed` / `m_MaxRunningSpeed` | 恒为 `600.0 / 750.0 / 1300.0`（对应 `DT_MapPlayableUnitMoveData` 行 `レイン`） | 高 | 95/95 快照与数据表完全一致；大地图 `Walking_Field` 为 `180/230/450`，陆行鸟 `1000/1400/2100`，飞空艇 `2000/2200/4000`，小舟 `720/600/800` |
| 冲刺开关 | `BP_MapUnit_Player_C.m_IsDash` | 全 95 份快照恒为 `true`（默认冲刺/切换设置态，**非瞬时按住冲刺**） | 高 | 95/95 恒 `true`；区分常态跑与冲刺需读 `CharacterMovementComponent.Velocity` 或位移速率 |
| 能否自由操作 | 组合条件（见 §1.3） | 仅 `field-idle`/`field-walk`/`auto-17`/`auto-25..26`/`chest-before`/`chest2-before`/`tent-after` 全满足 | 高 | 跨 27 类代表场面矩阵验证；**推翻**单独依赖 `m_IsEnableMovement`（95/95 恒 `true`）或 `m_IsEnableUnitInput`（仅战斗为 `false`） |

> [!WARNING]
> **关于 `K2_GetActorRotation()` 的采样 Bug**：`mod/FfrDump/Scripts/ffrdump/snapshot.lua` 将 `FRotator` 误传给读取 `.X/.Y/.Z` 的 `vector()`，导致全部 98 份快照的 `context.json.pawnRotation` 均为 `{x: null, y: null, z: null}`。FfrBridge v1 若需读取 Actor 旋转，必须读取 `.Pitch / .Yaw / .Roll`，并优先使用已证实的 `VM_MiniMap_C.PlayerDirection` / `PlayerDirectionDegree`（另 `CPP_MapUnit:GetDirection()` 为只读 C++ 函数，可在 v1 真机阶段复核是否与 `VM_MiniMap_C` 一致）。

### 1.2 `PlayerController.Pawn` 何时是真正操作的角色

全 95 份有 Pawn 的快照与 6425 条非空轨迹中，`PlayerController.Pawn` 始终指向 `BP_MapUnit_Player_C`。但它**并非始终代表可操控玩家的真实站位**，实机共观测到四类非自由移动情形：

1. **读条加载期（Pawn 为 `null` 或原点暂驻）**：`MUCHA_Title` 与跨图加载时（`auto-23`、`auto-42`，轨迹共 7 次、17 行报错 `pawn: 找不到有效的 Pawn`），`Pawn` 为 `null`；刚生成瞬间还会在 `(0, 0, z)` 短暂驻留 1 拍（如 `trace-01`/`07`/`10`/`11` 的 `(0,0,102.x)`、`(0,0,-268.4)`），随后瞬移至 `BP_MapTransitionPoint_C` 落点。
2. **Sequencer 过场演出候场位（藏身坐标）**：在 `trace-move-1`（`Com_00Com_45_PL`）、`auto-01..09`（`Event_00Eve_87_PL`）、`auto-12..13`（`Town_01Gra_03_PL`）及 Boss 战后 `trace-95`（`Dng_01Gra_40_PL`）中，`BP_MapUnit_Player_C` 被整体挪至固定场外坐标 **`(25408.0, 384.0, -500.0)`** 且 **`m_IsVisible == false`**；镜头中的角色实为 `CPP_TalkEventManager.m_TagBindings` 绑定的替身 `BP_MapUnit_NPC_C` 或临时生成的 `CPP_MapUnit`。
3. **战斗冻结位**：全部 25 份战斗快照（含结算与升级页）中，`BP_MapUnit_Player_C` 停留在遇敌时的野外坐标，**`m_IsVisible == false`** 且 **`m_IsEnableUnitInput == false`**。
4. **地图内脚本演出 / 对话 / 商店 / 存档点**：在 `auto-14..16`、`dialog-hero`、`talk-npc`、`auto-19`、`choice-strategy`、`rest-shop`、`savepoint`、`boss-start` 中，`BP_MapUnit_Player_C` 仍在真实地图位置且 `m_IsVisible == true`（甚至会被事件脚本以 `148–279 u/s` 自动驱动行走），此时靠 `CPP_TalkEventSubsystem.m_pEventManager != null` 识别玩家已被接管。

### 1.3 “是否能操作（`can_control`）”的精确判据

实测对比过场（`auto-02`）、地图内对话（`auto-15`/`talk-npc`）、加载（`auto-23`/`auto-42`）、战斗（`battle-command`/`boss-dialog`）、商店（`rest-shop`/`shop-buy`）、存档点（`savepoint`/`teleport-ui`/`tent-confirm-no`）、主菜单子页（`world-map-m`/`quick-heal-h`/`quest-list`/`item-use-before`）、侧边菜单（`main-menu`/`main-menu-2`）与自由野外（`field-idle`/`field-walk`/`chest-before`/`chest2-before`/`tent-after`）后，确认 `can_control` 必须同时满足：

1. **Pawn 有效且可见可用**：`Pawn != null`、`m_IsVisible == true`、`m_IsEnableUnitInput == true`、`m_IsEnableMovement == true`（并排除 `pawn_location` 处于候场坐标 `(25408, 384, -500)`）。
2. **非加载 / 非战斗 / 非事件对话 / 非系统弹窗**：
   - `CPP_LoadingScreenManager.IsShowLoadingScreen == false`
   - `CPP_UI_BattleSubsystem.bDuringBattle == false`
   - `CPP_TalkEventSubsystem.m_pEventManager == null`（实测覆盖过场动画、NPC 对话、商店、存档点菜单、开箱获得道具弹窗、地图教程）
   - `CPP_DialogManager.DialogWidget == null`（实测覆盖帐篷使用确认框 `tent-confirm-no/yes`、开箱确认框 `chest2-after`）
3. **未打开主菜单或全屏子界面**：
   - `CPP_MainMenuSubsystem:IsOpenMenu() == false`（C++ 只读函数；`CPP_VM_MainMenu.SceneDataStack` 同步反映已打开子页，实测在 `world-map-m`/`quick-heal-h`/`quest-list`/`item-use-before` 分别为 `UI.MainMenu.Scene.Map` / `.Heal` / `.Journal` / `.Item`）
   - 顶层 UI 输入层检查：`CPP_GameWidgetManagerSubsystem.m_GameLayout:GetInputTargetLayer()` 停留在野外层（用于一并拦住 `main-menu`/`main-menu-2` 对应的侧边菜单及系统菜单；因基类 `CPP_EU_WBP_GameLayout` 属性未落盘快照，需在 FfrBridge v1 真机阶段验收其返回 Tag，置信度：**中**）。
   - 另：`CPP_GameWidgetInputManager.InputBlockSet.TokenSet` 在游戏窗口失焦时会出现 `"WindowDeactivate"`，可用于检测窗口是否获得焦点。

### 1.4 轨迹测速与“1237 u/s”原因解析

对全量 58 份轨迹（6442 行、418 个移动区间，剔除 10 次瞬移跳变 `>= 2000 u/s` 后余 408 个区间）分别用**宿主机墙钟间隔 `dt_w`** 与**Lua 侧 `clock_ms` 间隔 `dt_c`** 计算速度：

| 移动模式 | 实测稳态速度（按 `clock_ms`） | 数据表基准（`DT_MapPlayableUnitMoveData`） | 证据样本 | 置信度 |
| --- | --- | --- | --- | --- |
| 剧情脚本步行 | `148 / 238 / 279 u/s` | —（事件自定义速度） | `trace-07`（11 帧均值 `148.1`）、`trace-11`（4 帧 `238.4`）、`trace-83`（5 帧 `278.5`） | 高 |
| 自由慢走（Walk） | 轨迹中未见明显稳态平台 | `Walk.speed = 600.0` | 现有操作全程默认奔跑或冲刺，`500–620 u/s` 均为启停过渡帧 | 低 |
| 常态奔跑（Jog，主峰） | **`740–760 u/s`**（2D 水平） | **`jog.speed = 750.0`** | 20 u/s 分箱 `740–760` 达 **96 个区间**；`trace-42/50/53/55/59/82/84` 连续稳态段均值 `735–752 u/s` | 高 |
| 冲刺（Dash / Run，次峰） | **`1025–1078 u/s`**（2D 水平）；斜坡 3D 合速度 **`1094–1102 u/s`** | `Run.speed = 1300.0`（实测约为其 ~80–83%） | `trace-60` 连续 6 帧 2D 均值 `1038.8`（上坡 3D `1094–1102`）；`trace-83` 连续 3 帧 `1023–1045`；`trace-95` 平地直线峰值 `1078.3` | 高（实测值）/ 低（与 1300 差值原因） |

- **为什么轨迹里曾出现 `~1237 u/s`（实测区间 `1234.7–1408.9 u/s`）**：
  - 宿主机每隔 ~0.53s 轮询 `state.json`，而 Lua 侧每隔 ~0.50s 写出一次。两者周期错拍导致偶发跳过一次写入，出现 **`dt_w ≈ 0.53s` 但 `dt_c ≈ 1.00–1.54s`** 的跨步样本；拿 1 秒的常态奔跑位移（~665–762 u）除以 0.53s 墙钟就会算出 1235–1409 u/s 的假象。
  - 典型例证：`trace-08.tsv` `00:33:26.692→27.231`（`d2=665.5, dt_w=0.539s → v_wall=1234.7 u/s`，按真实 `dt_c=1.007s` 实为 `660.9 u/s`）；`trace-59.tsv` `01:05:32.958→33.498`（`d2=760.8, dt_w=0.540s → v_wall=1408.9 u/s`，真实 `dt_c=1.009s → 754.0 u/s`）。
  - **结论**：FfrBridge v1 必须继续输出 Lua 侧单调时钟 `clock_ms`，执行器测速与位移估算一律用 `Δclock_ms` 而非宿主机接收时间。

---

## 2. 地图：标识、数据表映射、换图与命名规律

### 2.1 当前地图 ID 与 `CDT_MapData_Prod`（186 行）一一对应

- 离线核验 `CDT_MapData_Prod.json` 全部 **186 行**：每行 `Level.AssetPathName` 的短名（如 `Dng_01Gra_40_PL`）**186/186 全局唯一、无重名**，与 `ID` 形成严格 1:1 映射。
- 本次真机经历的全部 8 张游戏内地图均 100% 匹配数据表（仅 `MUCHA_Title` 不在表中，`mapId = -1`）：
  `Com_00Com_45_PL`（10040）、`Com_00Com_46_PL`（10041）、`Event_00Eve_87_PL`（5010）、`Town_01Gra_03_PL`（2013 王都格兰谢尔特）、`Town_01Gra_22_PL`（2014 格兰谢尔特城）、`Event_00Eve_86_PL`（20000 飞空艇）、`Dng_01Gra_43_PL`（3001 土之神殿入口）、`Dng_01Gra_40_PL`（3000 土之神殿内部）。
- **实时 `mapId` 与分区的最优读取源**：
  - **`CPP_MiniMapSubsystem.PropertyKey.mapId`（高置信度）**：全 98 份快照 **98/98（100%）** 与当前 `World` 对应的 `CDT_MapData_Prod.ID` 精确一致，且在读条中途（`auto-23`、`auto-42`，Pawn 尚为 `null`）就已经切到新图 ID。
  - **`CPP_MiniMapAreaManager.CurrentAreaId`（高置信度）**（经 `CPP_MiniMapSubsystem.AreaManager` 访问）：实时反映当前地图内的子区域/楼层 ID（实测 `Town_01Gra_22_PL`: `7 → 5 → 4`；`Dng_01Gra_40_PL`: `1 → 2 → 5 → 3 → 1`），与 `CPP_Map_NavMeshBoundsVolume.m_AreaId` 对应。
  - **`CPP_MapManager:GetMapId()` / `GetOldMapId()` / `GetMapAreaId()` / `GetCurrentIslandType()`（中置信度）**：`WorldSubsystem` 标准只读 getter，可在 v1 同步复核；若 `CPP_MiniMapSubsystem` 不可用（如标题图），退回 `World` 短名反查离线 `CDT_MapData_Prod`。
- **推翻 / 修正草稿的读取陷阱**：
  - **`CPP_MuchaGameSaveData.Map.mapId` 严重滞后**：仅在自动存档/手动存盘/事件结束时写回（如进入 `Dng_01Gra_40_PL (3000)` 后的 `auto-42`/`auto-43`/`chest-after`/`tutorial-p2` 仍滞留 `3001`；`Event_00Eve_87_PL` 全程滞留 `10040`），**绝不能用作实时当前地图 ID**。
  - **`VM_MiniMap_C.CurrentMapData.ID` 在过场/读条期间同样滞后**，须以 `CPP_MiniMapSubsystem.PropertyKey.mapId` 或 `World` 名为准。
  - **`CPP_MuchaWorldSetting` 多实例陷阱**：每个流式子关卡（`_BG`/`_GD`/`_EV`/`_SD`/`_LT`/`_KD`/`_NAV`）各有一个 `CPP_MuchaWorldSetting`，其 `isUseMapManager` 均为 `false`；只有 `*_PL` 持久关卡的实例才为 `true`。不可用 `FindFirstOf("CPP_MuchaWorldSetting")` 盲取，应读 `World.PersistentLevel.WorldSettings` 或按名称包含 `_PL` 过滤。

### 2.2 换图与同图切区机制

- **跨图切换（切换整个 Persistent Level）**：
  - 实测 7 次跨图（含 `Com_00Com_45_PL → Com_00Com_46_PL`、`Event_00Eve_86_PL → Dng_01Gra_43_PL`、`Dng_01Gra_43_PL → Dng_01Gra_40_PL`）均为 `World` 切向新的 `*_PL` 持久关卡，同时由 `CPP_MapSubLevelLoadManager` 流式加载该图的子关卡（`m_LoadLevelStreamingList`、`m_CompletedeLoadeLevelList`；室内外切换对应 `m_IndoorLoadLevel` / `IsCompletedLoadByIndoor()`，尚缺进出室内快照）。
  - 切换时序（轨迹 + `auto-23`/`auto-42` 快照）：旧图触发 `BP_MapTransitionTrigger_C` → `CPP_LoadingScreenManager.IsShowLoadingScreen` 变 `true` → `World` 先切新图名、`Pawn` 变 `null` 持续 **1–4 拍（0.5–2.0s）** → `Pawn` 生成（可能短暂在 `(0,0,z)`）→ 落位到目标 `BP_MapTransitionPoint_C` → `IsShowLoadingScreen` 变 `false`。
  - 判断“换图稳定完成”：`World` 与 `mapId` 已更新 + `IsShowLoadingScreen == false` + `Pawn != null` + `m_LoadLevelStreamingList` 已加载完毕（结合 `can_control` 判定）。
- **同图切区（不换 `World`）**：
  - 入口为 `BP_MapTransitionTrigger_C`（`m_IsSameMapTrasition == true`），不换 `World`，只瞬移角色坐标并更新 `CurrentAreaId`（如 `trace-07.tsv` `Town_01Gra_22_PL` 内从 `(3624, 0, 302)` 切至 `(-11148, 2780, 1282)`）。

### 2.3 地图与子关卡命名规律（全 186 行统计）

- **主关卡（Persistent Level）格式**：`<类别前缀>_<区域码>_<序号>_PL`（大地图特例为单图 `Wld_PL`，`ID = 1000`，`eMapType::Field`）。
  - **前缀分布与对应 `eMapType` / ID 范围**：
    - `Dng`（63 张，ID `3000–10066`）：迷宫与部分小神殿（`Dungeon: 43`、`VisionSmallShrine: 15`、`Town: 5`）。
    - `Com`（42 张，ID `10020–60000`）：序章战场、通用幻兵小神殿等（`VisionSmallShrine: 35`、`Dungeon: 4`、`Town: 3`）。
    - `Event`（41 张，ID `5010–20000`）：剧情专用图如战场过场、飞空艇甲板（`Town: 35`、`Dungeon: 6`）。
    - `Town`（39 张，ID `2000–20002`）：城镇与城堡（全部为 `Town: 39`）。
    - `Wld`（1 张，ID `1000`）：世界大地图（`Field: 1`）。
  - **区域码（12 种）**：`00Com`（44 通用/序章）、`00Eve`（41 事件）、`01Gra`（15 格兰谢尔特）、`02Lan`（7 兰泽尔特）、`03Kol`（2 科罗博斯）、`04Dir`（10 迪尔纳多）、`05Old`（14 奥尔德里昂）、`06Zol`（20 佐尔达多）、`07Mys`（12 密斯迪亚）、`08Gro`（9）、`09Pha`（9）、`10Cry`（2）。
- **流式子关卡后缀**（实测见于 `Town_01Gra_22`、`Dng_01Gra_43`、`Dng_01Gra_40` 等）：
  - `_PL`（持久主图）、`_BG`（地形/建筑美术）、`_GD`（玩法机关：宝箱、存档点、出入口触发器与落点、区域盒）、`_EV`（NPC 与剧情事件触发器）、`_SD`（音效）、`_LT`（按剧情进度切换的光照）、`_KD`（镜头控制盒等）、`_NAV`（导航网格包围体 `CPP_Map_NavMeshBoundsVolume`）；另遇敌时会预载战斗场景子关卡 `Btl_*_BG0`。

---

## 3. 可交互对象与“面前可交互”判定

### 3.1 玩家面前当前可交互对象（高置信度，成对快照实证）

实机存在**三条互相印证的链路**，FfrBridge v1 推荐以 Player Pawn 链路为主（直接拿到目标 Actor 指针与坐标），必要时用 UI/VM 链路兜底：

| 链路 | 类与属性 | 实机证据与变化 |
| --- | --- | --- |
| **1. Player Pawn 交互列表（推荐主源）** | `BP_MapUnit_Player_C.m_InteractMapUnitList`（`TArray<CPP_MapUnit*>`）<br>`BP_MapUnit_Player_C.m_pInteractMapGimmickList`（`TArray<CPP_MapGimmick*>`）<br>`BP_MapUnit_Player_C.m_InteractEventId.EventId` | - 靠近 NPC：`m_InteractMapUnitList` 非空（11 份快照，如 `auto-19`/`auto-26`/`rest-shop`/`shop-buy` 为 `[BP_MapUnit_NPC_C_X]`）；**修正草稿**：单指针 `m_pInteractMapUnit` 全 98 份快照恒为 `null`。<br>- 靠近机关：`m_pInteractMapGimmickList` 非空（18 份快照），且 `m_InteractEventId.EventId` 为 `"treasureBox"` 或 `"savePoint"`。<br>- **成对对比（`chest2-before → chest2-after`）**：`m_pInteractMapGimmickList` 由 `[BP_MapGimmick_TreasureBox_C_0]` 变为 `[]`，`EventId` 由 `"treasureBox"` 变为 `"None"`。 |
| **2. 交互提示 UI** | `WBP_ObjectInteract_C`：`m_IsInteract`、`m_IsOpen`、`m_InteractObjectList`（`TArray<int32>`） | - **成对对比（`chest2-before → chest2-after`）**：`m_IsInteract: true → false`，`m_IsOpen: true → false`，`m_InteractObjectList: [4] → []`。<br>- 跨 6 份交互快照核实：`m_InteractObjectList` 存的是目标 Actor 的 **`m_UniqueId`（= `TypeData.ID`）**，不是 `CPP_VM_FieldObject.ObjectId`（如 `chest-before` `[23]`、`savepoint` `[25]`、`tent-confirm-no` `[16]`、`auto-19` `[4]`、`auto-26` `[2]`）。 |
| **3. FieldObject ViewModel** | `CPP_FieldObjectSubsystem.FieldObjectVMManager.VMMap` → `CPP_VM_FieldObject`：`bInteractable`、`ObjectData.TypeData.{Type, ID}`、`ObjectData.InteractTag` | - **成对对比（`chest2-before → chest2-after`）**：`TypeData.ID=4` 的 `bInteractable` 从 `true → false`。<br>- `InteractTag` 精确标明交互类型：`Field.Object.Interact.Type.Talk` / `.Shop.Inn` / `.Shop.Tool` / `.Treasure` / `.Save` / `.Transition`。 |

### 3.2 各类可交互对象详情

#### (1) NPC（`BP_MapUnit_NPC_C` / `CPP_MapUnit_NPC`）

- **查找路径**：`CPP_MapManager.m_MapUnitStorage.m_MapUnits`（筛选 `m_MapObjectType == eMapObjectType::NPC (1)` 或类名含 `NPC`；Actor 位于 `*_EV` 子关卡）。
- **位置与状态**：坐标读 `Actor:K2_GetActorLocation()`；关键属性 `m_UniqueId`（全图唯一）、`m_MapUnitId`（外观/角色类型 ID，如 `1060`）、`m_Direction`（NPC 有效，实测 `North`/`South`）、`m_IsVisible`、`m_MapInteractType`（`Talk=1`、`Shop=17` 等）、`ShopEntry.ShopID`（`rest-shop` 中 `BP_MapUnit_NPC_C_2` 为 `ShopID=2`，普通 NPC 为 `-1`）、`InnEntry.InnID`、`m_EventList`、`m_TalkData`。
- **正在对话的 NPC**：对话触发后可从 `CPP_TalkEventSubsystem.m_pInteractTargetUnit` 直接获取（`talk-npc` 指向 `BP_MapUnit_NPC_C_3`，`auto-19` 指向 `BP_MapUnit_NPC_C_1`，`rest-shop`/`shop-buy` 指向 `BP_MapUnit_NPC_C_2`）。置信度：**高**。

#### (2) 宝箱（`BP_MapGimmick_TreasureBox_C` / `BP_MapGimmick_TreasureBox002_C` / `CPP_MapGimmick_TreasureBox`）

- **查找路径**：`CPP_MapManager.m_MapGimmickManager.m_MapGimmickList`（筛选 `interactType == eMapInteractType::Treasure (11)` 或 `IsA("CPP_MapGimmick_TreasureBox")`；位于 `*_GD` 子关卡）。
- **位置与开合状态（高置信度，成对实证）**：
  - 坐标：`Actor:K2_GetActorLocation()`；标识：`m_UniqueId`（同图机关唯一 ID，用于对应 `WBP_ObjectInteract_C`）与 `m_Id`（同图宝箱序号，对应数据表 `CDT_MapGimmickTreasureBox.ID`）。
  - **成对证据（`chest2-before` vs `chest2-after`，同坐标 `(5268.1, 1409.1, -888.4)`，`Dng_01Gra_40_PL`）**：
    - `BP_MapGimmick_TreasureBox_C_0.m_IsOpen`：**`false → true`**（C++ 亦有只读函数 `IsOpen()`、`IsUnlock()`；动画蓝图 `ABP_GimmickChest_C.m_IsOpen` 与 `m_IsFinishOpenAnim` 同步 `false → true`）。
    - 注：`chest-before` 与 `chest-after` 之间玩家在开箱后切换了地图（`Dng_01Gra_43_PL → Dng_01Gra_40_PL`），不能作同 Actor 对照；同图开关证据以 `chest2-before`/`chest2-after` 为准。
- **奖励数据表对齐与避坑（高置信度）**：
  - 宝箱内容由 **`m_TreasureBoxData`**（结构体 `{ID, mapId, ItemId, ItemNum, Money, IsTrap, battleGroupId}`）承载，通过复合键 `(mapId, m_Id)` 100% 对应 `CDT_MapGimmickTreasureBox.json`（238 行）。
  - **避坑**：`chest2` 的 `BP_MapGimmick_TreasureBox_C_0` 上旧字段 `m_ItemId = 12009`，而 `m_TreasureBoxData.ItemId = 4`（对应 `Dng_01Gra_40_1`）；实测 `chest2-after` 背包变化为 **`ItemId=4`（恢复药）`2 → 3`**、`12009` 未获得。因此奖励内容必须读 `m_TreasureBoxData` 或查 `CDT_MapGimmickTreasureBox`，切勿读 `m_ItemId`。
  - 跨图已开记录存于 `SaveData.Map.mapGimmick[mapId].treasureBox.treasureBoxList`（因快照未展开 TMap 结构体值，当前图直接读活体 Actor 的 `m_IsOpen`）。

#### (3) 存档点（紫色魔法阵，`BP_MapGimmick_SavePoint_C` / `CPP_MapGimmick_SavePoint`）

- **查找路径**：`CPP_MapManager.m_MapGimmickManager.m_MapGimmickList`（筛选 `interactType == eMapInteractType::SavePoint (33)` 或 `IsA("CPP_MapGimmick_SavePoint")`；位于 `*_GD` 子关卡）。
- **位置与关键属性（高置信度）**：
  - 实测两个实例：`Dng_01Gra_43_GD:BP_MapGimmick_SavePoint_C_1`（`m_UniqueId=25`, `m_Type=None (0)`）与 `Dng_01Gra_40_GD:BP_MapGimmick_SavePoint_C_0`（`m_UniqueId=16`, `m_Type=Boss (1)`）。
  - 关键属性：`m_UniqueId`、`m_Type`（`eMapGimmick_SavePointType`: `None`/`Boss`）、`m_IsTeleport: bool`、`m_TranditionMapId`、`m_TranditionPoint`、`m_ParentSavePointData: {mapId, UniqueID, savePoint}`、`m_EventList[0].EventId == "savePoint"`；只读函数 `CanTeleprot()`（注意源码拼写）、`IsInteracted()`。

#### (4) 帐篷能用的地点

- **规则来源（数据表 + 迷宫实机互证，高置信度）**：`DT_SavePointData.json` 按当前地图 `eMapType` 控制存档点菜单可用指令：
  - **`eMapType::Dungeon`（迷宫存档点）**：`SavePoint = true`、**`Tent = true`**、`Teleport = true`。实测 `savepoint`、`teleport-ui`、`tent-confirm-no/yes` 均显示“在帐篷中休息”（`ESavePointCommandType::Tent`，消耗 `ItemId = 500` 帐篷）。在 `tent-confirm-no/yes` 中弹出确认框 `WBP_DialogBox_C`（`DialogTag = "UI.Dialog.Box.Item.Use"`, `ChoiceTag = "UI.Dialog.Choice.YesNo"`），确认休息后的 `tent-after` 中背包 `ItemId = 500` 从 `3 → 2`，且 `m_InteractEventId.EventId` 重置为 `"None"`。
  - **`eMapType::Town`（城镇存档点）**：`SavePoint = true`、**`Tent = false`**、`Teleport = false`。
- **禁存档区**：`CPP_MapManager.m_MapCannotSaveAreaStorage`（`m_pCannotSaveAreaList` / `m_pOverlapCannotSaveAreaList`）管理禁存档区域；已测地图均为空（大地图 `Wld_PL` 野外能否用帐篷及禁存档区实机表现尚缺快照，置信度：**低**）。

---

## 4. 传送：紫色存档点往返传送 vs 大地图快速传送

真机快照证实本游戏存在**两套独立传送机制**，必须严格区分：

### 4.1 紫色存档点之间的迷宫传送（`teleport-ui-snapshot` 实测，高置信度）

- **机制纠正**：紫色存档点的“传送”**不是多目的地选单**，而是**同一迷宫“入口存档点 ↔ 深处（Boss 前）存档点”的成对往返**。`teleport-ui-snapshot` 中的教程文案明确写道：“*在特定的保存点之间可以使用‘传送’。使用传送后可在迷宫入口的保存点与最深处的保存点之间瞬间往返。*”
- **UI 与解锁变化（对比 `savepoint-snapshot` vs `teleport-ui-snapshot`）**：
  - 触发界面同为 `WBP_SavePointLayout_C`。在仅到过入口的 `savepoint-snapshot` 中只有 **2 个指令**（`保存`、`在帐篷中休息`）；抵达深处存档点后触发弹窗 `UI.Dialog.Box.SavePoint.Notice`（“现在可以在入口的保存点自由来回了”），`teleport-ui-snapshot` 中 `WBP_SavePointLayout_C` 出现第 **3 个指令**（`传送`，对应 `DT_SavePointCommand` 的 `ESavePointCommandType::Teleport` / `Teleport_Exit`，说明“回到迷宫入口”）。
- **数据配对与目标落点**：两端的目标地图与落点直接写在两个 `BP_MapGimmick_SavePoint_C` Actor 上，不依赖独立总表：
  - 入口点（`Dng_01Gra_43_PL`, `mapId=3001`, `m_UniqueId=25`, `m_Type=None`）：`m_IsTeleport=true`，`m_ParentSavePointData={mapId:3000, UniqueID:16}`，`m_TranditionMapId=3000`，`m_TranditionPoint=10000`。
  - 深处点（`Dng_01Gra_40_PL`, `mapId=3000`, `m_UniqueId=16`, `m_Type=Boss`）：`m_IsTeleport=true`，`m_ParentSavePointData={mapId:-1, UniqueID:-1}`，`m_TranditionMapId=3001`，`m_TranditionPoint=10000`。
  - 两端 `m_TranditionPoint = 10000` 均指向目标地图的 `BP_MapTransitionPoint_C`（`m_PointID = 10000`）。
- **运行时判断是否已解锁**：调用当前存档点 Actor 的只读 C++ 函数 **`CanTeleprot()`** 与 **`IsInteracted()`**（底层持久化在 `SaveData.Map.mapGimmick[mapId]`；尚缺执行传送瞬间的跨图快照）。

### 4.2 大地图地标快速传送（FastTravel，`world-map-m-snapshot` 实测）

- **已解锁传送地标列表（高置信度，与数据表 100% 互证）**：
  - `VM_WldMapLandmarkList_C.AllLandmarkIdList`：实测为 **`[3001, 2014]`**，元素即 `CDT_MapData_Prod.ID`（`3001` 土之神殿入口、`2014` 格兰谢尔特城）。
  - 同期 `CPP_UI_WldMapSubsystem.FldObjDataMap` 含地标对象映射（`213 → RelatedMapId: 3001`、`215 → RelatedMapId: 2014`），`WBP_WldMap_LandmarkLayout_C.EntryMap` 持有对应 `VM_WldMapEntry_C`。
- **数据表对应（`DA_MapLandmark.json`，共 61 条地标）**：
  - 每条含 `landmarkDataList[{Condition, mapId}]`、大地图世界坐标 `Location`、飞空艇着陆点 `airShipLandingPointId` / `airShipLandingLocation`。
  - 实测核对：`DA_MapLandmark` 第 40 项（`mapId=3001, Location=(17955, 20915, 413), airShipLandingPointId=1001`）与 `world-map-m-snapshot` 中 `VM_WldMap_C.FocusObjectData` 的字段数值**完全一致**。
- **解锁存储与可用性判定**：
  - 持久化解锁来源：`SaveData.Map.perMapSaveDataMap`（`TMap<int32, SaveData_PerMap>`，含 `bIsKnown`/`bIsVisited`/`bIsCleared`；置信度：**中**，因快照未展开 TMap 结构体）。
  - 当前能否快速传送：`VM_WldMap_C.FastTravelBehavior.{bCanTravel, DialogTag}`（在 `world-map-m-snapshot` 迷宫内为 `bCanTravel=false, DialogTag="UI.Dialog.Box.Map.FastTravel.CanNot.MapType"`，迷宫内禁止大地图快传；大地图/城镇内 `bCanTravel=true` 及实际传送过程尚缺快照）。

---

## 5. 大地图界面（M 键，`world-map-m-snapshot`）

> 注：本节主要基于单份快照 `20261124-170411-world-map-m-snapshot`（在迷宫内按 M 打开），无光标移动/切换岛屿对照，故光标互动类字段置信度标**中**，与数据表或其它快照互证的标**高**。

| 界面信息 | 类与属性（实例定位） | 实测示例值 | 置信度 |
| --- | --- | --- | --- |
| 是否处于大地图界面 | `CPP_VM_MainMenu.SceneDataStack` 栈顶 Tag（`MVVMViewModelCollectionObject`） | `["UI.MainMenu.Scene.Map"]`（且 `WBP_WldMapLayout_C` 存在） | 高 |
| 玩家当前位置与朝向 | `VM_WldMap_C`（`MVVMViewModelCollectionObject.VM_WldMap`）：`PlayerLocation`、`PlayerPosition`、`PlayerWldAlignment`、`CurrentIslandType`、`CurrentAreaId`、`PlayerDirection`、`PlayerDirectionDegree` | `PlayerLocation=(2215.7,3968.5,-1697.8)`（与 Pawn 相同）、`CurrentIslandType=Grandshelt`、`CurrentAreaId=1`、`PlayerDirection=East`、`PlayerDirectionDegree=90.0` | 高 |
| 光标与聚焦地标 | `VM_WldMap_C`：`CursorAlignment`、`CameraAlignment`、`FocusObjectId`、`FocusObjectData`、`FocusQuestId` | `FocusObjectId=213`、`FocusObjectData.{RelatedMapId=3001, Location=(17955,20915,413), LandingPointId=1001, RelatedQuestIdList=[1001]}`、`FocusQuestId=1001` | 中 |
| 地标/地点标记列表 | `VM_WldMapLandmarkList_C.AllLandmarkIdList`；`WBP_WldMap_LandmarkLayout_C.EntryMap` → `WBP_WldMapEntry_C` → `VM_WldMapEntry_C` | `[3001, 2014]`；条目含 `RelatedMapId`、`IconTag`（如 `UI.Map.Icon.Cave`、`UI.Map.Icon.Castle`）、`RelatedQuestIdList` | 高（列表）/ 中（单条图标） |
| 快传许可状态 | `VM_WldMap_C.FastTravelBehavior` | `{bCanTravel: false, DialogTag: "UI.Dialog.Box.Map.FastTravel.CanNot.MapType"}` | 中 |
| **常驻任务目标点（无需按 M 即可读）** | `CPP_ScenarioSubsystem`：`m_CurrentDestinationMapIdList`、`m_CurrentMainScenarioMapObjectList`<br>`CPP_QuestSubsystem`：`MainQuestId`、`PreferredQuestId`、`QuestDataMap`<br>`CPP_MiniMapSubsystem.EntryDataMap`（`MiniMapEntryData` 含 `bIsMarkedQuestTarget`、`BelongAreaId`） | `m_CurrentDestinationMapIdList=[3001]`、`MainQuestId=1001`、`PreferredQuestId=-1`（对应 `DT_MainScenario.DestinationList`） | 高 |

---

## 6. 连续坐标下的寻路与执行器设计

### 6.1 实机与反射中确认的导航与拓扑类

1. **实时 NavMesh 引擎层（实测存在 + 战斗 `.jmap` 确认完整只读 API，高置信度）**：
   - 场景实测实例：`/Script/NavigationSystem.RecastNavMesh`（`Dng_01Gra_40_PL` 存在 2 个活体实例）、`/Script/NavigationSystem.NavCollision`（195 个实例）。
   - `/Script/NavigationSystem.NavigationSystemV1` 暴露标准**只读寻路函数**：
     - `FindPathToLocationSynchronously(WorldContextObject, PathStart, PathEnd, PathfindingContext, FilterClass) -> NavigationPath*`
     - `FindPathToActorSynchronously(WorldContextObject, PathStart, GoalActor, ...)`
     - `K2_ProjectPointToNavigation(WorldContextObject, Point, ProjectedLocation, NavData, FilterClass, QueryExtent) -> bool`
     - `NavigationRaycast(WorldContextObject, RayStart, RayEnd, HitLocation, ...) -> bool`
     - `GetPathLength(...)` / `GetPathCost(...)`
   - `/Script/NavigationSystem.NavigationPath` 暴露 `PathPoints: TArray<FVector>`、`IsValid()`、`IsPartial()`、`GetPathLength()`。
2. **区域划分与跨区/跨图边（实测存在，高置信度）**：
   - `CPP_Map_NavMeshBoundsVolume`（位于 `*_NAV` 子关卡，如 `Dng_01Gra_40_NAV` 18–19 个、`Town_01Gra_22_NAV` 7 个）：属性 `m_AreaId`、`m_pBoxComponent`、`m_TransitionTriggerList`；只读函数 `GetAreaId()`、`GetConectionAreaIdList()`、`GetTransitionTriggerList()`。
   - `CPP_MapTransitionTrigger`（位于 `CPP_MapManager.m_pMapTransitionTriggerStorage.m_TransitionTriggerList`）：出口/换区触发盒，含 `m_UniqueId`、`m_MapId`（目标图 ID）、`m_PointID`（目标落点 ID）、`m_IsSameMapTrasition`、`m_IsAutoTransition`、`m_Condition`、`m_pRootBoxComponent`（坐标由 `K2_GetActorLocation()` 取得）。
   - `CPP_MapTransitionPoint`（位于 `CPP_MapManager.m_MapTransitionPointList`）：进图/传送落点，含 `m_PointID`、`m_Direction`、`m_PointName`、`m_pLadder`、`m_Elevator`、`m_AreaBoxList`。
3. **动态阻挡与特殊通行机关（`CPP_MapGimmickManager.m_MapGimmickList` 实测，中置信度）**：
   - `CPP_MapGimmick_BlockingBase`（剧情进度控制屏障，带 `VisibleCondition`，如 `BP_MapGimmick_BlockingBase_C`）、`CPP_MapGimmick_NoEntry`（禁入区，带 `m_ReturnDistance`）、`CPP_MapGimmickLadder`（梯子，带 `m_pTopBoxComponent`、`m_pBottomBoxComponent`、`m_pSplineComponent`）。
4. **严禁触碰的写接口（红线）**：`CPP_MapUnit::AIMoveToLocation`、`CPP_MapDebug::Execute_AutoMove` / `Execute_GoToTransitionMap` 等属于直接驱动角色或修改游戏状态，违反“游戏内只读 + 模拟按键”原则，**禁止调用**。

### 6.2 执行器“从 A 走到 B”可行方案

- **推荐主方案：只读 NavMesh 航点规划 + 按键闭环跟踪**
  1. **读取静态/换图拓扑**：进图时（或换图后一次）由桥采集当前图的 `TransitionTrigger`（坐标、目标 `mapId`/`pointId`、`isSameMap`）、`TransitionPoint`、`TreasureBox`、`SavePoint`、`NPC` 与 `NavMeshBoundsVolume` 的 `AreaId` 邻接关系。
  2. **同图/同区折线求解**：执行器需要从 A 走到 B 时，向桥发起一次**只读路径查询**（调用 `NavigationSystemV1:FindPathToLocationSynchronously(World, A, B)`；若终点落在宝箱/NPC 碰撞体内，先用 `K2_ProjectPointToNavigation` 或按交互半径向玩家侧偏移 `80–120 u`），返回 `PathPoints` 3D 航点序列及 `is_partial`。
  3. **按键闭环跟踪**：
     - 根据当前 `pawn_location` 到下一个航点的平面向量 `(dx, dy)` 及相机水平朝向（`VM_MiniMap_C.CameraDirectionDeg`，注意 `BP_MapUnit_Player_C.m_MapCameraInternalRotation == true` 的旋转镜头区需动态校正屏幕-世界坐标系旋转角），映射为 8 方向移动键（WASD 组合）持续按压。
     - **过冲与采样节拍匹配**：常态速度约 `750 u/s`（冲刺 `~1040 u/s`），在 `0.5s` 状态采样下每拍位移 `375–520 u`。建议执行器采用“距离 `> 350 u` 持续按住 + 临近目标 `< 300 u` 按 `距离 / 750` 做短脉冲点按（或按住慢走键）”的策略，航点切换容差 `80–120 u`，交互目标到达以 `interactables` 出现目标为准。
     - **卡墙与遇敌恢复**：若连续 2–3 拍（`1.0–1.5s`）有效位移 `< 25 u` 且 `can_control == true`，立即松键、重调一次 `FindPathToLocationSynchronously`（避开动态 `BlockingBase`/NPC 碰撞）或做 0.3s 侧向微调；遇敌/剧情打断（`can_control` 变 `false`）则立即松开所有方向键，待恢复 `can_control == true` 后从新坐标续算。
  4. **跨区 / 跨图长程导航**：在“地图-区域-TransitionTrigger”图上做最短路，将长程任务拆成若干“走到触发器 → 等待 `loading == false && can_control` → 下一段”；迷宫回城可优先利用已解锁的存档点传送或大地图快传。
- **兜底方案（无 NavMesh 区域，如大地图 `Wld_PL` 待验证）**：若 `FindPathToLocationSynchronously` 返回无效或 `RecastNavMesh` 缺失，则退化为基于离线地标坐标（`DA_MapLandmark.json`）+ 直线趋近 + 卡墙绕障的轻量跟踪。

---

## 7. 与草稿 `docs/state-map-draft.md` 的核对汇总

| 草稿章节与候选 | 核对结论 | 实机证据说明 |
| --- | --- | --- |
| §3 `CPP_MapUnit_Player` 位置（`K2_GetActorLocation`） | **已证实** | 95/95 有效；但须排除过场藏身坐标 `(25408, 384, -500)` 与 `m_IsVisible == false` |
| §3 `BP_MapUnit_Player_C.m_Direction`（玩家朝向） | **已推翻** | 全 95 份快照恒为 `North (1)`；实时朝向改为读 `VM_MiniMap_C.PlayerDirection` / `PlayerDirectionDegree`（仅 NPC 的 `m_Direction` 有效） |
| §3 `m_IsDash`、`m_IsEnableMovement`、`m_IsEnableUnitInput` | **部分推翻/修正** | `m_IsDash`、`m_IsEnableMovement` 95/95 恒 `true`；`m_IsEnableUnitInput` 仅战斗为 `false`，不能单独判别能否操作，须改用 §1.3 组合条件 |
| §3 `m_InteractMapUnitList` / `m_pInteractMapGimmickList` / `m_InteractEventId` | **已证实** | 靠近 NPC/宝箱/存档点时立即更新，成对快照 `chest2-before/after` 证实；**修正**：`m_pInteractMapUnit` 恒为 `null`，只用数组 `m_InteractMapUnitList` |
| §3 `CPP_MapUnitStorage.m_MapUnits` | **已证实** | 持有当前地图 Player 与全部 NPC 实例 |
| §3 `CPP_FieldObjectSubsystem` / `CPP_VM_FieldObject` | **已证实** | `FieldObjectVMManager.VMMap` 中 `bInteractable` 与 `InteractTag` 在 `chest2-before/after` 准确翻转 |
| §3 `SaveData.Map.unitLocation` / `unitDirection` / `mapId` | **已修正（非实时）** | 仅在自动存档/手动存盘刷新，换图与自由行走期间滞后；**不可作实时状态源** |
| §3 `CPP_MuchaPlayerController.Pawn` 载具切换（陆行鸟/飞空艇/船） | **尚缺快照** | 现有快照全部为步行（`Walking`），待进入 `Wld_PL` 与载具后核对 |
| §4 `CPP_MapManager`（触发器/落点/机关/子关卡） | **已证实** | `m_MapGimmickManager`、`m_pMapTransitionTriggerStorage`、`m_MapTransitionPointList`、`m_pSublevelLoadManager` 全部在野外快照实存；`GetMapId()` 待 v1 实调核对，同期已证实 `CPP_MiniMapSubsystem.PropertyKey.mapId` 98/98 准确 |
| §4 `CPP_LoadingScreenManager.IsShowLoadingScreen` | **已证实** | `auto-23`、`auto-42` 换图加载中准确为 `true`，其余均为 `false` |
| §4 `CPP_FadeManager` | **证据不足** | `m_FadeScreens` 始终有 2 个对象，未见淡入淡出过程中的快照；v1 可调用只读函数 `IsFadeFinish()` 核对 |
| §4 `CPP_MuchaWorldSetting` | **已修正** | 子关卡各有一份（开关全 `false`），须仅读 `*_PL` 持久关卡那份 |
| §4 `SaveData.Map.mapGimmick` / `perMapSaveDataMap` | **受限于快照格式** | 键存在（`mapId`），但 FfrDump 未展开 TMap 结构体值；当前图改读活体 Actor，已解锁地标改读 `VM_WldMapLandmarkList_C.AllLandmarkIdList` |
| §9 存档点菜单与禁存档区（`ESavePointCommandType` / `m_MapCannotSaveAreaStorage`） | **部分证实** | `WBP_SavePointLayout_C`、迷宫存档点帐篷与成对传送已证实；禁存档区在现有地图为空，尚缺实测 |

---

## 8. FfrBridge v1 建议字段

为兼顾实时操控精度与 UE4SS 采样开销，建议 FfrBridge v1 在保留 v0 顶层 `clock_ms`、`world`、`map`、`errors` 的基础上，将野外/地图状态分为 **0.5s 高频核心字段**、**低频/按变化刷新拓扑字段** 与 **按需只读 RPC（寻路查询）** 三层。所有对象读取均须沿用 v0 的 `IsValid()` + `pcall` 防护，绝不跨图缓存 `UObject` 指针。

### 8.1 每 0.5 秒输出的高频核心字段（共 16 个字段）

| # | 建议字段路径 | 类型 | 来源对象与属性 / Getter | Lua 查找与读取方式 | 备注 |
| --- | --- | --- | --- | --- | --- |
| 1 | `field.map_id` | `number \| null` | `CPP_MiniMapSubsystem.PropertyKey.mapId`（兜底 `CPP_MapManager:GetMapId()`） | `FindFirstOf("CPP_MiniMapSubsystem")` → `.PropertyKey.mapId` | 98/98 实时准确；标题图为 `-1` |
| 2 | `field.area_id` | `number \| null` | `CPP_MiniMapSubsystem.AreaManager.CurrentAreaId` | 沿上一步 `AreaManager.CurrentAreaId` | 当前子区域/楼层 ID |
| 3 | `field.is_loading` | `boolean` | `CPP_LoadingScreenManager.IsShowLoadingScreen` | `FindFirstOf("CPP_GameWidgetManagerSubsystem").m_LoadScrManager.IsShowLoadingScreen` | 换图读条期为 `true` |
| 4 | `field.in_battle` | `boolean` | `CPP_UI_BattleSubsystem.bDuringBattle` | `FindFirstOf("CPP_UI_BattleSubsystem").bDuringBattle` | 战斗期为 `true` |
| 5 | `field.in_event` | `boolean` | `CPP_TalkEventSubsystem.m_pEventManager ~= nil` | `FindFirstOf("CPP_TalkEventSubsystem").m_pEventManager` 判 `IsValid()` | 覆盖过场、对话、商店、存档点、开箱提示 |
| 6 | `field.in_dialog` | `boolean` | `CPP_DialogManager.DialogWidget ~= nil` | `GameWidgetMgr.m_DialogManager.DialogWidget` 判 `IsValid()` | 覆盖确认弹窗（帐篷/使用道具等） |
| 7 | `field.in_menu` | `boolean` | `CPP_MainMenuSubsystem:IsOpenMenu()` + `CPP_VM_MainMenu.SceneDataStack` 长度 | `FindFirstOf("CPP_MainMenuSubsystem"):IsOpenMenu()`（兜底 `CPP_VM_MainMenu`） | 覆盖主菜单、大地图、恢复、任务、道具页 |
| 8 | `field.can_control` | `boolean` | 派生汇总字段（见 §1.3） | `pawn有效 && visible && enInput && enMove && not is_loading && not in_battle && not in_event && not in_dialog && not in_menu && 非候场坐标`（并在真机核对顶层 `GetInputTargetLayer()` 补齐侧边菜单） | 执行器按键门控主标志 |
| 9 | `field.player.location` | `{x,y,z} \| null` | `PlayerController.Pawn:K2_GetActorLocation()` | 复用 v0 `pc.Pawn`（保留 `round1`） | 过场藏身时为 `(25408,384,-500)` |
| 10 | `field.player.visible` | `boolean \| null` | `BP_MapUnit_Player_C.m_IsVisible` | `pawn.m_IsVisible` | 过场候场与战斗时为 `false` |
| 11 | `field.player.unit_id` | `number \| null` | `BP_MapUnit_Player_C.m_MapUnitId` | `pawn.m_MapUnitId` | 序章 `60`，第 1 章起 `10` |
| 12 | `field.player.direction` | `string \| null` | `VM_MiniMap_C.PlayerDirection` | `FindFirstOf("VM_MiniMap_C").PlayerDirection`（枚举名） | 实时 8 方向；**勿读 `pawn.m_Direction`** |
| 13 | `field.player.direction_deg` | `number \| null` | `VM_MiniMap_C.PlayerDirectionDegree` | 同上 `.PlayerDirectionDegree`（并可附读 `CameraDirectionDeg` 供寻路映射） | `0=北, 90=东, 180=南, -90=西` |
| 14 | `field.player.movement_method` | `string \| null` | `BP_MapUnit_Player_C.m_MapMovementMethod` | `pawn.m_MapMovementMethod`（枚举名） | `Walking`/`Chocobo`/`AirShip`/`Ship` |
| 15 | `field.player.speed_2d` | `number \| null` | `CharacterMovementComponent.Velocity` 平面模长（`hypot(X,Y)`） | `pawn.m_pPawnMovementComponent.Velocity`（兜底由相邻两拍位置与 `Δclock_ms` 算） | 区分静止 / 慢走 / 750 跑 / 1040 冲刺 |
| 16 | `field.interact_target` | `{kind, unique_id, event_id, interact_type, location, shop_id?, is_open?, can_teleport?} \| null` | `pawn.m_InteractMapUnitList`、`pawn.m_pInteractMapGimmickList`、`pawn.m_InteractEventId.EventId` | 优先取 `m_pInteractMapGimmickList[1]` 或 `m_InteractMapUnitList[1]`，按类读 `m_UniqueId`、`m_MapInteractType`、`K2_GetActorLocation()` 及宝箱 `m_IsOpen` / 商店 `ShopEntry.ShopID` / 存档点 `CanTeleprot()` | 玩家面前当前按确认即可触发的目标 |

> 注：若需要把旋转镜头区的坐标映射到屏幕方向键，可将 `VM_MiniMap_C.CameraDirectionDeg` 与 `pawn.m_MapCameraInternalRotation` 作为 `field.camera_deg` 并入或随 `direction_deg` 输出；若后续 v1 保留 `pawn_rotation`，务必读取 `FRotator` 的 `.Pitch / .Yaw / .Roll` 而非 `.X / .Y / .Z`。

### 8.2 低频 / 按变化刷新的地图上下文与交互物清单（共 5 个字段）

以下字段在单张地图内基本不变或仅在交互/推剧情时变更，**不要每 0.5s 全量遍历 Actor 数组**；建议在 **换图完成（`map_id` 变化且 `is_loading == false`）时全量采集一次**，之后每 **2–5 秒**（或仅在 `in_event` 从 `true → false` / 地图内对象变更时）增量刷新，总耗时控制在毫秒级：

| # | 建议字段路径 | 类型 | 来源对象与属性 / Getter | 刷新策略与说明 |
| --- | --- | --- | --- | --- |
| 17 | `field.quest_targets` | `{main_quest_id, preferred_quest_id, destination_map_ids[]}` | `CPP_QuestSubsystem.{MainQuestId, PreferredQuestId}` + `CPP_ScenarioSubsystem.m_CurrentDestinationMapIdList` | 仅在 `map_id` 变化或每 5s 读一次（均为轻量 `GameInstanceSubsystem` 标量/短数组） |
| 18 | `field.unlocked_landmarks` | `number[]` | `VM_WldMapLandmarkList_C.AllLandmarkIdList`（打开过大地图后常驻；未打开时可留空或按需读 `SaveData.Map.perMapSaveDataMap`） | 每 5s 或换图后读一次，直接对齐 `CDT_MapData_Prod.ID` 与 `DA_MapLandmark.json` |
| 19 | `field.chests` | `Array<{unique_id, chest_id, is_open, item_id, item_num, money, location}>` | `CPP_MapManager.m_MapGimmickManager.m_MapGimmickList` 筛 `CPP_MapGimmick_TreasureBox`：读 `m_UniqueId`、`m_Id`、`m_IsOpen`、`m_TreasureBoxData`、`K2_GetActorLocation()` | 换图后扫一次 + 事件结束（`in_event: true→false`）复扫，单图通常 `< 10` 个 |
| 20 | `field.save_points` | `Array<{unique_id, type, is_teleport, can_teleport, target_map_id, target_point_id, location}>` | `m_MapGimmickList` 筛 `CPP_MapGimmick_SavePoint`：读 `m_UniqueId`、`m_Type`、`m_IsTeleport`、`CanTeleprot()`、`m_TranditionMapId`、`m_TranditionPoint`、`K2_GetActorLocation()` | 换图后扫一次 + 存档点交互结束复扫，单图通常 `0–2` 个 |
| 21 | `field.transitions_and_npcs` | `{triggers: [...], npcs: [...]}` | `CPP_MapManager.m_pMapTransitionTriggerStorage.m_TransitionTriggerList`（`m_UniqueId`/`m_MapId`/`m_PointID`/`m_IsSameMapTrasition`/`m_IsAutoTransition`/位置）+ `m_MapUnitStorage.m_MapUnits` 筛可见 NPC（`m_UniqueId`/`m_MapUnitId`/`m_MapInteractType`/`ShopEntry.ShopID`/`InnEntry.InnID`/位置） | 换图后扫一次 + 事件结束复扫；供规划层与寻路执行器选取目标坐标 |

### 8.3 按需只读查询接口（供 R5 寻路执行器用，不进 0.5s 轮询）

- **`query_nav_path(start, goal)`**：调用 `NavigationSystemV1:FindPathToLocationSynchronously(World, start, goal)`（纯只读查询，不修改角色状态），返回 `{valid, partial, length, points: [{x,y,z}, ...]}`。
- **性能与安全要求**：
  - 每次 0.5s 采样只做 `O(1)` 的 Subsystem / Pawn / ViewModel 标量读取，不调用 `FindAllOf` 扫描全 World；
  - 换图期间检测到 `is_loading == true` 或 `World` 完整名变更时，立即丢弃上一张图的全部局部对象引用并清空低频缓存，避免访问已卸载子关卡的悬空 `UObject`。
