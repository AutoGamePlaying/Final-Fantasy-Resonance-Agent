-- FfrBridge R4 in-process input injection command module (disabled by default).
-- Exposes command `press` gated strictly by `<bridge_dir>/input.enabled`,
-- a semantic action whitelist, a minimum call interval (50 ms), and audit logging.
--
-- Primary injection path (see docs/input.md):
--   EnhancedInputLocalPlayerSubsystem (IEnhancedInputSubsystemInterface):
--     - Discrete tap: InjectInputForAction (fallback InjectInputVectorForAction / PlayerMapping)
--     - Hold / continuous move: StartContinuousInputInjectionForAction + delayed Stop
--   Matches the game's own WBP_TagKeyGuide_C:InjectInputAction blueprint pattern.
-- Never mutates game data, stats, save slots, or CPP_GameWidgetInputManager.InputBlockSet.

local bridgeIo = require("ffrbridge.io")
local json = require("ffrbridge.json")
local util = require("ffrbridge.util")

local M = {
  name = "input",
  commands = {},
}

M.SWITCH_FILE = "input.enabled"
M.MIN_INTERVAL_MS = 50
M.MAX_TIMES = 20
M.MAX_HOLD_MS = 5000
M.MAX_MOVE_MS = 5000

-- Enum values verified from battle-jmap (/Script/EnhancedInput.EInputActionValueType).
M.VALUE_TYPE = {
  Boolean = 0,
  Axis1D = 1,
  Axis2D = 2,
  Axis3D = 3,
}

-- Enum values verified from battle-jmap (/Script/mucha.eMuchaInputMappingType).
M.MAPPING_TYPE = {
  MapUnit = 1,
  UI_Common = 12,
  UI_MainMenu = 17,
  UI_SystemMenu = 23,
}

-- Semantic action whitelist and Enhanced Input mapping table.
-- Every mappingType, actionName, and assetPath is verified from field-idle snapshot
-- CPP_DA_MUCHA_INPUT assets, battle-jmap reflection data, CDT_TagKeyGuide.json,
-- and DT_OP_KeyboardMouseButtonList.json (see docs/input.md §4).
M.ACTIONS = {
  -- Confirm in UI / menus / dialogue / battle; automatically uses MapUnit UnitInteract
  -- when screen_kind == "field" (DT_OP_KeyboardMouseButtonList: UICommon_Decide copies MapUnit_Interact).
  confirm = {
    kind = "discrete",
    mappingType = M.MAPPING_TYPE.UI_Common,
    mappingTypeName = "UI_Common",
    actionName = "UI_Common_Decide",
    assetPath = "/Game/Input/UI/Common/InputAction/IA_UI_Common_Decide.IA_UI_Common_Decide",
    mappingName = "UICommon_Decide",
    valueType = M.VALUE_TYPE.Boolean,
    vector = { x = 1, y = 0, z = 0 },
    fieldOverride = {
      mappingType = M.MAPPING_TYPE.MapUnit,
      mappingTypeName = "MapUnit",
      actionName = "UnitInteract",
      assetPath = "/Game/Input/Action/Map/IA_Map_UnitInteract.IA_Map_UnitInteract",
      mappingName = "MapUnit_Interact",
      valueType = M.VALUE_TYPE.Boolean,
      vector = { x = 1, y = 0, z = 0 },
    },
    evidence = "DA_Input_UI_Common + DA_Input_MapUnit + CDT_TagKeyGuide(Cmd_Decide) + DT_OP_KeyboardMouseButtonList(UICommon_Decide/MapUnit_Interact)",
  },

  -- Cancel / back / close (DA_Input_UI_Common, CDT_TagKeyGuide Cmn_Back/Cmn_Cancel/Cmn_Close).
  cancel = {
    kind = "discrete",
    mappingType = M.MAPPING_TYPE.UI_Common,
    mappingTypeName = "UI_Common",
    actionName = "UI_Common_Cancel",
    assetPath = "/Game/Input/UI/Common/InputAction/IA_UI_Common_Cancel.IA_UI_Common_Cancel",
    mappingName = "UICommon_Cancel",
    valueType = M.VALUE_TYPE.Boolean,
    vector = { x = 1, y = 0, z = 0 },
    evidence = "DA_Input_UI_Common + CDT_TagKeyGuide(Cmn_Cancel) + DT_OP_KeyboardMouseButtonList(UICommon_Cancel)",
  },

  -- Cursor navigation up (Axis2D +Y; matches field m_OldInputActionValue North=(0,1); verify sign on live game).
  up = {
    kind = "discrete",
    mappingType = M.MAPPING_TYPE.UI_Common,
    mappingTypeName = "UI_Common",
    actionName = "UI_Common_Direction",
    assetPath = "/Game/Input/UI/Common/InputAction/IA_UI_Common_Direction.IA_UI_Common_Direction",
    mappingName = "UICommon_MoveForwards",
    valueType = M.VALUE_TYPE.Axis2D,
    vector = { x = 0, y = 1, z = 0 },
    evidence = "DA_Input_UI_Common(UI_Common_Direction) + DT_OP_KeyboardMouseButtonList(UICommon_MoveForwards)",
  },

  -- Cursor navigation down (Axis2D -Y; matches field m_OldInputActionValue South=(0,-1); verify sign on live game).
  down = {
    kind = "discrete",
    mappingType = M.MAPPING_TYPE.UI_Common,
    mappingTypeName = "UI_Common",
    actionName = "UI_Common_Direction",
    assetPath = "/Game/Input/UI/Common/InputAction/IA_UI_Common_Direction.IA_UI_Common_Direction",
    mappingName = "UICommon_MoveBackwards",
    valueType = M.VALUE_TYPE.Axis2D,
    vector = { x = 0, y = -1, z = 0 },
    evidence = "DA_Input_UI_Common(UI_Common_Direction) + DT_OP_KeyboardMouseButtonList(UICommon_MoveBackwards)",
  },

  -- Cursor / target navigation left (Axis2D -X; matches field m_OldInputActionValue West=(-1,0)).
  left = {
    kind = "discrete",
    mappingType = M.MAPPING_TYPE.UI_Common,
    mappingTypeName = "UI_Common",
    actionName = "UI_Common_Direction",
    assetPath = "/Game/Input/UI/Common/InputAction/IA_UI_Common_Direction.IA_UI_Common_Direction",
    mappingName = "UICommon_MoveLeft",
    valueType = M.VALUE_TYPE.Axis2D,
    vector = { x = -1, y = 0, z = 0 },
    evidence = "DA_Input_UI_Common(UI_Common_Direction) + DT_OP_KeyboardMouseButtonList(UICommon_MoveLeft)",
  },

  -- Cursor / target navigation right (Axis2D +X; matches field m_OldInputActionValue East=(1,0)).
  right = {
    kind = "discrete",
    mappingType = M.MAPPING_TYPE.UI_Common,
    mappingTypeName = "UI_Common",
    actionName = "UI_Common_Direction",
    assetPath = "/Game/Input/UI/Common/InputAction/IA_UI_Common_Direction.IA_UI_Common_Direction",
    mappingName = "UICommon_MoveRight",
    valueType = M.VALUE_TYPE.Axis2D,
    vector = { x = 1, y = 0, z = 0 },
    evidence = "DA_Input_UI_Common(UI_Common_Direction) + DT_OP_KeyboardMouseButtonList(UICommon_MoveRight)",
  },

  -- Previous tab / page (Q key; Axis1D -1 on IA_UI_Common_TabSwitch; verify sign on live game).
  page_prev = {
    kind = "discrete",
    mappingType = M.MAPPING_TYPE.UI_Common,
    mappingTypeName = "UI_Common",
    actionName = "UI_Common_TabSwitch",
    assetPath = "/Game/Input/UI/Common/InputAction/IA_UI_Common_TabSwitch.IA_UI_Common_TabSwitch",
    mappingName = "UICommon_PreviewPage",
    valueType = M.VALUE_TYPE.Axis1D,
    vector = { x = -1, y = 0, z = 0 },
    evidence = "DA_Input_UI_Common(UI_Common_TabSwitch) + CDT_TagKeyGuide(Cmn_Page_ChangePage) + DT_OP_KeyboardMouseButtonList(UICommon_PreviewPage)",
  },

  -- Next tab / page (E key; Axis1D +1 on IA_UI_Common_TabSwitch; verify sign on live game).
  page_next = {
    kind = "discrete",
    mappingType = M.MAPPING_TYPE.UI_Common,
    mappingTypeName = "UI_Common",
    actionName = "UI_Common_TabSwitch",
    assetPath = "/Game/Input/UI/Common/InputAction/IA_UI_Common_TabSwitch.IA_UI_Common_TabSwitch",
    mappingName = "UICommon_NextPage",
    valueType = M.VALUE_TYPE.Axis1D,
    vector = { x = 1, y = 0, z = 0 },
    evidence = "DA_Input_UI_Common(UI_Common_TabSwitch) + CDT_TagKeyGuide(Cmn_Page_ChangePage) + DT_OP_KeyboardMouseButtonList(UICommon_NextPage)",
  },

  -- Open Main Menu root (DA_Input_UI_MainMenu Scene_Root, CDT_TagKeyGuide PopupHelp_OpenMainMenu).
  menu = {
    kind = "discrete",
    mappingType = M.MAPPING_TYPE.UI_MainMenu,
    mappingTypeName = "UI_MainMenu",
    actionName = "Scene_Root",
    assetPath = "/Game/Input/UI/MainMenu/InputAction/IA_Map_OpenMainMenu_Root.IA_Map_OpenMainMenu_Root",
    mappingName = "Map_OpenMainMenu_Root",
    valueType = M.VALUE_TYPE.Boolean,
    vector = { x = 1, y = 0, z = 0 },
    evidence = "DA_Input_UI_MainMenu(Scene_Root) + CDT_TagKeyGuide(PopupHelp_OpenMainMenu) + DT_OP_KeyboardMouseButtonList(Map_OpenMainMenu_Root)",
  },

  -- Open System Menu (Save/Load/Option/Title; DA_Input_UI_SystemMenu SystemMenu_Open).
  system_menu = {
    kind = "discrete",
    mappingType = M.MAPPING_TYPE.UI_SystemMenu,
    mappingTypeName = "UI_SystemMenu",
    actionName = "SystemMenu_Open",
    assetPath = "/Game/Input/UI/SystemMenu/InputAction/IA_UI_SystemMenu.IA_UI_SystemMenu",
    mappingName = "UICommon_SystemMenu",
    valueType = M.VALUE_TYPE.Boolean,
    vector = { x = 1, y = 0, z = 0 },
    evidence = "DA_Input_UI_SystemMenu(SystemMenu_Open) + CDT_TagKeyGuide(SystemMenu) + DT_OP_KeyboardMouseButtonList(UICommon_SystemMenu)",
  },

  -- Open World Map directly (M key; DA_Input_UI_MainMenu Scene_Map).
  map = {
    kind = "discrete",
    mappingType = M.MAPPING_TYPE.UI_MainMenu,
    mappingTypeName = "UI_MainMenu",
    actionName = "Scene_Map",
    assetPath = "/Game/Input/UI/MainMenu/InputAction/IA_Map_OpenMainMenu_Map.IA_Map_OpenMainMenu_Map",
    mappingName = "Map_OpenMainMenu_Map",
    valueType = M.VALUE_TYPE.Boolean,
    vector = { x = 1, y = 0, z = 0 },
    evidence = "DA_Input_UI_MainMenu(Scene_Map) + CDT_TagKeyGuide(PopupHelp_Map) + DT_OP_KeyboardMouseButtonList(Map_OpenMainMenu_Map)",
  },

  -- Open Quick Heal page directly (H key; DA_Input_UI_MainMenu Scene_Heal confirmed in field-idle snapshot).
  quick_heal = {
    kind = "discrete",
    mappingType = M.MAPPING_TYPE.UI_MainMenu,
    mappingTypeName = "UI_MainMenu",
    actionName = "Scene_Heal",
    assetPath = "/Game/Input/UI/MainMenu/InputAction/IA_Map_OpenMainMenu_Heal.IA_Map_OpenMainMenu_Heal",
    mappingName = "Map_OpenMainMenu_Heal",
    valueType = M.VALUE_TYPE.Boolean,
    vector = { x = 1, y = 0, z = 0 },
    evidence = "DA_Input_UI_MainMenu(Scene_Heal, field-idle snapshot) + DT_OP_KeyboardMouseButtonList(Map_OpenMainMenu_Heal)",
  },

  -- Explicit field interact (talk to NPC / open chest / activate save point; DA_Input_MapUnit UnitInteract).
  interact = {
    kind = "discrete",
    mappingType = M.MAPPING_TYPE.MapUnit,
    mappingTypeName = "MapUnit",
    actionName = "UnitInteract",
    assetPath = "/Game/Input/Action/Map/IA_Map_UnitInteract.IA_Map_UnitInteract",
    mappingName = "MapUnit_Interact",
    valueType = M.VALUE_TYPE.Boolean,
    vector = { x = 1, y = 0, z = 0 },
    evidence = "DA_Input_MapUnit(UnitInteract) + WBP_ObjectInteract.WBP_KeyIcon + DT_OP_KeyboardMouseButtonList(MapUnit_Interact)",
  },

  -- Continuous field movement vector (Axis2D (x, y) held for ms; DA_Input_MapUnit UnitMove).
  move = {
    kind = "move",
    mappingType = M.MAPPING_TYPE.MapUnit,
    mappingTypeName = "MapUnit",
    actionName = "UnitMove",
    assetPath = "/Game/Input/Action/Map/IA_Map_UnitMove.IA_Map_UnitMove",
    mappingName = "MapUnit_MoveForward",
    valueType = M.VALUE_TYPE.Axis2D,
    vector = nil,
    evidence = "DA_Input_MapUnit(UnitMove) + CDT_TagKeyGuide(PopupHelp_Move) + field.md §1.1(m_OldInputActionValue)",
  },
}

local function sortedActionNames()
  local names = {}
  for name in pairs(M.ACTIONS) do
    names[#names + 1] = name
  end
  table.sort(names)
  return names
end

M.WHITELIST = sortedActionNames()

local function isInteger(value)
  return util.isFiniteNumber(value) and math.floor(value) == value
end

local function safeCall(obj, method, ...)
  if obj == nil then return false, nil, false end
  local okFn, fn = pcall(function() return obj[method] end)
  if not okFn then return false, fn, true end
  if fn == nil then return false, nil, false end
  local okCall, res = pcall(fn, obj, ...)
  if not okCall then return false, res, true end
  return true, res, true
end

local function safeProp(obj, prop)
  if obj == nil then return false, nil end
  return pcall(function() return obj[prop] end)
end

local function isValidObject(obj)
  local valid = util.isValidObject(obj)
  return valid == true
end

local function logLine(ctx, message)
  if ctx and type(ctx.log) == "function" then
    pcall(ctx.log, message)
    return
  end
  pcall(print, "[FfrBridge] " .. tostring(message) .. "\n")
end

local function nowMsOf(ctx)
  if ctx and type(ctx.nowMs) == "function" then
    local ok, value = pcall(ctx.nowMs)
    if ok and util.isFiniteNumber(value) then
      return math.max(0, math.floor(value))
    end
  end
  if ctx and util.isFiniteNumber(ctx.clockMs) then
    return math.max(0, math.floor(ctx.clockMs))
  end
  return 0
end

local function isSwitchEnabled(ctx)
  if not ctx or type(ctx.directory) ~= "string" or ctx.directory == "" then
    return false, nil
  end
  local path = bridgeIo.join(ctx.directory, M.SWITCH_FILE)
  return bridgeIo.fileExists(path, ctx.fs), path
end

-- Extract the primary cursor index/uid and its source path from UI (+ battle cmd if in battle).
local function extractCursor(uiData, ctx)
  if type(uiData) ~= "table" then return nil, nil end
  local kind = uiData.screen_kind

  if (kind == "dialog" or kind == "choice") and type(uiData.choice) == "table" and not json.isNull(uiData.choice) then
    if util.isFiniteNumber(uiData.choice.focus_index) then
      return math.floor(uiData.choice.focus_index), "ui.choice.focus_index"
    end
  end

  if (kind == "system_menu" or kind == "main_menu" or kind == "savepoint" or kind == "title")
    and type(uiData.menu) == "table" and not json.isNull(uiData.menu)
  then
    if util.isFiniteNumber(uiData.menu.focus_index) then
      return math.floor(uiData.menu.focus_index), "ui.menu.focus_index"
    end
    local sub = uiData.menu.sub_state
    if type(sub) == "table" and not json.isNull(sub) and util.isFiniteNumber(sub.focus_quest_id) then
      return math.floor(sub.focus_quest_id), "ui.menu.sub_state.focus_quest_id"
    end
  end

  if kind == "shop" and type(uiData.shop) == "table" and not json.isNull(uiData.shop) then
    local sceneStack = uiData.shop.scene_stack
    local inTrade = type(sceneStack) == "table" and #sceneStack >= 2
    if inTrade and util.isFiniteNumber(uiData.shop.item_focus_index) then
      return math.floor(uiData.shop.item_focus_index), "ui.shop.item_focus_index"
    end
    if util.isFiniteNumber(uiData.shop.cmd_focus_index) then
      return math.floor(uiData.shop.cmd_focus_index), "ui.shop.cmd_focus_index"
    end
    if util.isFiniteNumber(uiData.shop.item_focus_index) then
      return math.floor(uiData.shop.item_focus_index), "ui.shop.item_focus_index"
    end
  end

  if kind == "save_ui" and type(uiData.save_ui) == "table" and not json.isNull(uiData.save_ui) then
    if util.isFiniteNumber(uiData.save_ui.focus_index) then
      return math.floor(uiData.save_ui.focus_index), "ui.save_ui.focus_index"
    end
  end

  if kind == "tutorial" and type(uiData.tutorial) == "table" and not json.isNull(uiData.tutorial) then
    if util.isFiniteNumber(uiData.tutorial.page_index) then
      return math.floor(uiData.tutorial.page_index), "ui.tutorial.page_index"
    end
  end

  if kind == "battle_result" and type(uiData.battle_result) == "table" and not json.isNull(uiData.battle_result) then
    if util.isFiniteNumber(uiData.battle_result.page_index) then
      return math.floor(uiData.battle_result.page_index), "ui.battle_result.page_index"
    end
  end

  if kind == "battle" and ctx then
    local finder = type(ctx.freshFind) == "function" and ctx.freshFind or ctx.find
    if type(finder) == "function" then
      local okCmd, vmCmd = pcall(finder, "VM_BtlCmd_C")
      if okCmd and isValidObject(vmCmd) then
        local okUiBtl, uiBattle = pcall(finder, "CPP_UI_BattleSubsystem")
        local isTarget = false
        if okUiBtl and isValidObject(uiBattle) then
          local okMgr, cmdMgr = safeProp(uiBattle, "BtlCmdManager")
          if okMgr and isValidObject(cmdMgr) then
            local okSt, cmdState = safeProp(cmdMgr, "CmdState")
            if okSt then
              local stateStr = util.decodeEnum(cmdState, { [0] = "List", [1] = "Target" })
              isTarget = stateStr == "Target"
            end
          end
        end
        if isTarget then
          local okSel, selVm = safeProp(vmCmd, "UnitSelectVM")
          if okSel and isValidObject(selVm) then
            local okUid, focusUid = safeProp(selVm, "FocusBaseUniqueId")
            if okUid and util.isFiniteNumber(focusUid) and focusUid >= 0 then
              return math.floor(focusUid), "battle.cmd.target.focus_uid"
            end
          end
        end
        local okFocus, focusData = safeProp(vmCmd, "FocusCmdData")
        if okFocus and focusData ~= nil then
          local okIdx, entryIdx = safeProp(focusData, "EntryIndex")
          if okIdx and util.isFiniteNumber(entryIdx) and entryIdx >= 0 then
            return math.floor(entryIdx), "battle.cmd.focus_index"
          end
        end
      end
    end
  end

  return nil, nil
end

-- Sample current ui.screen_kind, cursor, and input_blocked via protocol's fresh UI sampler.
local function captureObservation(ctx)
  local uiData = nil
  if ctx and type(ctx.sampleUi) == "function" then
    local ok, sampled = pcall(ctx.sampleUi)
    if ok and type(sampled) == "table" and not json.isNull(sampled) then
      uiData = sampled
    end
  else
    local okReq, uiTopic = pcall(require, "ffrbridge.topics.ui")
    if okReq and type(uiTopic) == "table" and type(uiTopic.collect) == "function" then
      local uiCtx = {
        find = ctx and ctx.find or nil,
        findAll = ctx and ctx.findAll or nil,
        staticFindObject = ctx and ctx.staticFindObject or nil,
        err = function() end,
        cache = {},
        clockMs = nowMsOf(ctx),
        tick = ctx and ctx.tick or 0,
      }
      local okCollect, sampled = pcall(uiTopic.collect, uiCtx)
      if okCollect and type(sampled) == "table" and not json.isNull(sampled) then
        uiData = sampled
      end
    end
  end

  local screenKind = (type(uiData) == "table" and type(uiData.screen_kind) == "string")
    and uiData.screen_kind or nil
  local cursor, cursorSource = extractCursor(uiData, ctx)
  local inputBlocked = {}
  if type(uiData) == "table" and type(uiData.input_blocked) == "table" then
    for index = 1, #uiData.input_blocked do
      local token = uiData.input_blocked[index]
      if type(token) == "string" and token ~= "" then
        inputBlocked[#inputBlocked + 1] = token
      end
    end
  end

  return {
    screen_kind = screenKind or json.null,
    cursor = cursor ~= nil and cursor or json.null,
    cursor_source = cursorSource or json.null,
    input_blocked = json.array(inputBlocked),
  }
end

local function hasWindowDeactivate(observation)
  if type(observation) ~= "table" or type(observation.input_blocked) ~= "table" then
    return false
  end
  for index = 1, #observation.input_blocked do
    if observation.input_blocked[index] == "WindowDeactivate" then
      return true
    end
  end
  return false
end

local function resolveTargetSpec(actionName, spec, beforeObs)
  if spec.fieldOverride and beforeObs and beforeObs.screen_kind == "field" then
    return spec.fieldOverride
  end
  return spec
end

local function shortAssetName(path)
  if type(path) ~= "string" then return nil end
  return path:match("([^./]+)$") or path
end

local function toFName(str)
  local fnameCtor = rawget(_G, "FName")
  if type(fnameCtor) == "function" then
    local ok, fname = pcall(fnameCtor, str)
    if ok and fname ~= nil then return fname end
  end
  return str
end

-- Resolve UInputAction* via CPP_InputManager:GetInputAction(MappingType, ActionName),
-- falling back to StaticFindObject(assetPath).
local function resolveInputAction(ctx, target)
  local inputMgr = ctx and type(ctx.find) == "function" and ctx.find("CPP_InputManager") or nil
  if isValidObject(inputMgr) then
    local okGet, actionObj = safeCall(inputMgr, "GetInputAction", target.mappingType, target.actionName)
    if okGet and isValidObject(actionObj) then
      return actionObj, "CPP_InputManager:GetInputAction"
    end
  end
  if ctx and type(ctx.staticFindObject) == "function" and type(target.assetPath) == "string" then
    local obj = ctx.staticFindObject(target.assetPath)
    if isValidObject(obj) then
      return obj, "StaticFindObject"
    end
  end
  return nil, nil
end

-- Build FInputActionValue via EnhancedInputLibrary:MakeInputActionValueOfType(x, y, z, valueType).
local function makeActionValue(ctx, vx, vy, vz, valueType)
  local lib = nil
  if ctx and type(ctx.staticFindObject) == "function" then
    lib = ctx.staticFindObject("/Script/EnhancedInput.Default__EnhancedInputLibrary")
  end
  if not isValidObject(lib) and ctx and type(ctx.find) == "function" then
    lib = ctx.find("EnhancedInputLibrary")
  end
  if not isValidObject(lib) then return nil end
  local okMake, val = safeCall(lib, "MakeInputActionValueOfType", vx, vy, vz, valueType)
  if okMake and val ~= nil then return val end
  return nil
end

-- Optionally spawn InputTriggerPressed (matching WBP_TagKeyGuide_C:InjectInputAction) for one-shot taps.
local function makePressedTriggers(ctx, subsys)
  if not ctx or type(ctx.staticFindObject) ~= "function" then return {} end
  local triggerClass = ctx.staticFindObject("/Script/EnhancedInput.InputTriggerPressed")
  if not isValidObject(triggerClass) then return {} end
  local statics = ctx.staticFindObject("/Script/Engine.Default__GameplayStatics")
  if not isValidObject(statics) and type(ctx.find) == "function" then
    statics = ctx.find("GameplayStatics")
  end
  if not isValidObject(statics) then return {} end
  local okSpawn, triggerObj = safeCall(statics, "SpawnObject", triggerClass, subsys)
  if okSpawn and isValidObject(triggerObj) then
    return { triggerObj }
  end
  return {}
end

local function scheduleDelayed(ctx, delayMs, fn)
  if ctx and type(ctx.scheduleDelayed) == "function" then
    return ctx.scheduleDelayed(delayMs, fn)
  end
  local execDelay = rawget(_G, "ExecuteWithDelay")
  local execGame = rawget(_G, "ExecuteInGameThread")
  if type(execDelay) == "function" then
    execDelay(delayMs, function()
      if type(execGame) == "function" then
        pcall(execGame, function() pcall(fn) end)
      else
        pcall(fn)
      end
    end)
    return true
  end
  return false
end

local function stopActiveContinuous(cache, subsys)
  if type(cache) ~= "table" or type(cache.activeContinuous) ~= "table" then return end
  local active = cache.activeContinuous
  cache.activeContinuous = nil
  local targetSubsys = isValidObject(subsys) and subsys or active.subsys
  if not isValidObject(targetSubsys) then return end
  if active.stopMethod == "StopContinuousInputInjectionForAction" and isValidObject(active.actionObj) then
    safeCall(targetSubsys, "StopContinuousInputInjectionForAction", active.actionObj)
  elseif active.stopMethod == "StopContinuousInputInjectionForPlayerMapping" and active.mappingName then
    safeCall(targetSubsys, "StopContinuousInputInjectionForPlayerMapping", active.mappingName)
  end
end

-- Execute a single discrete pulse on EnhancedInputLocalPlayerSubsystem.
local function injectDiscreteOnce(ctx, subsys, actionObj, target, vx, vy, vz)
  local rawValue = makeActionValue(ctx, vx, vy, vz, target.valueType)
  local triggers = makePressedTriggers(ctx, subsys)
  local vec = { X = vx, Y = vy, Z = vz }
  local modifiers = {}

  if isValidObject(actionObj) then
    if rawValue ~= nil then
      local ok, err, existed = safeCall(subsys, "InjectInputForAction", actionObj, rawValue, modifiers, triggers)
      if ok then return "InjectInputForAction", nil end
      if existed and err ~= nil then
        -- Fall through to vector overload if raw struct marshaling fails.
        local okVec, errVec = safeCall(subsys, "InjectInputVectorForAction", actionObj, vec, modifiers, triggers)
        if okVec then return "InjectInputVectorForAction", nil end
        return nil, "InjectInputForAction 失败：" .. tostring(err) .. "；Vector 回退失败：" .. tostring(errVec)
      end
    end
    local okVec, errVec, existedVec = safeCall(subsys, "InjectInputVectorForAction", actionObj, vec, modifiers, triggers)
    if okVec then return "InjectInputVectorForAction", nil end
    if existedVec then
      return nil, "InjectInputVectorForAction 失败：" .. tostring(errVec)
    end
  end

  if type(target.mappingName) == "string" and target.mappingName ~= "" then
    local mappingKey = toFName(target.mappingName)
    if rawValue ~= nil then
      local okMap, errMap, existedMap = safeCall(
        subsys,
        "InjectInputForPlayerMapping",
        mappingKey,
        rawValue,
        modifiers,
        triggers
      )
      if okMap then return "InjectInputForPlayerMapping", nil end
      if existedMap and errMap ~= nil then
        local okVec, errVec = safeCall(
          subsys,
          "InjectInputVectorForPlayerMapping",
          mappingKey,
          vec,
          modifiers,
          triggers
        )
        if okVec then return "InjectInputVectorForPlayerMapping", nil end
        return nil, "InjectInputForPlayerMapping 失败：" .. tostring(errMap) .. "；Vector 回退失败：" .. tostring(errVec)
      end
    end
    local okVec, errVec, existedVec = safeCall(
      subsys,
      "InjectInputVectorForPlayerMapping",
      mappingKey,
      vec,
      modifiers,
      triggers
    )
    if okVec then return "InjectInputVectorForPlayerMapping", nil end
    if existedVec then
      return nil, "InjectInputVectorForPlayerMapping 失败：" .. tostring(errVec)
    end
  end

  return nil, "EnhancedInputLocalPlayerSubsystem 缺少可用的输入注入方法或找不到目标 InputAction"
end

-- Start a continuous input injection and schedule automatic stop after durationMs.
local function injectContinuous(ctx, cache, subsys, actionObj, target, vx, vy, vz, durationMs)
  stopActiveContinuous(cache, subsys)

  local rawValue = makeActionValue(ctx, vx, vy, vz, target.valueType)
  local modifiers = {}
  local triggers = {}
  local method = nil
  local stopMethod = nil
  local mappingKey = nil

  if isValidObject(actionObj) and rawValue ~= nil then
    local okStart, errStart, existedStart = safeCall(
      subsys,
      "StartContinuousInputInjectionForAction",
      actionObj,
      rawValue,
      modifiers,
      triggers
    )
    if okStart then
      method = "StartContinuousInputInjectionForAction"
      stopMethod = "StopContinuousInputInjectionForAction"
    elseif existedStart and errStart ~= nil then
      return nil, "StartContinuousInputInjectionForAction 失败：" .. tostring(errStart)
    end
  end

  if method == nil and type(target.mappingName) == "string" and target.mappingName ~= "" and rawValue ~= nil then
    mappingKey = toFName(target.mappingName)
    local okMap, errMap, existedMap = safeCall(
      subsys,
      "StartContinuousInputInjectionForPlayerMapping",
      mappingKey,
      rawValue,
      modifiers,
      triggers
    )
    if okMap then
      method = "StartContinuousInputInjectionForPlayerMapping"
      stopMethod = "StopContinuousInputInjectionForPlayerMapping"
    elseif existedMap and errMap ~= nil then
      return nil, "StartContinuousInputInjectionForPlayerMapping 失败：" .. tostring(errMap)
    end
  end

  if method == nil then
    -- Fallback to single-frame injection if continuous injection methods are unavailable.
    local singleMethod, singleErr = injectDiscreteOnce(ctx, subsys, actionObj, target, vx, vy, vz)
    if not singleMethod then return nil, singleErr end
    return singleMethod, nil, false
  end

  cache.continuousSeq = (cache.continuousSeq or 0) + 1
  local token = cache.continuousSeq
  cache.activeContinuous = {
    token = token,
    subsys = subsys,
    actionObj = actionObj,
    mappingName = mappingKey,
    stopMethod = stopMethod,
  }

  local scheduled = scheduleDelayed(ctx, durationMs, function()
    if type(cache.activeContinuous) == "table" and cache.activeContinuous.token == token then
      stopActiveContinuous(cache, subsys)
      logLine(ctx, "input stop_continuous action=" .. tostring(target.actionName) .. " ms=" .. tostring(durationMs))
    end
  end)
  if not scheduled then
    -- No async timer available (e.g. synchronous test runner without scheduler): stop cleanly.
    stopActiveContinuous(cache, subsys)
  end

  return method, nil, scheduled
end

local function parseMoveParams(args)
  local src = type(args.move) == "table" and not json.isNull(args.move) and args.move or args
  local x = src.x
  local y = src.y
  local ms = src.ms
  if ms == nil then ms = args.ms end
  if ms == nil then ms = args.hold_ms end

  if not util.isFiniteNumber(x) or not util.isFiniteNumber(y) then
    return nil, "move 动作必须提供有限数字坐标 x 和 y"
  end
  if x < -1 or x > 1 or y < -1 or y > 1 then
    return nil, "move 的 x 和 y 必须在 [-1, 1] 范围内"
  end
  local magSq = x * x + y * y
  if magSq <= 0 then
    return nil, "move 的方向向量 (x, y) 不能为零向量"
  end
  if magSq > 1.0001 then
    local mag = math.sqrt(magSq)
    x = x / mag
    y = y / mag
  end

  if not isInteger(ms) or ms < 1 or ms > M.MAX_MOVE_MS then
    return nil, "move 的持续时间 ms 必须是 1.." .. tostring(M.MAX_MOVE_MS) .. " 的整数"
  end
  if args.times ~= nil and (not isInteger(args.times) or args.times ~= 1) then
    return nil, "move 动作不支持 times > 1"
  end
  return { x = x, y = y, ms = ms }, nil
end

local function parseDiscreteParams(args)
  local times = 1
  if args.times ~= nil and not json.isNull(args.times) then
    if not isInteger(args.times) or args.times < 1 or args.times > M.MAX_TIMES then
      return nil, "times 必须是 1.." .. tostring(M.MAX_TIMES) .. " 的整数"
    end
    times = args.times
  end
  local holdMs = 0
  if args.hold_ms ~= nil and not json.isNull(args.hold_ms) then
    if not isInteger(args.hold_ms) or args.hold_ms < 0 or args.hold_ms > M.MAX_HOLD_MS then
      return nil, "hold_ms 必须是 0.." .. tostring(M.MAX_HOLD_MS) .. " 的整数"
    end
    holdMs = args.hold_ms
  end
  return { times = times, holdMs = holdMs }, nil
end

local function formatObsLog(obs)
  if type(obs) ~= "table" then return "unknown:null" end
  local sk = (type(obs.screen_kind) == "string") and obs.screen_kind or "null"
  local cur = util.isFiniteNumber(obs.cursor) and tostring(obs.cursor) or "null"
  return sk .. ":" .. cur
end

-- Command: press { action, times?, hold_ms?, x?, y?, ms? }
M.commands.press = function(ctx, args)
  local safeArgs = (type(args) == "table" and not json.isNull(args)) and args or {}
  local action = safeArgs.action

  -- 1. Hard gate: `<bridge_dir>/input.enabled` MUST exist.
  local enabled, switchPath = isSwitchEnabled(ctx)
  if not enabled then
    logLine(
      ctx,
      "input rejected reason=disabled action="
        .. tostring(action)
        .. " switch="
        .. tostring(switchPath or M.SWITCH_FILE)
    )
    return nil, "输入未开启：桥目录缺少 " .. M.SWITCH_FILE .. " 开关文件"
  end

  -- 2. Strict semantic action whitelist validation.
  if type(action) ~= "string" or action == "" then
    logLine(ctx, "input rejected reason=missing_action")
    return nil, "缺少有效的动作名 action"
  end
  local spec = M.ACTIONS[action]
  if not spec then
    logLine(ctx, "input rejected reason=not_whitelisted action=" .. tostring(action))
    return nil, "不允许的输入动作：" .. tostring(action) .. "（不在白名单中）"
  end

  -- 3. Parameter validation by action kind.
  local parsed, parseErr
  if spec.kind == "move" then
    parsed, parseErr = parseMoveParams(safeArgs)
  else
    parsed, parseErr = parseDiscreteParams(safeArgs)
  end
  if not parsed then
    logLine(ctx, "input rejected reason=invalid_args action=" .. action .. " err=" .. tostring(parseErr))
    return nil, parseErr
  end

  -- 4. Minimum interval guard to prevent accidental rapid double-firing.
  local cache = (ctx and type(ctx.cache) == "table") and ctx.cache or {}
  local minIntervalMs = (ctx and util.isFiniteNumber(ctx.minIntervalMs))
    and math.max(0, math.floor(ctx.minIntervalMs)) or M.MIN_INTERVAL_MS
  local nowMs = nowMsOf(ctx)
  if util.isFiniteNumber(cache.lastInvokeClockMs) and nowMs >= cache.lastInvokeClockMs then
    local elapsed = nowMs - cache.lastInvokeClockMs
    if elapsed < minIntervalMs then
      logLine(
        ctx,
        "input rejected reason=rate_limited action="
          .. action
          .. " elapsed_ms="
          .. tostring(elapsed)
          .. " min_interval_ms="
          .. tostring(minIntervalMs)
      )
      return nil,
        "输入调用过于频繁：距离上次按键仅 "
          .. tostring(elapsed)
          .. " 毫秒（最小间隔 "
          .. tostring(minIntervalMs)
          .. " 毫秒）"
    end
  end

  -- 5. Locate EnhancedInputLocalPlayerSubsystem.
  local subsys = ctx and type(ctx.find) == "function" and ctx.find("EnhancedInputLocalPlayerSubsystem") or nil
  if not isValidObject(subsys) then
    logLine(ctx, "input failed reason=no_subsystem action=" .. action)
    return nil, "找不到有效的 EnhancedInputLocalPlayerSubsystem，无法注入输入"
  end

  -- 6. Capture pre-injection UI screen_kind and cursor, then resolve target InputAction.
  local beforeObs = captureObservation(ctx)
  local target = resolveTargetSpec(action, spec, beforeObs)
  local actionObj, resolvedVia = resolveInputAction(ctx, target)

  local vx, vy, vz
  local durationMs = 0
  local times = 1
  if spec.kind == "move" then
    vx, vy, vz = parsed.x, parsed.y, 0
    durationMs = parsed.ms
  else
    vx, vy, vz = target.vector.x, target.vector.y, target.vector.z
    times = parsed.times
    durationMs = parsed.holdMs
  end

  -- 7. Inject input on EnhancedInputLocalPlayerSubsystem.
  local method, injectErr
  local scheduledStop = false
  if durationMs > 0 then
    method, injectErr, scheduledStop = injectContinuous(
      ctx,
      cache,
      subsys,
      actionObj,
      target,
      vx,
      vy,
      vz,
      durationMs
    )
    if method and times > 1 then
      local stepMs = durationMs + math.max(minIntervalMs, M.MIN_INTERVAL_MS)
      for step = 2, times do
        local scheduled = scheduleDelayed(ctx, (step - 1) * stepMs, function()
          if isValidObject(subsys) then
            injectContinuous(ctx, cache, subsys, actionObj, target, vx, vy, vz, durationMs)
            logLine(
              ctx,
              "input repeat action="
                .. action
                .. " step="
                .. tostring(step)
                .. "/"
                .. tostring(times)
                .. " hold_ms="
                .. tostring(durationMs)
            )
          end
        end)
        if not scheduled then
          local _, repeatErr = injectContinuous(
            ctx,
            cache,
            subsys,
            actionObj,
            target,
            vx,
            vy,
            vz,
            durationMs
          )
          if repeatErr then
            injectErr = repeatErr
            break
          end
        end
      end
    end
  else
    stopActiveContinuous(cache, subsys)
    method, injectErr = injectDiscreteOnce(ctx, subsys, actionObj, target, vx, vy, vz)
    if method and times > 1 then
      local stepMs = math.max(minIntervalMs, M.MIN_INTERVAL_MS)
      for step = 2, times do
        local scheduled = scheduleDelayed(ctx, (step - 1) * stepMs, function()
          if isValidObject(subsys) then
            injectDiscreteOnce(ctx, subsys, actionObj, target, vx, vy, vz)
            logLine(
              ctx,
              "input repeat action=" .. action .. " step=" .. tostring(step) .. "/" .. tostring(times)
            )
          end
        end)
        if not scheduled then
          local _, repeatErr = injectDiscreteOnce(ctx, subsys, actionObj, target, vx, vy, vz)
          if repeatErr then
            injectErr = repeatErr
            break
          end
        end
      end
    end
  end

  if not method then
    logLine(ctx, "input failed action=" .. action .. " err=" .. tostring(injectErr))
    return nil, tostring(injectErr)
  end

  cache.lastInvokeClockMs = nowMs

  -- 8. Capture post-injection UI screen_kind and cursor, and write audit log.
  local afterObs = captureObservation(ctx)
  local windowDeactivated = hasWindowDeactivate(beforeObs) or hasWindowDeactivate(afterObs)
  local iaShort = shortAssetName(target.assetPath) or target.actionName

  local logParts = {
    "input ok",
    "action=" .. action,
    "method=" .. method,
    "ia=" .. tostring(iaShort),
    "before=" .. formatObsLog(beforeObs),
    "after=" .. formatObsLog(afterObs),
  }
  if spec.kind == "move" then
    logParts[#logParts + 1] = "x=" .. tostring(vx)
    logParts[#logParts + 1] = "y=" .. tostring(vy)
    logParts[#logParts + 1] = "ms=" .. tostring(durationMs)
  else
    logParts[#logParts + 1] = "times=" .. tostring(times)
    logParts[#logParts + 1] = "hold_ms=" .. tostring(durationMs)
  end
  if windowDeactivated then
    logParts[#logParts + 1] = "blocked=WindowDeactivate"
  end
  logLine(ctx, table.concat(logParts, " "))

  local result = {
    action = action,
    times = times,
    hold_ms = durationMs,
    injected = {
      method = method,
      input_action = iaShort,
      asset_path = target.assetPath,
      action_name = target.actionName,
      mapping_type = target.mappingTypeName,
      mapping_name = target.mappingName,
      resolved_via = resolvedVia or "PlayerMapping",
      vector = { x = vx, y = vy, z = vz },
      continuous_stop_scheduled = scheduledStop,
    },
    before = beforeObs,
    after = afterObs,
    window_deactivated = windowDeactivated,
  }
  if spec.kind == "move" then
    result.x = vx
    result.y = vy
    result.ms = durationMs
  end

  return result, nil
end

return M
