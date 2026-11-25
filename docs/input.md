# R4 模拟输入调研与设计（进程内注入）

本文依据 R1 真机快照（`title`、`field-idle`、`cmd-cursor-1`、`world-map-m` 等）、战斗期反射数据（`battle-jmap`）与 R2 导出输入表（`CDT_TagKeyGuide.json`、`DT_OP_KeyboardMouseButtonList.json`），比较 UE4SS Lua 下三种进程内输入注入路线，并给出《FINAL FANTASY RESONANCE》R4 的首选方案、备选方案、动作映射依据及待真机验证项。

> [!IMPORTANT]
> 本方案严格遵循 `docs/plan.md` 原则 4（**只读加按键**）：不安装虚拟手柄驱动、不修改角色坐标/菜单光标/存档内存数值、**不调用** `CPP_BFL_UI_Input:PopBlock` / `ResetBlock` 等修改游戏内部状态的函数；唯一允许的写操作是通过引擎标准输入入口注入模拟输入。输入命令在模组和 `ffr-module` 两侧**默认关闭**。

---

## 1. 已知运行时输入管线与关键制约

结合 `docs/state-map/ui.md` §6 与反射/快照交叉核对，游戏输入链路如下：

```text
物理键鼠/手柄
  → APlayerController::InputKey (C++ 虚函数，非 UFUNCTION)
  → UEnhancedPlayerInput (PlayerController.PlayerInput)
  → UEnhancedInputLocalPlayerSubsystem (实现 IEnhancedInputSubsystemInterface)
      ├─ [动作层注入点] InjectInputForAction / StartContinuousInputInjectionForAction
      └─ 触发已绑定的 EnhancedInputComponent 回调（经 CPP_InputRegistrant.m_BindHandleList 注册）
          ├─ 野外角色移动/交互 → BP_MapUnit_Player_C:InputMove / InputInteract
          ├─ 主菜单快捷键      → CPP_MainMenuSubsystem:HandleInputScene
          ├─ 战斗快捷动作      → CPP_BtlInput:On*
          └─ 全部通用 UI 输入  → CPP_GameWidgetInputManager:HandleInput*
                                   ├─ [门控] 检查 InputBlockSet（失焦含 "WindowDeactivate"）
                                   └─ 转发至 WBP_OverAllLayout_C 当前输入层的焦点控件
                                        └─ [界面层接口] CPP_BI_WidgetBasicInput:Input*
```

关键实证事实：

1. **游戏原生已使用 `EnhancedInputLocalPlayerSubsystem:InjectInputForAction`**：在战斗期反射数据中，游戏自带的两个 UI 蓝图函数 `/Game/UI/MainMenu/Common/Key/KeyGuide/WBP_TagKeyGuide.WBP_TagKeyGuide_C:InjectInputAction` 与 `/Game/UI/Common/Key/Mapping/Widget/WBP_KeyMappingButton.WBP_KeyMappingButton_C:BP_OnClicked`（玩家用鼠标点击底部按键提示时）正是调用 `EnhancedInputLocalPlayerSubsystem:InjectInputForAction`，并用 `EnhancedInputLibrary:MakeInputActionValueOfType`（或 `CPP_EU_BFL_InputActionValue:MakeFromEditable`）构造 `FInputActionValue`，搭配 `InputTriggerPressed` 触发一次点击。
2. **游戏自己的 `IA_*` 资产注册表**：`field-idle` 快照完整包含全部 `CPP_DA_MUCHA_INPUT` 数据资产（如 `DA_Input_UI_Common`、`DA_Input_MapUnit`、`DA_Input_UI_MainMenu`、`DA_Input_UI_SystemMenu`），每个资产的 `InputActionList`（`TMap<FString, UInputAction*>`）直接对应 `CPP_InputManager:GetInputAction(MappingType, ActionName)` 的参数与 `UInputAction` 对象路径。
3. **窗口失焦屏蔽门控（10c 重要发现）**：窗口失去焦点时，`CPP_GameWidgetInputManager.InputBlockSet.TokenSet` 被写入 `"WindowDeactivate"`（可经 `CPP_BFL_UI_Input:IsBlocked()` 查询），屏蔽界面输入；窗口回到前台后该令牌自动清空。

---

## 2. 三种进程内注入方案对比

### 2.1 方案一：动作层 — `EnhancedInputLocalPlayerSubsystem`（首选）

- **核心反射签名**（声明于 `/Script/EnhancedInput.EnhancedInputSubsystemInterface`，全部为 `FUNC_BlueprintCallable | FUNC_Native | FUNC_Public`，由 `/Script/EnhancedInput.EnhancedInputLocalPlayerSubsystem` 实现）：
  - `InjectInputForAction(Action: UInputAction*, RawValue: FInputActionValue, Modifiers: TArray<UInputModifier*>, Triggers: TArray<UInputTrigger*>)`
  - `InjectInputVectorForAction(Action: UInputAction*, Value: FVector, Modifiers: TArray<UInputModifier*>, Triggers: TArray<UInputTrigger*>)`
  - `InjectInputForPlayerMapping(MappingName: FName, RawValue: FInputActionValue, Modifiers: TArray<UInputModifier*>, Triggers: TArray<UInputTrigger*>)`
  - `InjectInputVectorForPlayerMapping(MappingName: FName, Value: FVector, Modifiers: TArray<UInputModifier*>, Triggers: TArray<UInputTrigger*>)`
  - **连续输入三件套**（UE 5.6 原生，维护于 `EnhancedInputLocalPlayerSubsystem.ContinuouslyInjectedInputs`）：
    - `StartContinuousInputInjectionForAction(Action: UInputAction*, RawValue: FInputActionValue, Modifiers: TArray<UInputModifier*>, Triggers: TArray<UInputTrigger*>)`
    - `UpdateValueOfContinuousInputInjectionForAction(Action: UInputAction*, RawValue: FInputActionValue)`
    - `StopContinuousInputInjectionForAction(Action: UInputAction*)`
    - 以及对应的 `*ForPlayerMapping(MappingName, ...)` 版本。
- **Lua 可调性与参数结构体构造**：
  - **对象获取**：`FindFirstOf("EnhancedInputLocalPlayerSubsystem")` 取得子系统单例；`UInputAction*` 优先调用 `CPP_InputManager:GetInputAction(MappingType, ActionName)`（`BP=True`，游戏自身 `WBP_TalkEventLayout_C:Handle_InputSkip_Ongoing` 即如此调用），失败时用 `StaticFindObject("/Game/Input/.../IA_*.IA_*")` 兜底。
  - **`RawValue`（`/Script/EnhancedInput.InputActionValue`）构造**：反射证实 `FInputActionValue` **没有反射 UProperty 字段**（32 字节不透明结构体），Lua 无法用表字面量直接拼装；但 `/Script/EnhancedInput.EnhancedInputLibrary` 暴露了静态蓝图函数 **`MakeInputActionValueOfType(X: double, Y: double, Z: double, ValueType: EInputActionValueType) -> FInputActionValue`**（`0=Boolean, 1=Axis1D, 2=Axis2D, 3=Axis3D`），可在 Lua 中通过 `StaticFindObject("/Script/EnhancedInput.Default__EnhancedInputLibrary")` 调用返回原生结构体。作为零结构体构造兜底，`InjectInputVectorForAction` 直接接受标准 `FVector`（Lua 表 `{ X = x, Y = y, Z = z }`）。
  - **`Modifiers` / `Triggers` 构造**：空表 `{}` 即可传入 `TArray<UInputModifier*>` / `TArray<UInputTrigger*>`；离散单次点按可参考 `WBP_TagKeyGuide_C:InjectInputAction` 经 `GameplayStatics:SpawnObject(InputTriggerPressed, subsys)` 传入单元素 `{ triggerPressed }`，连续移动/长按传 `{}`。
  - **UE4SS 接口分发注意点（要真机验证）**：上述 `InjectInput*` 声明在 `UInterface`（`EnhancedInputSubsystemInterface`）而非直接声明在 `EnhancedInputLocalPlayerSubsystem` 子类成员表上；需在真机确认 UE4SS v4.0.0 Lua 是否能直接 `subsys:InjectInputForAction(...)` 解析到接口 `UFunction`，以及 `Triggers = { InputTriggerPressed }` 与 `Triggers = {}` 在不同 `IA_*`（尤其是带 `CPP_EIT_AcceleHold` 触发器的 `IA_UI_Common_TabSwitch`）上的实际效果。
- **窗口失焦时是否会被 `InputBlockSet` 拦下**：
  - **界面动作大概率会被拦下（要真机验证）**：因为 `IA_UI_*` 回调全部落到 `CPP_GameWidgetInputManager:HandleInput*`，该管理器在转发给控件前检查 `InputBlockSet` 中的 `"WindowDeactivate"`。
  - **野外移动与主菜单快捷键（要真机验证）**：`IA_Map_UnitMove` / `IA_Map_UnitInteract` 绑定在 `BP_MapUnit_Player_C:InputMove` / `InputInteract`，`IA_Map_OpenMainMenu_*` 绑定在 `CPP_MainMenuSubsystem:HandleInputScene`，未必经过 `CPP_GameWidgetInputManager.InputBlockSet`，但仍可能受 `CPP_InputManager:OnApplicationFocusChanged(false)` 或引擎无焦点降频影响，需真机实测。
- **野外移动与界面导航适用性**：
  - **两者均适用**。离散界面操作（确认、取消、方向、翻页、菜单、地图、快速回复）走 `InjectInputForAction`；野外连续轴移动（`IA_Map_UnitMove`，`Axis2D`）走 `StartContinuousInputInjectionForAction` + 定时 `StopContinuousInputInjectionForAction`。
  - 自动免疫用户自定义改键和手柄确认/取消互换（`bSwapDecideButton`）。
- **风险**：
  - `InjectInputForAction` 仅将输入写入 `EnhancedPlayerInput.InputsInjectedThisTick`，在下一帧 `ProcessPlayerInput` 生效，不会在调用当帧同步改变 UI 状态（需等下一帧回读状态确认）；
  - 若 `StartContinuousInputInjectionForAction` 后未正确调用 `StopContinuousInputInjectionForAction`（如异常或换图），会导致角色持续奔跑，因此必须限制最大时长并在 `pcall` + token 保护下必停；
  - 只有在对应 IMC 已激活且动作已绑定时才会生效（跨场景误发某 `IA_*` 会静默无效果，需按场面区分如野外 `IA_Map_UnitInteract` vs 界面 `IA_UI_Common_Decide`）；
  - `IA_UI_Common_Direction` 的 Y 轴正负号与 `IA_UI_Common_TabSwitch` 的 ±1 方向**要真机验证**。

### 2.2 方案二：按键层 — `PlayerController:InputKey`（按 `FKey` 注入）

- **核心反射核查结果**：
  - 在 `battle-jmap` 全部 46,171 个反射对象中，**`APlayerController::InputKey(const FInputKeyParams&)` 不是 `UFUNCTION`**；`/Script/Engine.PlayerController` 仅暴露只读查询函数 `GetInputKeyTimeDown`、`IsInputKeyDown`、`WasInputKeyJustPressed`、`WasInputKeyJustReleased`；`/Script/Engine.InputKeyParams` 在反射表中完全缺失；`/Script/EnhancedInput.EnhancedPlayerInput` 未暴露任何 `UFUNCTION`。
  - 唯一暴露 `PressKey(FKey, bool)` 的反射函数是 `/Script/UMG.WidgetInteractionComponent:PressKey`，但其设计用途是 3D 空间射线虚拟光标交互，不经过 `PlayerInput` / `EnhancedInput`，无法驱动全屏 CommonUI 与野外移动。
  - `/Script/InputCore.Key` 结构本身很简单（仅含 `KeyName: FName`），且物理键名可通过 `CPP_InputManager:FindCurrentMappingForSlot(MappingName, 0)` 或 `GetKeyListToInputAction(IA)` 动态反查，但缺少可调用的注入 `UFUNCTION`。
- **Lua 可调性**：
  - **纯 UE4SS Lua 不可直接调用**。若强行在进程内走 `PlayerController::InputKey`，只能编写 C++ UE4SS 模组按虚表/特征码调用原生 C++ 函数，或退回到宿主机 Win32 按键消息。
- **窗口失焦时是否会被 `InputBlockSet` 拦下**：
  - **会被拦下**。即使在 C++ 层调用 `PlayerController::InputKey`，事件经 `EnhancedPlayerInput` 仍流向 `CPP_GameWidgetInputManager`，同样受 `"WindowDeactivate"` 门控；若改走 Win32 `SendInput`，则必须抢占前景窗口并干扰用户键鼠。
- **野外移动与界面导航适用性**：
  - 理论上（仅 C++ 原生封装前提下）发送 `IE_Pressed` / `IE_Released` 可覆盖离散键与按住 WASD 移动，但纯 Lua 下不可用。
- **风险**：
  - 脱离 Lua 模组体系，引入 MSVC 交叉编译与版本偏移脆弱性，且在 Denuvo 保护下做二进制签名扫描/钩子风险显著高于调用反射 `UFunction`。

### 2.3 方案三：界面层 — CommonUI / `EnhancedUI` / `CPP_GameWidgetInputManager`

- **核心反射核查结果**：
  - **CommonUI 原生**：`/Script/CommonUI.CommonUIActionRouterBase` 未暴露任何 `UFUNCTION`；`CommonActivatableWidget` 仅有 `BP_OnHandleBackAction() -> bool`；`CommonButtonBase` 仅有单按钮级 `HandleButtonClicked()` / `HandleButtonPressed()`。
  - **EnhancedUI 控件接口（游戏实际 UI 框架）**：`/Script/EnhancedUI.CPP_BI_WidgetBasicInput` 暴露了一套 `BlueprintCallable` 语义函数：
    - `InputDecide(InEventType: EWidgetInputEventType)`（`0=Triggered, 1=Pressed, 2=Released`）
    - `InputCancel(InEventType: EWidgetInputEventType)`
    - `InputDirection(InNavigation: EUINavigation, InEventType: EWidgetInputEventType)`（`0=Left, 1=Right, 2=Up, 3=Down`）
    - `InputTabSwitch(InDirection: EEU_TabSwitchDirection, InEventType: EWidgetInputEventType)`（`0=Prev, 1=Next`）
    - 反射证实约 30+ 蓝图控件实现上述接口（如 `WBP_DialogBox_C`、`WBP_TutorialLayout_C`、`WBP_OptionList_C`、`WBP_BtlCmdLayout_C`、`WBP_BtlCmd_UnitSelect_C`、`WBP_Btl_ResultLayout_C`、`WBP_DirectionLayoutMenu_C`、`WBP_MenuCategoryTab_C`）。
  - **游戏界面输入管理器**：`/Script/mucha.CPP_GameWidgetInputManager` 暴露 40 个 `HandleInput*` 函数（如 `HandleInputDecide`、`HandleInputCancel`、`HandleInputDirection`、`HandleInputTabSwitch`），但全部标记为 `FUNC_Final | FUNC_Native | FUNC_Private` 且接收 `FInputActionValue`。
- **Lua 可调性与参数构造**：
  - 沿 `CPP_GameWidgetManagerSubsystem.m_GameLayout:GetInputTargetLayer():GetActiveWidget()` 下钻 `InputTargetWidget` / `PathMenu` 后，Lua 可以直接传整数枚举调用 `CPP_BI_WidgetBasicInput:Input*`，参数最简单（无需结构体）。
  - `CPP_GameWidgetInputManager:HandleInput*` 为私有非 BP 函数，UE4SS Lua 能否直接调用及是否绕过绑定状态**要真机验证**。
- **窗口失焦时是否会被 `InputBlockSet` 拦下**：
  - 调 `CPP_GameWidgetInputManager:HandleInput*`：大概率仍被 `InputBlockSet` 拦下（**要真机验证**）。
  - 直接调焦点控件的 `CPP_BI_WidgetBasicInput:Input*`：**有可能绕过 `CPP_GameWidgetInputManager.InputBlockSet` 的前置检查**（**要真机验证**），但若控件内部额外检查 `CPP_BFL_UI_Input:IsBlocked()` 仍会失效。
- **野外移动与界面导航适用性**：
  - **仅覆盖部分离散 UI 导航**，**完全不支持**野外移动（`IA_Map_UnitMove`）、野外交互（`IA_Map_UnitInteract`）、以及走 `CPP_MainMenuSubsystem` / `WBP_TalkWindow_C` / `CPP_BtlInput` 的快捷键与对话推进。
- **风险**：
  - 不同界面控件树结构各异（`PathMenu`、`InputTargetWidget`、`WBP_OptionList_C` 等），找错接收控件极易静默失败或绕过 `IsOpeningProcess`/`CanInput()` 保护导致 UI 状态错乱；
  - 直接调用界面内部回调比标准输入注入更贴近“调内部业务函数”，只适合作为特定 UI 故障时的降级研究方向，不宜做主路径。

### 2.4 综合对比表

| 维度 | 动作层（`EnhancedInputLocalPlayerSubsystem`） | 按键层（`PlayerController:InputKey`） | 界面层（`CPP_BI_WidgetBasicInput` / `CPP_GameWidgetInputManager`） |
| --- | --- | --- | --- |
| 纯 UE4SS Lua 可调用 | **可**（全部 `BlueprintCallable`，且游戏自身在用；接口分发细节待真机确认） | **否**（`InputKey` 非 `UFUNCTION`，`FInputKeyParams` 无反射） | **部分可**（`CPP_BI_WidgetBasicInput` 可调；`HandleInput*` 为 Private 待验） |
| 参数构造难度 | 中（`EnhancedInputLibrary:MakeInputActionValueOfType`，兜底 `InjectInputVectorForAction` 传 `{X,Y,Z}`） | 不适用（Lua 无法直调） | 低（整数枚举 `EUINavigation` / `EWidgetInputEventType`） |
| 窗口失焦被 `InputBlockSet` 拦截 | UI 动作大概率被拦；野外移动/热键**要真机验证** | 是（仍经 `CPP_GameWidgetInputManager`） | `HandleInput*` 大概率被拦；控件 `Input*` 可能绕过（**要真机验证**） |
| 界面导航（离散按键） | **适用**（覆盖全部 UI、战斗、对话、热键） | 纯 Lua 不可用 | 仅覆盖部分 EnhancedUI 控件 |
| 野外移动（连续轴输入） | **适用**（`Start/StopContinuousInputInjectionForAction`） | 纯 Lua 不可用 | **不适用** |
| 合规性与维护风险 | 低（走游戏标准输入管线，不改数据、免疫改键） | 高（需 C++ 模组偏移或干扰用户键鼠） | 中高（控件树碎片化，易绕过前置状态门控） |

---

## 3. 结论：首选方案与备选方案

1. **首选方案（本次实现）**：**动作层 `EnhancedInputLocalPlayerSubsystem` 注入**。
   - **依据**：游戏原生 `WBP_TagKeyGuide_C:InjectInputAction` 与 `WBP_KeyMappingButton_C:BP_OnClicked` 实证采用该路径；`EnhancedInputSubsystemInterface` 完整暴露离散（`InjectInputForAction` / `InjectInputVectorForAction`）与连续（`StartContinuousInputInjectionForAction` / `StopContinuousInputInjectionForAction`）两套 `BlueprintCallable` 入口；所有目标 `UInputAction` 对象名与路径已由 `field-idle` 快照的 `CPP_DA_MUCHA_INPUT` 与 `CDT_TagKeyGuide.json` 100% 证实。
   - **调用链**：
     1. 校验 `<bridge_dir>/input.enabled` 开关文件、白名单动作与最小调用间隔（50 ms）；
     2. 采样注入前的 `ui.screen_kind`、当前光标与 `input_blocked`；
     3. 通过 `CPP_InputManager:GetInputAction(MappingType, ActionName)` 取得 `UInputAction*`（失败时 `StaticFindObject(assetPath)` 兜底）；
     4. 通过 `EnhancedInputLibrary:MakeInputActionValueOfType(x, y, z, valueType)` 构造 `FInputActionValue`；离散点按优先尝试 `InjectInputForAction`（失败降级 `InjectInputVectorForAction`，若仍拿不到 `UInputAction*` 再降级 `InjectInputForPlayerMapping` / `InjectInputVectorForPlayerMapping`）；长按与野外 `move` 使用 `StartContinuousInputInjectionForAction` 并在 `ms` 毫秒后通过 `ExecuteWithDelay` + `ExecuteInGameThread` 调用 `StopContinuousInputInjectionForAction`；
     5. 每次调用写 `[FfrBridge] input ...` 日志，采样注入后 `ui.screen_kind` 和光标一并返回。
2. **备选方案 A（动作层按 `MappingName` 注入）**：当个别动作的 `UInputAction*` 指针无法从 `CPP_InputManager` 或 `StaticFindObject` 取得时，同一子系统上的 `InjectInputForPlayerMapping(MappingName, ...)` / `InjectInputVectorForPlayerMapping` 可直接按 `DT_OP_KeyboardMouseButtonList.json` 的 `MappingName` 注入，已在 `topics/input.lua` 中作为自动回退路径保留（**要真机验证**）。
3. **备选方案 B（界面层控件接口兜底）**：若真机实测发现后台/失焦状态下 `EnhancedInput` UI 动作被 `WindowDeactivate` 彻底拦住且无法前台化窗口，再专项评估通过活跃层焦点控件的 `CPP_BI_WidgetBasicInput:InputDecide/Cancel/Direction/TabSwitch` 实现 UI 专用的失焦兜底（仅在真机验证确认可行且不破坏开闭动画门控后才引入）。
4. **备选方案 C（宿主机窗口激活 + 物理键反查）**：若真机实测 `WindowDeactivate` 阻止全部后台输入，则在 AGP 侧执行按键前确保游戏窗口处于前景激活状态，仍优先走游戏内动作层注入，避免抢占用户物理键鼠；仅在动作层本身不可用时才退回按键层方案。

---

## 4. 白名单语义动作与 `IA_*` 映射依据

全部映射表定义在 `mod/FfrBridge/Scripts/ffrbridge/topics/input.lua`（`M.ACTIONS`），每项均来源于 `field-idle` 快照的 `CPP_DA_MUCHA_INPUT` 实例、`battle-jmap` 的 `eMuchaInputMappingType` 枚举、`CDT_TagKeyGuide.json` 与 `DT_OP_KeyboardMouseButtonList.json`：

| 语义名（`action`） | `eMuchaInputMappingType`（枚举值） | `GetInputAction` 的 `ActionName` | `InputAction` 资产路径 | `MappingName`（回退） | 值类型与默认向量 | 需真机核验项 |
| --- | --- | --- | --- | --- | --- | --- |
| `confirm` | `UI_Common` (`12`)；野外 `field` 自动切 `MapUnit` (`1`) | `UI_Common_Decide` / 野外 `UnitInteract` | `/Game/Input/UI/Common/InputAction/IA_UI_Common_Decide.IA_UI_Common_Decide`（野外 `IA_Map_UnitInteract`） | `UICommon_Decide` / `MapUnit_Interact` | `Boolean (0)`, `(1,0,0)` | 确认 `Triggers` 用 `InputTriggerPressed` 还是 `{}`；野外互动自动切换 |
| `cancel` | `UI_Common` (`12`) | `UI_Common_Cancel` | `/Game/Input/UI/Common/InputAction/IA_UI_Common_Cancel.IA_UI_Common_Cancel` | `UICommon_Cancel` | `Boolean (0)`, `(1,0,0)` | 弹窗关闭与战斗退回上一级 |
| `up` | `UI_Common` (`12`) | `UI_Common_Direction` | `/Game/Input/UI/Common/InputAction/IA_UI_Common_Direction.IA_UI_Common_Direction` | `UICommon_MoveForwards` | `Axis2D (2)`, `(0,1,0)` | **要真机验证** Y 轴正负号（`+1` 为上还是 `-1` 为上） |
| `down` | `UI_Common` (`12`) | `UI_Common_Direction` | `/Game/Input/UI/Common/InputAction/IA_UI_Common_Direction.IA_UI_Common_Direction` | `UICommon_MoveBackwards` | `Axis2D (2)`, `(0,-1,0)` | **要真机验证** Y 轴正负号与菜单/战斗单格步进 |
| `left` | `UI_Common` (`12`) | `UI_Common_Direction` | `/Game/Input/UI/Common/InputAction/IA_UI_Common_Direction.IA_UI_Common_Direction` | `UICommon_MoveLeft` | `Axis2D (2)`, `(-1,0,0)` | **要真机验证** 战斗目标切换与弹窗左右选项 |
| `right` | `UI_Common` (`12`) | `UI_Common_Direction` | `/Game/Input/UI/Common/InputAction/IA_UI_Common_Direction.IA_UI_Common_Direction` | `UICommon_MoveRight` | `Axis2D (2)`, `(1,0,0)` | **要真机验证** 战斗目标切换与弹窗左右选项 |
| `page_prev` | `UI_Common` (`12`) | `UI_Common_TabSwitch` | `/Game/Input/UI/Common/InputAction/IA_UI_Common_TabSwitch.IA_UI_Common_TabSwitch` | `UICommon_PreviewPage` | `Axis1D (1)`, `(-1,0,0)` | **要真机验证** `Axis1D` 正负号及 `CPP_EIT_AcceleHold` 触发表现 |
| `page_next` | `UI_Common` (`12`) | `UI_Common_TabSwitch` | `/Game/Input/UI/Common/InputAction/IA_UI_Common_TabSwitch.IA_UI_Common_TabSwitch` | `UICommon_NextPage` | `Axis1D (1)`, `(1,0,0)` | **要真机验证** `Axis1D` 正负号及 `CPP_EIT_AcceleHold` 触发表现 |
| `menu` | `UI_MainMenu` (`17`) | `Scene_Root` | `/Game/Input/UI/MainMenu/InputAction/IA_Map_OpenMainMenu_Root.IA_Map_OpenMainMenu_Root` | `Map_OpenMainMenu_Root` | `Boolean (0)`, `(1,0,0)` | 野外呼出主菜单根目录页 |
| `system_menu` | `UI_SystemMenu` (`23`) | `SystemMenu_Open` | `/Game/Input/UI/SystemMenu/InputAction/IA_UI_SystemMenu.IA_UI_SystemMenu` | `UICommon_SystemMenu` | `Boolean (0)`, `(1,0,0)` | 野外/界面呼出系统菜单（保存/读取/选项） |
| `map` | `UI_MainMenu` (`17`) | `Scene_Map` | `/Game/Input/UI/MainMenu/InputAction/IA_Map_OpenMainMenu_Map.IA_Map_OpenMainMenu_Map` | `Map_OpenMainMenu_Map` | `Boolean (0)`, `(1,0,0)` | 野外直接打开大地图（M 键） |
| `quick_heal` | `UI_MainMenu` (`17`) | `Scene_Heal` | `/Game/Input/UI/MainMenu/InputAction/IA_Map_OpenMainMenu_Heal.IA_Map_OpenMainMenu_Heal` | `Map_OpenMainMenu_Heal` | `Boolean (0)`, `(1,0,0)` | 野外直接打开快速回复（H 键） |
| `interact` | `MapUnit` (`1`) | `UnitInteract` | `/Game/Input/Action/Map/IA_Map_UnitInteract.IA_Map_UnitInteract` | `MapUnit_Interact` | `Boolean (0)`, `(1,0,0)` | 野外面向 NPC/宝箱/存档点显式触发交互 |
| `move` | `MapUnit` (`1`) | `UnitMove` | `/Game/Input/Action/Map/IA_Map_UnitMove.IA_Map_UnitMove` | `MapUnit_MoveForward` | `Axis2D (2)`, `(x,y,0)` 持续 `ms` ms | **要真机验证** 连续注入启动/停止及时性、位移幅度与旋转镜头区方向映射 |

---

## 5. 待真机验证清单

以下项目无法离线确认，需在下次连接游戏机并显式启用 `--enable-input` 后逐项验证：

1. **UE4SS 接口调用与触发器**：确认 `EnhancedInputLocalPlayerSubsystem:InjectInputForAction` 经接口绑定可直接调用，核对 `Triggers = { InputTriggerPressed }` 与 `{}` 的触发差异（避免单按变双触发或无触发）。
2. **窗口失焦行为**：窗口切到后台（`ui.input_blocked` 出现 `"WindowDeactivate"`）时，分别测试 `down`（UI 动作）、`map`（主菜单快捷键）、`move`（野外移动）是否被拦截，决定是否要求前景化或启用界面层兜底。
3. **轴向符号**：在系统菜单/战斗指令中验证 `up`/`down` 的 Y 轴符号，在战斗选目标/弹窗中验证 `left`/`right` 的 X 轴符号，在商店分类或教学翻页中验证 `page_prev`/`page_next` 的 `Axis1D` 正负号。
4. **野外连续移动停止安全性**：在野外执行 `move { direction: "up", ms: 400 }`，确认 `field.player.location` 沿预期方向移动后完全停下（`field.player.speed_2d` 归零、无持续奔跑残留）。
5. **跨场景动作映射**：验证野外 `confirm` 面向 NPC/宝箱触发 `IA_Map_UnitInteract`、对话与菜单中触发 `IA_UI_Common_Decide`；大地图内方向键是走 `IA_UI_Common_Direction` 还是 `IA_UI_WldMap_Cursor`。
