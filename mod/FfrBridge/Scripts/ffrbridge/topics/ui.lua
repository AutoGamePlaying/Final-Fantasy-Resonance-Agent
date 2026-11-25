-- FfrBridge v1 topic "ui" (task 11-ui).
-- Read-only: only reads UObject properties and calls pure getters (Get*/Is*/Can*/Find*).
-- Never mutates game state and never sends input.
--
-- Contract (implemented by state.lua, task 11-field):
--   M.collect(ctx) -> table|nil, called every tick on the game thread inside pcall.
--   ctx.find(className) -> object|nil      (FindFirstOf with IsValid check, cached per tick)
--   ctx.findAll(className) -> list          (FindAllOf, invalid objects dropped)
--   ctx.err(field, message)                 (record one error into data.errors; never throw)
--   ctx.cache                               (table private to this topic, kept across ticks)
--   ctx.clockMs, ctx.tick                   (monotonic ms since load, tick counter)
--   M.commands[name] = function(ctx, args) -> result|nil, errorMessage   (on-demand, read-only)
--
-- Performance (per 500ms tick):
--   - Steady state (field / title, warm cache): 1 ctx.find("World") for map-change detection
--     (reuses state.lua's per-tick World lookup -> 0 extra FindFirstOf), 0 singleton lookups
--     (all GameInstance / World subsystems served from ctx.cache with IsValid() checks),
--     and 0 FindAllOf calls.
--   - Cold start / first tick after map travel: up to 8-9 singleton ctx.find calls
--     (World, CPP_GameWidgetManagerSubsystem, CPP_ActionGuideSubsystem, CPP_MovieSubsystem,
--      CPP_TalkEventSubsystem, CPP_TutorialSubsystem, CPP_TaskManager,
--      CPP_UI_BattleSubsystem, CPP_MainMenuSubsystem).
--   - Active overlay / menu / shop: 0-2 targeted singleton lookups (e.g. CPP_BFL_TalkEvent,
--     VM_Tooltip_C, or active main-menu subpage VM) only while that screen is open.
--   - Never calls ctx.findAll; widget children are traversed via active layer stack pointers.
--
-- v1 字段表（供 docs/bridge.md 汇总；全部在 data.ui 命名空间下）:
--   常驻轻量路由字段（每 0.5s 输出，5 项）:
--     1. ui.screen_kind       : string
--        按固定优先级汇总派生的画面仲裁值：
--        "loading" -> "movie" -> "tutorial" -> "dialog" -> "choice" -> "talk" -> "cutscene"
--        -> "save_ui" -> "system_menu" -> "shop" -> "savepoint" -> "main_menu"
--        -> "battle_result" -> "battle" -> "title" -> "field"
--     2. ui.active_layer      : string | null
--        WBP_OverAllLayout_C 当前输入目标图层的 GameplayTag（如 "UI.Layer.System"）
--     3. ui.active_widget     : string | null
--        活跃图层栈顶控件短类名（检查 IsActivated 且非开闭动画中，过滤 CommonUI 池化残留）
--     4. ui.action_guide_tag  : string | null
--        CPP_ActionGuideSubsystem.GuideTag.TagName（"None" 输出 null）
--     5. ui.input_blocked     : string[]
--        CPP_GameWidgetInputManager.InputBlockSet.TokenSet（兜底 CPP_BFL_UI_Input:IsBlocked()）
--
--   按需展开界面子结构（仅对应界面活跃时非空，否则为 null，8 项）:
--     6. ui.movie             : { active: boolean, subtitle: string | null } | null
--        触发：CPP_MovieSubsystem.m_pMovieScreen 有效
--     7. ui.talk              : {
--          event_id: string | null, window_class: string | null, is_cutscene: boolean,
--          speaker: string | null, text: string | null, waiting_key: boolean,
--          text_finished: boolean | null, can_skip: boolean, is_auto: boolean
--        } | null
--        触发：CPP_TalkEventSubsystem.m_pEventManager 有效，且非 movie/shop/savepoint/save_ui
--     8. ui.choice            : {
--          source: "dialog" | "event_choice", dialog_tag: string | null,
--          choice_tag: string | null, message: string, options: string[],
--          focus_index: integer | null
--        } | null
--        触发：CPP_DialogManager.DialogWidget 有效，或事件 WidgetStack 栈顶为 *ChoiceDialog*
--     9. ui.tutorial          : {
--          name: string, page_index: integer, page_count: integer,
--          caption: string, text: string, can_close: boolean
--        } | null
--        触发：CPP_TutorialSubsystem.m_TutorialWidget 有效
--    10. ui.menu              : {
--          kind: "system_menu" | "main_menu" | "savepoint" | "title",
--          scene_stack: string[], entries: string[], focus_index: integer | null,
--          sub_state: table | null
--        } | null
--        触发：Stack_System 栈顶为 WBP_SystemMenuLayout_C / WBP_SavePointLayout_C，
--        或主菜单 SceneDataStack 非空，或 Stack_Game 栈顶为 WBP_TitleLayout_C。
--        sub_state 按主菜单子页展开：
--          - Journal: { display_quest_category, focus_quest_id (对 DT_QuestData.ID),
--                       quests: [{ id (对 DT_QuestData.ID), name, subtitle }] }
--          - Heal:    { current_state, unit_id (对 DT_UnitParameter.ID),
--                       target_unit_id (对 DT_UnitParameter.ID), can_aoe,
--                       actions: [{ category, id (对 DT_ItemData.ID / DT_SkillData.ID),
--                                   count, user_unit_id }] }
--          - Item:    { current_state }
--          - Map:     { current_island_type, current_area_id }
--    11. ui.shop              : {
--          shop_id: integer | null (对 DT_ShopList.ID), shop_type: string | null,
--          scene_stack: string[], is_buy: boolean | null,
--          category_index: integer | null, categories: string[],
--          cmd_focus_index: integer | null, item_focus_index: integer | null,
--          tooltip_item_id: integer | null (对 DT_ItemData.ID),
--          total_amount: integer | null,
--          items: [{ id (对 DT_ItemData.ID), name, price, owned, stock, limit }]
--        } | null
--        触发：Stack_System 栈顶为 WBP_SHOP_Layout_C 且 CPP_VM_Colosseum_Shop 有效
--    12. ui.save_ui           : {
--          mode: "save" | "load", slot_indices: integer[], focus_index: integer | null
--        } | null
--        触发：Stack_System_Front1 栈顶为 WBP_SaveLayout_C 或 WBP_LoadLayout_C
--    13. ui.battle_result     : {
--          page_index: integer | null, has_level_up_window: boolean
--        } | null
--        触发：task_class == "CPP_BtlTaskWin" 且 ResultWidget 有效且 IsFinishPage == false
--
--   按需只读命令（M.commands）:
--     - input_state(args?): args 可选 { mapping_names?: string[], slot?: integer }
--       返回 { active_imcs: string[], swap_decide: boolean | null,
--              input_enabled: boolean | null, input_blocked: string[],
--              key_bindings: { [mappingName]: string | null } }

local json = require("ffrbridge.json")

local M = { name = "ui", commands = {} }

local TEXT_LIMIT = 200
local MAX_ARRAY_ITEMS = 64

-- Enum lookup tables verified from battle jmap reflection data.
local SHOP_TYPE_ENUM = {
  [0] = "None",
  [1] = "ToolShop",
  [2] = "WeaponShop",
  [3] = "ArmorShop",
  [4] = "AccessoryShop",
  [5] = "VariousShops",
  [6] = "Other",
}

local SHOP_SCENE_ENUM = {
  [0] = "None",
  [1] = "Command",
  [2] = "Shop",
}

local QUEST_CATEGORY_ENUM = {
  [0] = "None",
  [1] = "Main",
  [2] = "Sub",
  [3] = "Mini",
}

local ACTION_CATEGORY_ENUM = {
  [0] = "None",
  [1] = "Skill",
  [2] = "Item",
}

local ISLAND_TYPE_ENUM = {
  [0] = "None",
  [1] = "Grandshelt",
  [2] = "Lanzelt",
  [3] = "Kolobos",
  [4] = "Dirnado",
  [5] = "Olderion",
  [6] = "Zoldaad",
  [7] = "MagiNationMysidia",
  [8] = "Gronoa",
  [9] = "Pharm",
  [10] = "LandOfTheCrystals",
  [11] = "Ardra",
  [12] = "AirShip",
  [13] = "AirShip_Ride",
  [14] = "Common",
}

local SAVEPOINT_CMD_ENUM = {
  [0] = "None",
  [1] = "SavePoint",
  [2] = "Tent",
  [3] = "Teleport",
  [4] = "Teleport_Exit",
  [5] = "DLC",
}

local TITLE_ENTRY_ENUM = {
  [0] = "Continue",
  [1] = "Load",
  [2] = "NewGame",
  [3] = "Option",
  [4] = "QuitGame",
}

local INPUT_MAPPING_TYPE_ENUM = {
  [0] = "None",
  [1] = "MapUnit",
  [2] = "MapUnitDash",
  [3] = "Race",
  [4] = "RaceUnit",
  [5] = "TitleMenu",
  [6] = "Battle",
  [7] = "OptionMenu",
  [8] = "Shop",
  [9] = "SoundPlayer",
  [10] = "Option_InputSetting",
  [11] = "UI_Battle",
  [12] = "UI_Common",
  [13] = "UI_Equip",
  [14] = "UI_Field",
  [15] = "UI_FieldSideMenu",
  [16] = "UI_Info",
  [17] = "UI_MainMenu",
  [18] = "UI_MapCommon",
  [19] = "UI_PartyEdit",
  [20] = "UI_PartyHeal",
  [21] = "UI_OmakaseCure",
  [22] = "UI_ActionEquip",
  [23] = "UI_SystemMenu",
  [24] = "UI_WldMap",
  [25] = "UI_TextEdit",
  [26] = "UI_ChatList",
  [27] = "UI_Chat",
  [28] = "UI_Event",
  [29] = "UI_EventPause",
  [30] = "UI_Backlog",
  [31] = "UI_Quest",
  [32] = "UI_Key",
  [33] = "UI_Item",
}

-- Fallback mapping from WBP_OverAllLayout_C Stack_* slot property to GameplayTag.
local LAYOUT_STACK_SLOTS = {
  { prop = "Stack_Modal", tag = "UI.Layer.Modal" },
  { prop = "Stack_System_Front2", tag = "UI.Layer.System.Front2" },
  { prop = "Stack_System_Front1", tag = "UI.Layer.System.Front1" },
  { prop = "Stack_System", tag = "UI.Layer.System" },
  { prop = "Stack_Event", tag = "UI.Layer.Event" },
  { prop = "Stack_Game_Front", tag = "UI.Layer.Game.Front" },
  { prop = "Stack_Game", tag = "UI.Layer.Game" },
  { prop = "Stack_Save", tag = "UI.Layer.Save" },
  { prop = "Stack_Chat", tag = "UI.Layer.Chat" },
  { prop = "Stack_ActionGuide", tag = "UI.Layer.ActionGuide" },
  { prop = "Stack_Tooltip", tag = "UI.Layer.Tooltip" },
  { prop = "Stack_Timer", tag = "UI.Layer.Timer" },
}

-- Verified MappingName keys from DT_OP_KeyboardMouseButtonList.json for R4 input queries.
local DEFAULT_MAPPING_NAMES = {
  "UICommon_Decide",
  "UICommon_Cancel",
  "UICommon_PreviewPage",
  "UICommon_NextPage",
  "UICommon_MoveForwards",
  "UICommon_MoveBackwards",
  "UICommon_MoveLeft",
  "UICommon_MoveRight",
  "UICommon_SystemMenu",
  "MapUnit_MoveForward",
  "MapUnit_MoveBackwards",
  "MapUnit_MoveLeft",
  "MapUnit_MoveRight",
  "MapUnit_Interact",
  "Map_OpenMainMenu_Root",
  "Map_OpenMainMenu_Map",
  "Map_OpenMainMenu_Heal",
  "Map_OpenMainMenu_Journal",
  "Map_OpenMainMenu_Item",
  "Map_OpenMainMenu_Equip",
  "Map_OpenMainMenu_Party",
  "UIEvent_AutoPlay",
  "UIEvent_Skip",
  "UIBattle_Limit",
  "UIBattle_AoE",
  "UIBattle_DisplayLibra",
}

-- UTF-8 safe truncation to at most `limit` codepoints.
local function utf8Step(value, index)
  local byte = value:byte(index)
  if byte < 0x80 then return 1 end
  if byte < 0xE0 then return 2 end
  if byte < 0xF0 then return 3 end
  return 4
end

local function truncateText(value, limit)
  local maxLen = limit or TEXT_LIMIT
  if type(value) ~= "string" then return nil end
  if maxLen < 1 then return "" end
  local count = 0
  local index = 1
  local previousEnd = 0
  local totalBytes = #value
  while index <= totalBytes do
    count = count + 1
    local nextIndex = index + utf8Step(value, index)
    if count == maxLen then
      if nextIndex > totalBytes then return value end
      return value:sub(1, previousEnd) .. "…"
    end
    previousEnd = nextIndex - 1
    index = nextIndex
  end
  return value
end

local function recordErr(ctx, field, err)
  if ctx and type(ctx.err) == "function" then
    pcall(ctx.err, field, tostring(err))
  end
end

local function isFiniteNumber(value)
  return type(value) == "number" and value == value and value ~= math.huge and value ~= -math.huge
end

local function toInt(value)
  if not isFiniteNumber(value) then return nil end
  return math.floor(value)
end

-- Unwrap UE4SS RemoteUnrealParam wrappers returned inside ForEach callbacks.
local function unwrap(value)
  if value == nil then return nil end
  local kind = type(value)
  if kind == "table" or kind == "userdata" then
    local okGet, fn = pcall(function() return value.get end)
    if okGet and (type(fn) == "function" or type(fn) == "userdata") then
      local okVal, unwrapped = pcall(fn, value)
      if okVal then return unwrapped end
    end
  end
  return value
end

-- Safe UObject validity check. Returns (boolean, errOrNil).
local function isValidObject(obj)
  if obj == nil then return false, nil end
  local kind = type(obj)
  if kind ~= "table" and kind ~= "userdata" then return false, nil end
  local okFn, fn = pcall(function() return obj.IsValid end)
  if not okFn then return false, fn end
  if fn == nil then return false, nil end
  local okCall, valid = pcall(fn, obj)
  if not okCall then return false, valid end
  return valid == true, nil
end

-- Safe property read. Returns (ok, valueOrErr).
local function safeProp(obj, prop)
  if obj == nil then return false, nil end
  return pcall(function() return obj[prop] end)
end

-- Safe method call compatible with both Lua functions and UE4SS callable UFunction userdata.
-- Returns (calledOk, valueOrErr, methodExisted).
local function safeCall(obj, method, ...)
  if obj == nil then return false, nil, false end
  local okMember, fn = pcall(function() return obj[method] end)
  if not okMember then return false, fn, true end
  if fn == nil then return false, nil, false end
  local okCall, res = pcall(fn, obj, ...)
  if not okCall then return false, res, true end
  return true, res, true
end

-- Resolve a UObject or TWeakObjectPtr<UObject> to a valid UObject.
local function resolveObject(ptr)
  if ptr == nil then return nil end
  if isValidObject(ptr) then return ptr end
  local okGet, inner = safeCall(ptr, "Get")
  if okGet and isValidObject(inner) then return inner end
  return nil
end

-- Convert Lua string / FName / FString / FText to a Lua string.
local function toStr(value)
  if value == nil then return nil end
  if type(value) == "string" then return value end
  local ok, str = safeCall(value, "ToString")
  if ok and type(str) == "string" then return str end
  return nil
end

-- Read text from an FText/FString/FName or a UMG TextBlock/RichTextBlock widget, truncated to 200 chars.
local function readText(value, ctx, errField)
  if value == nil then return nil end
  if type(value) == "string" then return truncateText(value, TEXT_LIMIT) end
  -- Check if value is a UObject widget (e.g. CommonTextBlock, CPP_GradualTextBlock).
  if isValidObject(value) then
    local okCall, textVal, existed = safeCall(value, "GetText")
    if okCall and textVal ~= nil then
      local s = toStr(textVal)
      if s ~= nil then return truncateText(s, TEXT_LIMIT) end
    elseif existed and textVal ~= nil and ctx and errField then
      recordErr(ctx, errField, textVal)
      return nil
    end
    local okProp, propVal = safeProp(value, "Text")
    if okProp and propVal ~= nil then
      local s = toStr(propVal)
      if s ~= nil then return truncateText(s, TEXT_LIMIT) end
    elseif not okProp and ctx and errField then
      recordErr(ctx, errField, propVal)
      return nil
    end
    return nil
  end
  local s = toStr(value)
  if s ~= nil then return truncateText(s, TEXT_LIMIT) end
  return nil
end

-- Read a GameplayTag struct or string into its tag name; "None" and "" become nil.
local function readTagName(tag)
  if tag == nil then return nil end
  local s = toStr(tag)
  if s == nil then
    local ok, rawName = safeProp(tag, "TagName")
    if ok then s = toStr(rawName) end
  end
  if not s or s == "" or s == "None" then return nil end
  return s
end

-- Extract the short class name from a UObject (e.g. "WBP_SystemMenuLayout_C").
local function shortClassName(obj)
  if not isValidObject(obj) then return nil end
  local okCls, cls = safeCall(obj, "GetClass")
  if not okCls or not isValidObject(cls) then return nil end
  local okFull, full = safeCall(cls, "GetFullName")
  if okFull and type(full) == "string" and full ~= "" then
    local short = full:match("([^.%s]+)$")
    if short and short ~= "" then return short end
  end
  local okFn, fname = safeCall(cls, "GetFName")
  if okFn and fname ~= nil then
    local s = toStr(fname)
    if s and s ~= "" then return s end
  end
  return nil
end

-- Extract the short object name (last segment after "." or " ").
local function shortObjectName(obj)
  if not isValidObject(obj) then return nil end
  local okFn, fname = safeCall(obj, "GetFName")
  if okFn and fname ~= nil then
    local s = toStr(fname)
    if s and s ~= "" then return s end
  end
  local okFull, full = safeCall(obj, "GetFullName")
  if okFull and type(full) == "string" and full ~= "" then
    local short = full:match("([^.%s]+)$")
    if short and short ~= "" then return short end
  end
  return nil
end

-- Compare two UObject references across potentially distinct UE4SS userdata wrappers.
local function sameObject(left, right)
  if left == nil or right == nil then return false end
  if left == right then return true end
  local okAddrL, addrL = safeCall(left, "GetAddress")
  local okAddrR, addrR = safeCall(right, "GetAddress")
  if okAddrL and okAddrR and addrL ~= nil and addrR ~= nil then
    return addrL == addrR
  end
  local okNameL, nameL = safeCall(left, "GetFullName")
  local okNameR, nameR = safeCall(right, "GetFullName")
  if okNameL and okNameR and type(nameL) == "string" and nameL ~= "" then
    return nameL == nameR
  end
  return false
end

-- Map an enum integer or string ("Enum::Value") to its short name.
local function readEnumName(value, lookup)
  if value == nil then return nil end
  if type(value) == "number" and isFiniteNumber(value) then
    local idx = math.floor(value)
    if lookup and lookup[idx] ~= nil then return lookup[idx] end
    return tostring(idx)
  end
  local s = toStr(value)
  if s and s ~= "" then
    return s:match("::([^:]+)$") or s
  end
  return nil
end

-- Read a UE4SS TArray or Lua sequence into a Lua list (up to `limit` items).
local function readArray(arr, limit)
  local out = {}
  if arr == nil then return out end
  local maxItems = limit or MAX_ARRAY_ITEMS
  local length
  local okNum, n = safeCall(arr, "GetArrayNum")
  if okNum and isFiniteNumber(n) then
    length = math.floor(n)
  else
    local okNum2, n2 = safeCall(arr, "Num")
    if okNum2 and isFiniteNumber(n2) then
      length = math.floor(n2)
    else
      local okLen, n3 = pcall(function() return #arr end)
      if okLen and isFiniteNumber(n3) then length = math.floor(n3) end
    end
  end
  if length and length >= 0 then
    local count = math.min(length, maxItems)
    for index = 1, count do
      local okItem, item = pcall(function() return arr[index] end)
      if okItem then
        out[#out + 1] = unwrap(item)
      end
    end
    return out
  end
  -- Fallback to ForEach if length is unavailable.
  local okFe, feFn = safeProp(arr, "ForEach")
  if okFe and (type(feFn) == "function" or type(feFn) == "userdata") then
    pcall(feFn, arr, function(first, second)
      if #out >= maxItems then return true end
      local item = second ~= nil and second or first
      out[#out + 1] = unwrap(item)
    end)
  end
  return out
end

-- Read a UE4SS TSet<FString/FName> or Lua array into a Lua string list.
local function readStringSet(setObj, limit)
  local out = {}
  if setObj == nil then return out, true end
  local maxItems = limit or MAX_ARRAY_ITEMS
  local okFe, feFn = safeProp(setObj, "ForEach")
  if okFe and (type(feFn) == "function" or type(feFn) == "userdata") then
    local okRun, runErr = pcall(feFn, setObj, function(first, second)
      if #out >= maxItems then return true end
      -- Handle both TSet ForEach(elem) and 2-arg ForEach(index, elem).
      local raw = (second ~= nil and type(first) == "number") and second or first
      local s = toStr(unwrap(raw))
      if s and s ~= "" then out[#out + 1] = truncateText(s, TEXT_LIMIT) end
    end)
    if not okRun then return out, false, runErr end
    return out, true
  end
  if type(setObj) == "table" or type(setObj) == "userdata" then
    local items = readArray(setObj, maxItems)
    for _, item in ipairs(items) do
      local s = toStr(item)
      if s and s ~= "" then out[#out + 1] = truncateText(s, TEXT_LIMIT) end
    end
    return out, true
  end
  return out, false, "TokenSet 类型不受支持"
end

-- Iterate over a UE4SS TMap or Lua table, invoking visitor(key, value).
local function forEachMap(mapObj, visitor, limit)
  if mapObj == nil then return false end
  local maxItems = limit or MAX_ARRAY_ITEMS
  local count = 0
  local okFe, feFn = safeProp(mapObj, "ForEach")
  if okFe and (type(feFn) == "function" or type(feFn) == "userdata") then
    local okRun = pcall(feFn, mapObj, function(key, val)
      count = count + 1
      if count > maxItems then return true end
      if visitor(unwrap(key), unwrap(val)) == true then return true end
    end)
    return okRun
  end
  if type(mapObj) == "table" then
    for key, val in pairs(mapObj) do
      count = count + 1
      if count > maxItems then break end
      if visitor(key, val) == true then break end
    end
    return true
  end
  return false
end

-- Check ESlateVisibility: 0=Visible, 1=Collapsed, 2=Hidden, 3=HitTestInvisible, 4=SelfHitTestInvisible.
local function readVisibility(widget)
  if not isValidObject(widget) then return nil end
  local okCall, vis = safeCall(widget, "GetVisibility")
  if okCall and vis ~= nil then return vis end
  local okProp, visProp = safeProp(widget, "Visibility")
  if okProp and visProp ~= nil then return visProp end
  return nil
end

local function isVisibilityVisible(vis)
  if vis == nil then return false end
  if type(vis) == "number" then return vis == 0 end
  local s = toStr(vis)
  if s then
    local short = s:match("::([^:]+)$") or s
    return short == "Visible"
  end
  return false
end

local function isVisibilityShown(vis)
  if vis == nil then return false end
  if type(vis) == "number" then return vis ~= 1 and vis ~= 2 end
  local s = toStr(vis)
  if s then
    local short = s:match("::([^:]+)$") or s
    return short ~= "Collapsed" and short ~= "Hidden"
  end
  return false
end

-- CommonUI pooling guard: verify widget is valid, activated, and not in open/close transition.
local function isWidgetActive(widget)
  if not isValidObject(widget) then return false end
  local okAct, act, actExisted = safeCall(widget, "IsActivated")
  if actExisted then
    if okAct and act == false then return false end
  else
    local okFlag, flag = safeProp(widget, "bIsActive")
    if okFlag and flag == false then return false end
  end
  local okOpen, opening = safeProp(widget, "IsOpeningProcess")
  if okOpen and opening == true then return false end
  local okClose, closing = safeProp(widget, "IsClosingProcess")
  if okClose and closing == true then return false end
  return true
end

-- Cache management: GameInstance-scoped singletons persist across maps (re-validated via IsValid);
-- World-scoped singletons are invalidated whenever World changes.
local function ensureCache(ctx)
  local cache = (ctx and type(ctx.cache) == "table") and ctx.cache or {}
  if type(cache.gi) ~= "table" then cache.gi = {} end
  if type(cache.world) ~= "table" then cache.world = {} end
  return cache
end

local function syncWorldCache(ctx, cache)
  if not ctx or type(ctx.find) ~= "function" then return end
  local okWorld, world = pcall(ctx.find, "World")
  if not okWorld then return end
  local worldName = nil
  if isValidObject(world) then
    local okName, name = safeCall(world, "GetFullName")
    if okName and type(name) == "string" and name ~= "" then
      worldName = name
    end
  end
  if worldName ~= nil and cache.worldName ~= nil and worldName ~= cache.worldName then
    cache.world = {}
  elseif world == nil and cache.worldName ~= nil then
    cache.world = {}
  end
  cache.worldName = worldName
end

local function getCachedSingleton(ctx, bucket, className)
  local cached = bucket[className]
  if cached ~= nil then
    if isValidObject(cached) then return cached, nil end
    bucket[className] = nil
  end
  if not ctx or type(ctx.find) ~= "function" then return nil, nil end
  local ok, obj = pcall(ctx.find, className)
  if not ok then return nil, obj end
  if isValidObject(obj) then
    bucket[className] = obj
    return obj, nil
  end
  return nil, nil
end

-- Resolve active layer tag from WBP_OverAllLayout_C.
local function resolveLayerTag(layout, layer)
  if not isValidObject(layer) then return nil end
  -- 1. Match against layout.Layers TMap<GameplayTag, Container>.
  local okLayers, layersMap = safeProp(layout, "Layers")
  if not okLayers or layersMap == nil then
    local okCall, mapRes = safeCall(layout, "GetAllLayerMap")
    if okCall then layersMap = mapRes end
  end
  if layersMap ~= nil then
    local matchedTag = nil
    forEachMap(layersMap, function(tagKey, stackVal)
      if sameObject(stackVal, layer) then
        matchedTag = readTagName(tagKey)
        return true
      end
    end, 32)
    if matchedTag then return matchedTag end
  end
  -- 2. Fallback: compare against known Stack_* slot properties on WBP_OverAllLayout_C.
  for _, slot in ipairs(LAYOUT_STACK_SLOTS) do
    local okSlot, stackObj = safeProp(layout, slot.prop)
    if okSlot and sameObject(stackObj, layer) then
      return slot.tag
    end
  end
  -- 3. Fallback: object short name (e.g. "Stack_System").
  return shortObjectName(layer)
end

-- Get active widget from a CommonActivatableWidgetContainerBase stack.
local function getStackActiveWidget(stack, ctx, errField)
  if not isValidObject(stack) then return nil end
  local okCall, widget, existed = safeCall(stack, "GetActiveWidget")
  if okCall and isValidObject(widget) then
    return isWidgetActive(widget) and widget or nil
  elseif existed and not okCall and widget ~= nil and ctx and errField then
    recordErr(ctx, errField, widget)
  end
  local okProp, displayed = safeProp(stack, "DisplayedWidget")
  if okProp and isValidObject(displayed) then
    return isWidgetActive(displayed) and displayed or nil
  elseif not okProp and ctx and errField then
    recordErr(ctx, errField, displayed)
  end
  return nil
end

-- Find a specific layer stack on WBP_OverAllLayout_C by Stack_* property or Layers map tag.
local function findLayerStack(layout, propName, layerTag)
  if not isValidObject(layout) then return nil end
  local okProp, stack = safeProp(layout, propName)
  if okProp and isValidObject(stack) then return stack end
  local okLayers, layersMap = safeProp(layout, "Layers")
  if okLayers and layersMap ~= nil then
    local found = nil
    forEachMap(layersMap, function(tagKey, stackVal)
      if readTagName(tagKey) == layerTag and isValidObject(stackVal) then
        found = stackVal
        return true
      end
    end, 32)
    if found then return found end
  end
  return nil
end

-- Read focused item index from a CPP_WBP_MenuBase / CPP_BI_MenuBase widget.
local function getMenuFocusIndex(menu)
  if not isValidObject(menu) then return nil end
  -- 1. Prefer CPP_BI_MenuBase:GetFocusedItemIndex(0 = Current).
  local okCall, idx = safeCall(menu, "GetFocusedItemIndex", 0)
  if okCall and isFiniteNumber(idx) then
    local value = math.floor(idx)
    if value >= 0 then return value end
  end
  -- 2. Fallback: read FocuedItemDataMap[0].Index (note game typo: Focued).
  local okMap, focusMap = safeProp(menu, "FocuedItemDataMap")
  if okMap and focusMap ~= nil then
    local foundIdx = nil
    forEachMap(focusMap, function(key, val)
      local keyIsCurrent = (key == 0)
        or (toStr(key) == "Current")
        or (toStr(key) == "EEnhancedMenu_FocuedTimeType::Current")
      if keyIsCurrent and val ~= nil then
        local okIdx, rawIdx = safeProp(val, "Index")
        if okIdx and isFiniteNumber(rawIdx) and rawIdx >= 0 then
          foundIdx = math.floor(rawIdx)
          return true
        end
      end
    end, 16)
    if foundIdx ~= nil then return foundIdx end
  end
  return nil
end

-- Traverse EnhancedUI CPP_EU_WBP_MenuPath wrappers (PathMenu / GetPathMenu / WBP_PathMenu)
-- returning the chain of valid widgets (up to depth 4).
local function collectMenuPathChain(root)
  local chain = {}
  local current = root
  for _ = 1, 5 do
    if not isValidObject(current) then break end
    chain[#chain + 1] = current
    local nextObj = nil
    local okPm, pm = safeProp(current, "PathMenu")
    if okPm and isValidObject(pm) then
      nextObj = pm
    else
      local okCall, pm2 = safeCall(current, "GetPathMenu")
      if okCall and isValidObject(pm2) then
        nextObj = pm2
      else
        local okWpm, wpm = safeProp(current, "WBP_PathMenu")
        if okWpm and isValidObject(wpm) then nextObj = wpm end
      end
    end
    if nextObj == nil or sameObject(nextObj, current) then break end
    current = nextObj
  end
  return chain
end

-- Resolve the WBP_OptionList_C inside a WBP_SHOP_CommandWindow_C.
local function getShopOptionList(cmdWindow)
  if not isValidObject(cmdWindow) then return nil end
  -- Direct shortcut if test/caller passes WBP_OptionList_C directly.
  local okDirect, directIdx = safeProp(cmdWindow, "FocuedIndex")
  if okDirect and isFiniteNumber(directIdx) then return cmdWindow end
  local okLists, commonLists = safeProp(cmdWindow, "WBP_SHOP_CommonLists")
  if not okLists or not isValidObject(commonLists) then return nil end
  local okMain, mainList = safeProp(commonLists, "WBP_SHOP_MainList")
  if not okMain or not isValidObject(mainList) then return nil end
  local okOpt, optList = safeProp(mainList, "WBP_CommandList")
  if okOpt and isValidObject(optList) then return optList end
  return nil
end

-- Read input_blocked token array from CPP_GameWidgetInputManager (fallback CPP_BFL_UI_Input:IsBlocked()).
local function collectInputBlocked(ctx, cache, gwMgr)
  local inputMgr = nil
  if isValidObject(gwMgr) then
    local okProp, mgr = safeProp(gwMgr, "m_GameWidgetInputManager")
    if not okProp then
      recordErr(ctx, "ui.input_blocked", mgr)
    elseif isValidObject(mgr) then
      inputMgr = mgr
    end
  end
  if not isValidObject(inputMgr) then
    local found, findErr = getCachedSingleton(ctx, cache.gi, "CPP_GameWidgetInputManager")
    if findErr then recordErr(ctx, "ui.input_blocked", findErr) end
    inputMgr = found
  end
  if isValidObject(inputMgr) then
    local okBlockSet, blockSet = safeProp(inputMgr, "InputBlockSet")
    if okBlockSet and blockSet ~= nil then
      local okTokenSet, tokenSet = safeProp(blockSet, "TokenSet")
      if okTokenSet and tokenSet ~= nil then
        local tokens, readOk, readErr = readStringSet(tokenSet, MAX_ARRAY_ITEMS)
        if readOk then
          return json.array(tokens)
        end
        recordErr(ctx, "ui.input_blocked", readErr)
      elseif not okTokenSet then
        recordErr(ctx, "ui.input_blocked", tokenSet)
      end
    elseif not okBlockSet then
      recordErr(ctx, "ui.input_blocked", blockSet)
    end
  end
  -- Fallback: CPP_BFL_UI_Input:IsBlocked().
  local bfl, bflErr = getCachedSingleton(ctx, cache.gi, "CPP_BFL_UI_Input")
  if bflErr then recordErr(ctx, "ui.input_blocked", bflErr) end
  if isValidObject(bfl) then
    local okBlocked, blocked, existed = safeCall(bfl, "IsBlocked")
    if okBlocked and type(blocked) == "boolean" then
      return json.array(blocked and { "Blocked" } or {})
    elseif existed and not okBlocked and blocked ~= nil then
      recordErr(ctx, "ui.input_blocked", blocked)
    end
  elseif not isValidObject(inputMgr) then
    recordErr(ctx, "ui.input_blocked", "找不到有效的 CPP_GameWidgetInputManager")
  end
  return json.array({})
end

-- Substructure #6: ui.movie
local function collectMovie(ctx, movieSub, talkWidget)
  if not isValidObject(movieSub) then return nil end
  local okScreen, screen = safeProp(movieSub, "m_pMovieScreen")
  if not okScreen then
    recordErr(ctx, "ui.movie", screen)
    return nil
  end
  if not isValidObject(screen) then return nil end

  local subtitle = json.null
  if isValidObject(talkWidget) then
    local okWm, wm = safeProp(talkWidget, "WindowManager")
    if okWm and isValidObject(wm) then
      local okWin, win = safeProp(wm, "m_pWindow")
      if okWin and isValidObject(win) then
        local winClass = shortClassName(win)
        if winClass == "WBP_TalkEvent_Caption_C" then
          local okTb, tb = safeProp(win, "m_pTextBlock")
          local text = (okTb and tb ~= nil) and readText(tb, ctx, "ui.movie.subtitle") or nil
          if (not text or text == "") then
            local okData, curData = safeProp(talkWidget, "m_pCurrentTalkData")
            if okData and isValidObject(curData) then
              local okGt, rawText = safeCall(curData, "GetText")
              if okGt then text = readText(rawText) end
            end
          end
          if text and text ~= "" then subtitle = text end
        end
      end
    end
  end

  return {
    active = true,
    subtitle = subtitle,
  }
end

-- Substructure #9: ui.tutorial
local function collectTutorial(ctx, tutSub)
  if not isValidObject(tutSub) then return nil end
  local okW, tutLayout = safeProp(tutSub, "m_TutorialWidget")
  if not okW then
    recordErr(ctx, "ui.tutorial", tutLayout)
    return nil
  end
  if not isValidObject(tutLayout) then return nil end

  local tut = tutLayout
  local okInner, inner = safeProp(tutLayout, "WBP_Tutorial")
  if not okInner then
    recordErr(ctx, "ui.tutorial", inner)
  elseif isValidObject(inner) then
    tut = inner
  end

  local pageIndex = 0
  local okPi, pi = safeProp(tut, "PageIndex")
  if not okPi then
    recordErr(ctx, "ui.tutorial.page_index", pi)
  elseif isFiniteNumber(pi) then
    pageIndex = math.floor(pi)
  end

  local name = ""
  local caption = ""
  local text = ""
  local pageCount = 0

  local okData, data = safeProp(tut, "Data")
  if not okData then
    recordErr(ctx, "ui.tutorial.data", data)
  elseif data ~= nil then
    local okName, rawName = safeProp(data, "Name")
    if okName and rawName ~= nil then
      name = readText(rawName) or ""
    end
    local okPages, rawPages = safeProp(data, "Pages")
    if okPages and rawPages ~= nil then
      local pages = readArray(rawPages, MAX_ARRAY_ITEMS)
      pageCount = #pages
      local page = (pageIndex >= 0 and pageIndex + 1 <= pageCount) and pages[pageIndex + 1] or nil
      if page ~= nil then
        local okCap, rawCap = safeProp(page, "Caption")
        if okCap and rawCap ~= nil then caption = readText(rawCap) or "" end
        local okTxt, rawTxt = safeProp(page, "Text")
        if okTxt and rawTxt ~= nil then text = readText(rawTxt) or "" end
      end
    end
  end

  if name == "" then
    local okTitle, titleW = safeProp(tut, "Text_Title")
    if okTitle and titleW ~= nil then name = readText(titleW) or "" end
  end
  if text == "" then
    local okDesc, descW = safeProp(tut, "Text_Description")
    if okDesc and descW ~= nil then text = readText(descW) or "" end
  end

  local canClose = (pageCount == 0) or (pageIndex + 1 >= pageCount)
  return {
    name = name,
    page_index = pageIndex,
    page_count = pageCount,
    caption = caption,
    text = text,
    can_close = canClose,
  }
end

-- Substructure #8: ui.choice (system dialog or event choice)
local function collectChoice(ctx, cache, gwMgr, talkWidget)
  local source = nil
  local root = nil

  -- 1. System / item confirmation dialog: CPP_DialogManager.DialogWidget
  local dlgMgr = nil
  if isValidObject(gwMgr) then
    local okDm, dm = safeProp(gwMgr, "m_DialogManager")
    if not okDm then
      recordErr(ctx, "ui.choice", dm)
    elseif isValidObject(dm) then
      dlgMgr = dm
    end
  end
  if not isValidObject(dlgMgr) then
    dlgMgr = getCachedSingleton(ctx, cache.gi, "CPP_DialogManager")
  end
  if isValidObject(dlgMgr) then
    local okDw, dw = safeProp(dlgMgr, "DialogWidget")
    if not okDw then
      recordErr(ctx, "ui.choice", dw)
    elseif isValidObject(dw) then
      source = "dialog"
      root = dw
    end
  end

  -- 2. Event branch choice: WBP_TalkEventLayout_C.WidgetStack:GetActiveWidget() matching *ChoiceDialog*
  if root == nil and isValidObject(talkWidget) then
    local okWs, ws = safeProp(talkWidget, "WidgetStack")
    if not okWs then
      recordErr(ctx, "ui.choice", ws)
    elseif isValidObject(ws) then
      local activeDlg = getStackActiveWidget(ws, ctx, "ui.choice")
      if isValidObject(activeDlg) then
        local clsName = shortClassName(activeDlg)
        if clsName and clsName:find("ChoiceDialog", 1, true) ~= nil then
          source = "event_choice"
          root = activeDlg
        end
      end
    end
  end

  if root == nil then return nil end

  -- Read DialogBoxInfo from root or child box (WBP_DialogBox / WBP_PathMenu).
  local box = nil
  local okDb, db = safeProp(root, "WBP_DialogBox")
  if okDb and isValidObject(db) then
    box = db
  else
    local okPm, pm = safeProp(root, "WBP_PathMenu")
    if okPm and isValidObject(pm) then box = pm end
  end

  local dialogTag = nil
  local choiceTag = nil
  local message = ""

  local function absorbDialogBoxInfo(owner)
    if not isValidObject(owner) then return end
    local okInfo, info = safeProp(owner, "DialogBoxInfo")
    if not okInfo or info == nil then return end
    if not dialogTag then
      local okDt, dt = safeProp(info, "DialogTag")
      if okDt then dialogTag = readTagName(dt) end
    end
    if not choiceTag then
      local okCt, ct = safeProp(info, "ChoiceTag")
      if okCt then choiceTag = readTagName(ct) end
    end
    if message == "" then
      local okMsg, rawMsg = safeProp(info, "Message")
      if okMsg and rawMsg ~= nil then message = readText(rawMsg) or "" end
    end
  end

  absorbDialogBoxInfo(root)
  absorbDialogBoxInfo(box)

  if not dialogTag then
    local okDt, dt = safeProp(root, "DialogTag")
    if okDt then dialogTag = readTagName(dt) end
  end
  if not choiceTag and isValidObject(talkWidget) then
    local okCt, ct = safeProp(talkWidget, "m_DialogChoiceTag")
    if okCt then choiceTag = readTagName(ct) end
  end
  if message == "" and isValidObject(box) then
    local okTm, tm = safeProp(box, "Text_Message")
    if okTm and tm ~= nil then message = readText(tm) or "" end
  end

  -- Traverse PathMenu chain to locate m_ChoiceData, CPP_WBP_MenuBase focus, and DirectionGridPanel entries.
  local chain = collectMenuPathChain(box or root)
  if box ~= nil then
    -- Also include root itself in case a mock puts m_ChoiceData / GetFocusedItemIndex directly on root.
    chain[#chain + 1] = root
  end

  local options = {}
  local focusIndex = nil
  local entryWidgets = nil

  for _, node in ipairs(chain) do
    -- Try m_ChoiceData / GetChoiceData() for option text list.
    if #options == 0 then
      local choiceData = nil
      local okCd, cd = safeProp(node, "m_ChoiceData")
      if okCd and cd ~= nil then
        choiceData = cd
      else
        local okCall, cd2 = safeCall(node, "GetChoiceData")
        if okCall and cd2 ~= nil then choiceData = cd2 end
      end
      if choiceData ~= nil then
        if not choiceTag then
          local okCt, ct = safeProp(choiceData, "ChoiceTag")
          if okCt then choiceTag = readTagName(ct) end
        end
        local okList, list = safeProp(choiceData, "ChoiceList")
        if okList and list ~= nil then
          for _, item in ipairs(readArray(list, MAX_ARRAY_ITEMS)) do
            local s = readText(item)
            if s ~= nil then options[#options + 1] = s end
          end
        end
      end
    end

    -- Try CPP_BI_MenuBase:GetFocusedItemIndex(0) or FocuedItemDataMap.
    if focusIndex == nil then
      focusIndex = getMenuFocusIndex(node)
    end

    -- Locate DirectionGridPanel.EntryWidgetList for fallback option texts and Image_Select focus.
    if entryWidgets == nil then
      local grid = nil
      local okGp, gp = safeProp(node, "DirectionGridPanel")
      if okGp and isValidObject(gp) then
        grid = gp
      else
        local okCall, gp2 = safeCall(node, "GetDirectionGridPanel")
        if okCall and isValidObject(gp2) then grid = gp2 end
      end
      if isValidObject(grid) then
        local okEntries, rawEntries = safeProp(grid, "EntryWidgetList")
        if okEntries and rawEntries ~= nil then
          entryWidgets = readArray(rawEntries, MAX_ARRAY_ITEMS)
        end
      end
    end
  end

  if entryWidgets ~= nil and #entryWidgets > 0 then
    if #options == 0 then
      for _, entry in ipairs(entryWidgets) do
        if isValidObject(entry) then
          local okTxt, txtWidget = safeProp(entry, "Text")
          local label = (okTxt and txtWidget ~= nil) and readText(txtWidget) or ""
          options[#options + 1] = label
        end
      end
    end
    if focusIndex == nil then
      for index, entry in ipairs(entryWidgets) do
        if isValidObject(entry) then
          local okImg, imgSel = safeProp(entry, "Image_Select")
          if okImg and isValidObject(imgSel) then
            local vis = readVisibility(imgSel)
            if isVisibilityShown(vis) then
              focusIndex = index - 1
              break
            end
          end
        end
      end
    end
  end

  return {
    source = source,
    dialog_tag = dialogTag or json.null,
    choice_tag = choiceTag or json.null,
    message = message,
    options = json.array(options),
    focus_index = focusIndex ~= nil and focusIndex or json.null,
  }
end

-- Substructure #7: ui.talk (dialogue window or non-dialogue cutscene)
local function collectTalk(ctx, cache, evMgr, talkWidget)
  if not isValidObject(evMgr) then return nil end

  -- Current event ID via CPP_BFL_TalkEvent:GetCurrentEventId().
  local eventId = json.null
  local bflTalk, bflErr = getCachedSingleton(ctx, cache.gi, "CPP_BFL_TalkEvent")
  if bflErr then
    recordErr(ctx, "ui.talk.event_id", bflErr)
  elseif isValidObject(bflTalk) then
    local okId, rawId, existed = safeCall(bflTalk, "GetCurrentEventId")
    if okId and rawId ~= nil then
      local s = toStr(rawId)
      if s and s ~= "" and s ~= "None" then eventId = truncateText(s, TEXT_LIMIT) end
    elseif existed and not okId and rawId ~= nil then
      recordErr(ctx, "ui.talk.event_id", rawId)
    end
  end

  local win = nil
  if isValidObject(talkWidget) then
    local okWm, wm = safeProp(talkWidget, "WindowManager")
    if not okWm then
      recordErr(ctx, "ui.talk.window", wm)
    elseif isValidObject(wm) then
      local okWin, rawWin = safeProp(wm, "m_pWindow")
      if not okWin then
        recordErr(ctx, "ui.talk.window", rawWin)
      elseif isValidObject(rawWin) then
        win = rawWin
      end
    end
  end

  local windowClass = win and shortClassName(win) or nil
  local isCaption = type(windowClass) == "string" and windowClass:find("Caption", 1, true) ~= nil
  local isCutscene = (win == nil) or isCaption

  local speaker = nil
  local text = nil
  local waitingKey = false
  local textFinished = json.null

  if isValidObject(win) then
    if not isCaption then
      local okName, nameWidget = safeProp(win, "Text_Name")
      if not okName then
        recordErr(ctx, "ui.talk.speaker", nameWidget)
      elseif nameWidget ~= nil then
        speaker = readText(nameWidget, ctx, "ui.talk.speaker")
      end
    end

    local textBlock = nil
    local okTb, tb = safeProp(win, "m_pTextBlock")
    if not okTb then
      recordErr(ctx, "ui.talk.text", tb)
    elseif isValidObject(tb) then
      textBlock = tb
    else
      local okCall, tb2 = safeCall(win, "GetTextBlock")
      if okCall and isValidObject(tb2) then textBlock = tb2 end
    end

    if isValidObject(textBlock) then
      text = readText(textBlock, ctx, "ui.talk.text")
      local okFin, fin, finExisted = safeCall(textBlock, "IsTextFinished")
      if okFin and type(fin) == "boolean" then
        textFinished = fin
      elseif finExisted and not okFin and fin ~= nil then
        recordErr(ctx, "ui.talk.text_finished", fin)
      end
    end

    -- Fallback to m_pCurrentTalkData when window is active but text/speaker wasn't read from widgets.
    if isValidObject(talkWidget) and ((not isCaption and (not speaker or speaker == "")) or (not text or text == "")) then
      local okData, curData = safeProp(talkWidget, "m_pCurrentTalkData")
      if okData and isValidObject(curData) then
        if not isCaption and (not speaker or speaker == "") then
          local okSpk, rawSpk = safeCall(curData, "GetSpeaker")
          if okSpk and rawSpk ~= nil then
            local s = toStr(rawSpk)
            if s and s ~= "" and s ~= "None" then speaker = truncateText(s, TEXT_LIMIT) end
          end
        end
        if not text or text == "" then
          local okTxt, rawTxt = safeCall(curData, "GetText")
          if okTxt and rawTxt ~= nil then text = readText(rawTxt) end
        end
      end
    end

    -- Waiting for key press: WBP_TalkKeyWait visibility == Visible (0); captions auto-advance.
    if not isCaption then
      local okWait, waitWidget = safeProp(win, "WBP_TalkKeyWait")
      if not okWait then
        recordErr(ctx, "ui.talk.waiting_key", waitWidget)
      elseif isValidObject(waitWidget) then
        waitingKey = isVisibilityVisible(readVisibility(waitWidget))
      end
    end
  end

  local canSkip = false
  local isAuto = false
  if isValidObject(talkWidget) then
    local okSkip, skipVal, skipExisted = safeCall(talkWidget, "CanSkip")
    if okSkip and type(skipVal) == "boolean" then
      canSkip = skipVal
    elseif not skipExisted then
      local okProp, propSkip = safeProp(talkWidget, "CanSkip")
      if okProp and type(propSkip) == "boolean" then canSkip = propSkip end
    end

    local okAuto, autoVal, autoExisted = safeCall(talkWidget, "IsAutoPlay")
    if okAuto and type(autoVal) == "boolean" then
      isAuto = autoVal
    elseif not autoExisted then
      local okProp, propAuto = safeProp(talkWidget, "IsAutoPlay")
      if okProp and type(propAuto) == "boolean" then isAuto = propAuto end
    end
  end

  if speaker == "" then speaker = nil end
  if text == "" then text = nil end

  return {
    event_id = eventId,
    window_class = windowClass or json.null,
    is_cutscene = isCutscene,
    speaker = speaker or json.null,
    text = text or json.null,
    waiting_key = waitingKey,
    text_finished = textFinished,
    can_skip = canSkip,
    is_auto = isAuto,
  }
end

-- Substructure #12: ui.save_ui
local function collectSaveUi(ctx, layout)
  local stackFront1 = findLayerStack(layout, "Stack_System_Front1", "UI.Layer.System.Front1")
  local active = getStackActiveWidget(stackFront1, ctx, "ui.save_ui")
  if not isValidObject(active) then return nil end

  local clsName = shortClassName(active)
  if clsName ~= "WBP_SaveLayout_C" and clsName ~= "WBP_LoadLayout_C" then
    return nil
  end

  local saveDataList = nil
  local okPm, pm = safeProp(active, "PathMenu")
  if not okPm then
    recordErr(ctx, "ui.save_ui", pm)
  elseif isValidObject(pm) then
    saveDataList = pm
  else
    local okCall, pm2 = safeCall(active, "GetPathMenu")
    if okCall and isValidObject(pm2) then saveDataList = pm2 end
  end

  local mode = (clsName == "WBP_LoadLayout_C") and "load" or "save"
  local slotIndices = {}
  local focusIndex = nil

  if isValidObject(saveDataList) then
    local okIsSave, isSave = safeProp(saveDataList, "IsSaveLayout")
    if okIsSave and type(isSave) == "boolean" then
      mode = isSave and "save" or "load"
    end
    local okSlots, rawSlots = safeProp(saveDataList, "SlotIndexList")
    if not okSlots then
      recordErr(ctx, "ui.save_ui.slot_indices", rawSlots)
    elseif rawSlots ~= nil then
      for _, slot in ipairs(readArray(rawSlots, MAX_ARRAY_ITEMS)) do
        local idx = toInt(slot)
        if idx ~= nil then slotIndices[#slotIndices + 1] = idx end
      end
    end

    for _, node in ipairs(collectMenuPathChain(saveDataList)) do
      focusIndex = getMenuFocusIndex(node)
      if focusIndex ~= nil then break end
    end
  end

  return {
    mode = mode,
    slot_indices = json.array(slotIndices),
    focus_index = focusIndex ~= nil and focusIndex or json.null,
  }
end

-- Substructure #11: ui.shop
local function collectShop(ctx, cache, systemActive)
  if not isValidObject(systemActive) or shortClassName(systemActive) ~= "WBP_SHOP_Layout_C" then
    return nil
  end

  -- Active gate: CPP_VM_Colosseum_Shop must be valid (unbound immediately on shop exit).
  local vmColShop = nil
  local okVm, vm = safeProp(systemActive, "CPP_VM_Colosseum_Shop")
  if not okVm then
    recordErr(ctx, "ui.shop", vm)
  elseif isValidObject(vm) then
    vmColShop = vm
  else
    local okVm2, vm2 = safeProp(systemActive, "VM_ColosseumShop")
    if okVm2 and isValidObject(vm2) then vmColShop = vm2 end
  end
  if not isValidObject(vmColShop) then return nil end

  -- Blueprint trade window VM (avoid C++ default CPP_VM_TradeWindow which has ShopID = -1).
  local vmTrade = nil
  local okVt, vt = safeProp(systemActive, "VM_Shop_TradeWindow")
  if not okVt then
    recordErr(ctx, "ui.shop.trade_vm", vt)
  elseif isValidObject(vt) then
    vmTrade = vt
  end

  -- Active trade window inside WBP_SHOP_Layout_C.WidgetStack.
  local tradeWin = nil
  local okWs, ws = safeProp(systemActive, "WidgetStack")
  if okWs and isValidObject(ws) then
    local top = getStackActiveWidget(ws, ctx, "ui.shop.trade_window")
    if isValidObject(top) then
      tradeWin = top
      if not isValidObject(vmTrade) then
        local okWvt, wvt = safeProp(tradeWin, "VM_Shop_TradeWindow")
        if okWvt and isValidObject(wvt) then vmTrade = wvt end
      end
    end
  end
  if not isValidObject(vmTrade) then
    vmTrade = getCachedSingleton(ctx, cache.gi, "VM_Shop_TradeWindow_C")
  end

  -- Shop type and outer scene stack from CPP_VM_Colosseum_Shop.
  local shopType = nil
  local okSt, rawSt = safeProp(vmColShop, "ShopType")
  if okSt then shopType = readEnumName(rawSt, SHOP_TYPE_ENUM) end

  local sceneStack = {}
  local okSs, rawSs = safeProp(vmColShop, "SceneTypeStack")
  if okSs and rawSs ~= nil then
    for _, scene in ipairs(readArray(rawSs, 16)) do
      local name = readEnumName(scene, SHOP_SCENE_ENUM)
      if name then sceneStack[#sceneStack + 1] = name end
    end
  end

  -- Trade VM fields: shop_id, is_buy (never read CPP_VM_Colosseum_Shop.IsBuy!), total_amount, items.
  local shopId = nil
  local isBuy = nil
  local totalAmount = nil
  local items = {}

  if isValidObject(vmTrade) then
    local okId, rawId = safeProp(vmTrade, "ShopID")
    if okId then
      local idVal = toInt(rawId)
      if idVal ~= nil and idVal >= 0 then shopId = idVal end
    end

    local okBuy, rawBuy = safeProp(vmTrade, "IsBuy")
    if not okBuy then
      recordErr(ctx, "ui.shop.is_buy", rawBuy)
    elseif type(rawBuy) == "boolean" then
      isBuy = rawBuy
    end

    local okTot, rawTot = safeProp(vmTrade, "CurrentTotalAmount")
    if okTot then totalAmount = toInt(rawTot) end

    local infoById = {}
    local infoList = {}
    local okInfos, rawInfos = safeProp(vmTrade, "ItemInfoList")
    if not okInfos then
      recordErr(ctx, "ui.shop.items", rawInfos)
    elseif rawInfos ~= nil then
      for _, info in ipairs(readArray(rawInfos, MAX_ARRAY_ITEMS)) do
        if info ~= nil then
          local okIid, iid = safeProp(info, "ID")
          local itemId = okIid and toInt(iid) or nil
          local okName, rawName = safeProp(info, "ItemName")
          local okBuyP, rawBuyP = safeProp(info, "buyingPrice")
          local okSellP, rawSellP = safeProp(info, "sellingPrice")
          local okNum, rawNum = safeProp(info, "ItemNum")
          local okStock, rawStock = safeProp(info, "Stock")
          local okLim, rawLim = safeProp(info, "BuyCountLimit")
          local price = nil
          if isBuy == false then
            if okSellP then price = toInt(rawSellP) end
          else
            if okBuyP then price = toInt(rawBuyP) end
          end
          local entry = {
            id = itemId ~= nil and itemId or json.null,
            name = (okName and rawName ~= nil and readText(rawName)) or json.null,
            price = price ~= nil and price or json.null,
            owned = (okNum and toInt(rawNum)) or json.null,
            stock = (okStock and toInt(rawStock)) or json.null,
            limit = (okLim and toInt(rawLim)) or json.null,
          }
          infoList[#infoList + 1] = entry
          if itemId ~= nil and infoById[itemId] == nil then
            infoById[itemId] = entry
          end
        end
      end
    end

    local okCur, rawCur = safeProp(vmTrade, "CurrentItemList")
    local currentIds = (okCur and rawCur ~= nil) and readArray(rawCur, MAX_ARRAY_ITEMS) or {}
    if #currentIds > 0 then
      for _, rawItemId in ipairs(currentIds) do
        local itemId = toInt(rawItemId)
        if itemId ~= nil then
          items[#items + 1] = infoById[itemId] or {
            id = itemId,
            name = json.null,
            price = json.null,
            owned = json.null,
            stock = json.null,
            limit = json.null,
          }
        end
      end
    else
      items = infoList
    end
  end

  if shopId == nil then
    local okId, rawId = safeProp(vmColShop, "ShopID")
    if okId then
      local idVal = toInt(rawId)
      if idVal ~= nil and idVal >= 0 then shopId = idVal end
    end
  end

  -- Category tab and two-level WBP_OptionList_C.FocuedIndex cursors.
  local categoryIndex = nil
  local categories = {}
  local cmdFocusIndex = nil
  local itemFocusIndex = nil

  local okCmdWin, rootCmdWin = safeProp(systemActive, "CommandWindow")
  if okCmdWin and isValidObject(rootCmdWin) then
    local cmdOpt = getShopOptionList(rootCmdWin)
    if isValidObject(cmdOpt) then
      local okFi, fi = safeProp(cmdOpt, "FocuedIndex")
      if okFi then cmdFocusIndex = toInt(fi) end
    end
  end

  if isValidObject(tradeWin) then
    local okTab, tab = safeProp(tradeWin, "WBP_InventoryCategoryTab")
    if okTab and isValidObject(tab) then
      local okIdx, rawIdx = safeProp(tab, "CurSelectedIdx")
      if okIdx then categoryIndex = toInt(rawIdx) end
      local okCats, rawCats = safeProp(tab, "CategoryList")
      if okCats and rawCats ~= nil then
        for _, tag in ipairs(readArray(rawCats, 16)) do
          local tagName = readTagName(tag)
          if tagName then categories[#categories + 1] = tagName end
        end
      end
    end
    local okTcw, tradeCmdWin = safeProp(tradeWin, "WBP_SHOP_CommandWindow")
    if okTcw and isValidObject(tradeCmdWin) then
      local tradeOpt = getShopOptionList(tradeCmdWin)
      if isValidObject(tradeOpt) then
        local okFi, fi = safeProp(tradeOpt, "FocuedIndex")
        if okFi then itemFocusIndex = toInt(fi) end
      end
    end
  end

  -- Tooltip highlighted item ID from VM_Tooltip_C.ActionKey.ID.
  local tooltipItemId = nil
  local vmTooltip = getCachedSingleton(ctx, cache.gi, "VM_Tooltip_C")
  if isValidObject(vmTooltip) then
    local okAk, actionKey = safeProp(vmTooltip, "ActionKey")
    if okAk and actionKey ~= nil then
      local okId, rawId = safeProp(actionKey, "ID")
      if okId then
        local idVal = toInt(rawId)
        if idVal ~= nil and idVal > 0 then tooltipItemId = idVal end
      end
    end
  end

  local isBuyJson = json.null
  if type(isBuy) == "boolean" then isBuyJson = isBuy end

  return {
    shop_id = shopId ~= nil and shopId or json.null,
    shop_type = shopType or json.null,
    scene_stack = json.array(sceneStack),
    is_buy = isBuyJson,
    category_index = categoryIndex ~= nil and categoryIndex or json.null,
    categories = json.array(categories),
    cmd_focus_index = cmdFocusIndex ~= nil and cmdFocusIndex or json.null,
    item_focus_index = itemFocusIndex ~= nil and itemFocusIndex or json.null,
    tooltip_item_id = tooltipItemId ~= nil and tooltipItemId or json.null,
    total_amount = totalAmount ~= nil and totalAmount or json.null,
    items = json.array(items),
  }
end

-- Helper: scan a list of entry widgets for an active (non-Collapsed, non-Hidden) WBP_FingerCursor_C.
-- Remember: stale items keep their CursorWidget pointer after losing focus and only set Visibility=Collapsed.
local function findFingerCursorFocus(entryWidgets, expectedEntryTags)
  if not entryWidgets or #entryWidgets == 0 then return nil end
  for index, entry in ipairs(entryWidgets) do
    if isValidObject(entry) then
      local node = entry
      local cursor = nil
      for _ = 1, 4 do
        if not isValidObject(node) then break end
        local okCur, cur = safeProp(node, "CursorWidget")
        if okCur and isValidObject(cur) then
          cursor = cur
          break
        end
        local okBtn, btn = safeProp(node, "PathButton")
        if okBtn and isValidObject(btn) then
          node = btn
        else
          local okCall, btn2 = safeCall(node, "GetPathButton")
          if okCall and isValidObject(btn2) then
            node = btn2
          else
            break
          end
        end
      end
      if isValidObject(cursor) and isVisibilityShown(readVisibility(cursor)) then
        if expectedEntryTags and #expectedEntryTags > 0 then
          local okEt, entryType = safeProp(entry, "EntryType")
          local tag = okEt and readTagName(entryType) or nil
          if tag then
            for mappedIdx, expected in ipairs(expectedEntryTags) do
              if expected == tag then return mappedIdx - 1 end
            end
          end
        end
        return index - 1
      end
    end
  end
  return nil
end

-- Substructure #10 helpers: main menu sub_state per active scene tag.
local function collectMainMenuSubState(ctx, cache, topScene, mainLayout)
  if type(topScene) ~= "string" then return json.null end

  local innerActive = nil
  if isValidObject(mainLayout) then
    local okSt, innerStack = safeProp(mainLayout, "CPP_EU_ActivatableWidgetStack_51")
    if okSt and isValidObject(innerStack) then
      innerActive = getStackActiveWidget(innerStack, ctx, "ui.menu.sub_state")
    end
  end

  if topScene == "UI.MainMenu.Scene.Journal" then
    local vmJ = getCachedSingleton(ctx, cache.gi, "VM_Journal_C")
    if not isValidObject(vmJ) then return json.null end
    local okCat, rawCat = safeProp(vmJ, "DisplayQuestCategory")
    local okFoc, rawFoc = safeProp(vmJ, "FocusQuestId")
    local quests = {}
    local okList, rawList = safeProp(vmJ, "DisplayQuestDataList")
    if not okList then
      recordErr(ctx, "ui.menu.sub_state", rawList)
    elseif rawList ~= nil then
      for _, q in ipairs(readArray(rawList, MAX_ARRAY_ITEMS)) do
        if q ~= nil then
          local okId, qid = safeProp(q, "ID")
          local okName, qname = safeProp(q, "Name")
          local okSub, qsub = safeProp(q, "Subtitle")
          quests[#quests + 1] = {
            id = (okId and toInt(qid)) or json.null,
            name = (okName and qname ~= nil and readText(qname)) or json.null,
            subtitle = (okSub and qsub ~= nil and readText(qsub)) or json.null,
          }
        end
      end
    end
    return {
      display_quest_category = (okCat and readEnumName(rawCat, QUEST_CATEGORY_ENUM)) or json.null,
      focus_quest_id = (okFoc and toInt(rawFoc)) or json.null,
      quests = json.array(quests),
    }
  end

  if topScene == "UI.MainMenu.Scene.Heal" then
    local healLayout = (isValidObject(innerActive) and shortClassName(innerActive) == "WBP_HealLayout_C")
      and innerActive or nil
    if not isValidObject(healLayout) then
      healLayout = getCachedSingleton(ctx, cache.world, "WBP_HealLayout_C")
    end
    local vmH = getCachedSingleton(ctx, cache.gi, "VM_HealLayout_C")
    local currentState = nil
    if isValidObject(healLayout) then
      local okCs, cs = safeProp(healLayout, "CurrentState")
      if not okCs then
        recordErr(ctx, "ui.menu.sub_state", cs)
      else
        currentState = toInt(cs)
      end
    end
    local unitId = nil
    local targetUnitId = nil
    local canAoe = false
    local actions = {}
    if isValidObject(vmH) then
      local okUid, uid = safeProp(vmH, "UnitId")
      if okUid then unitId = toInt(uid) end
      local okTuid, tuid = safeProp(vmH, "TargetUnitId")
      if okTuid then targetUnitId = toInt(tuid) end
      local okAoe, aoe = safeProp(vmH, "bCanAoe")
      if okAoe and type(aoe) == "boolean" then canAoe = aoe end
      local okActs, rawActs = safeProp(vmH, "DisplayActionList")
      if okActs and rawActs ~= nil then
        for _, act in ipairs(readArray(rawActs, MAX_ARRAY_ITEMS)) do
          if act ~= nil then
            local okAk, ak = safeProp(act, "ActionKey")
            local cat = nil
            local actId = nil
            if okAk and ak ~= nil then
              local okC, rawC = safeProp(ak, "Category")
              if okC then cat = readEnumName(rawC, ACTION_CATEGORY_ENUM) end
              local okI, rawI = safeProp(ak, "ID")
              if okI then actId = toInt(rawI) end
            end
            local okNum, num = safeProp(act, "TmpNum")
            local okU, userUid = safeProp(act, "UserUnitId")
            actions[#actions + 1] = {
              category = cat or json.null,
              id = actId ~= nil and actId or json.null,
              count = (okNum and toInt(num)) or json.null,
              user_unit_id = (okU and toInt(userUid)) or json.null,
            }
          end
        end
      end
    end
    return {
      current_state = currentState ~= nil and currentState or json.null,
      unit_id = unitId ~= nil and unitId or json.null,
      target_unit_id = targetUnitId ~= nil and targetUnitId or json.null,
      can_aoe = canAoe,
      actions = json.array(actions),
    }
  end

  if topScene == "UI.MainMenu.Scene.Item" then
    local itemLayout = (isValidObject(innerActive) and shortClassName(innerActive) == "WBP_ItemLayout_C")
      and innerActive or nil
    if not isValidObject(itemLayout) then
      itemLayout = getCachedSingleton(ctx, cache.world, "WBP_ItemLayout_C")
    end
    local currentState = nil
    if isValidObject(itemLayout) then
      local okCs, cs = safeProp(itemLayout, "CurrentState")
      if not okCs then
        recordErr(ctx, "ui.menu.sub_state", cs)
      else
        currentState = toInt(cs)
      end
    end
    return {
      current_state = currentState ~= nil and currentState or json.null,
    }
  end

  if topScene == "UI.MainMenu.Scene.Map" then
    local vmMap = getCachedSingleton(ctx, cache.gi, "VM_WldMap_C")
    if not isValidObject(vmMap) then return json.null end
    local okIsl, rawIsl = safeProp(vmMap, "CurrentIslandType")
    local okArea, rawArea = safeProp(vmMap, "CurrentAreaId")
    return {
      current_island_type = (okIsl and readEnumName(rawIsl, ISLAND_TYPE_ENUM)) or json.null,
      current_area_id = (okArea and toInt(rawArea)) or json.null,
    }
  end

  return json.null
end

-- Substructure #10: ui.menu (system_menu / savepoint / main_menu / title)
local function collectMenu(ctx, cache, systemActive, gameActive, mainMenuSub)
  local sysClass = isValidObject(systemActive) and shortClassName(systemActive) or nil

  -- 1. System menu (WBP_SystemMenuLayout_C)
  if sysClass == "WBP_SystemMenuLayout_C" then
    local sysMenu = nil
    local okSm, sm = safeProp(systemActive, "WBP_SystemMenu")
    if okSm and isValidObject(sm) then sysMenu = sm end

    local vmSys = nil
    if isValidObject(sysMenu) then
      local okVm, vm = safeProp(sysMenu, "VM_SystemMenu")
      if okVm and isValidObject(vm) then vmSys = vm end
    end
    if not isValidObject(vmSys) then
      vmSys = getCachedSingleton(ctx, cache.gi, "VM_SystemMenu_C")
    end

    local entries = {}
    if isValidObject(vmSys) then
      local okEl, rawEl = safeProp(vmSys, "EnableEntryList")
      if not okEl then
        recordErr(ctx, "ui.menu.entries", rawEl)
      elseif rawEl ~= nil then
        for _, tag in ipairs(readArray(rawEl, 32)) do
          local name = readTagName(tag)
          if name then entries[#entries + 1] = truncateText(name, TEXT_LIMIT) end
        end
      end
    end

    local focusIndex = nil
    local entryWidgets = nil
    for _, node in ipairs(collectMenuPathChain(sysMenu or systemActive)) do
      if focusIndex == nil then
        focusIndex = getMenuFocusIndex(node)
      end
      if entryWidgets == nil then
        local okGp, gp = safeProp(node, "DirectionGridPanel")
        if okGp and isValidObject(gp) then
          local okEw, rawEw = safeProp(gp, "EntryWidgetList")
          if okEw and rawEw ~= nil then entryWidgets = readArray(rawEw, 32) end
        end
      end
    end

    if #entries == 0 and entryWidgets ~= nil then
      for _, item in ipairs(entryWidgets) do
        if isValidObject(item) then
          local okEt, et = safeProp(item, "EntryType")
          local label = okEt and readTagName(et) or nil
          if not label then
            local okTb, tb = safeProp(item, "CommonTextBlock_49")
            if okTb and tb ~= nil then label = readText(tb) end
          end
          if label then entries[#entries + 1] = truncateText(label, TEXT_LIMIT) end
        end
      end
    end

    if focusIndex == nil and entryWidgets ~= nil then
      focusIndex = findFingerCursorFocus(entryWidgets, entries)
    end

    return {
      kind = "system_menu",
      scene_stack = json.array({}),
      entries = json.array(entries),
      focus_index = focusIndex ~= nil and focusIndex or json.null,
      sub_state = json.null,
    }
  end

  -- 2. Savepoint menu (WBP_SavePointLayout_C)
  if sysClass == "WBP_SavePointLayout_C" then
    local entries = {}
    local okVm, vmSp = safeProp(systemActive, "m_VM_SavePoint")
    if okVm and isValidObject(vmSp) then
      local okCmds, rawCmds = safeCall(vmSp, "GetCommandTypeList")
      if okCmds and rawCmds ~= nil then
        for _, cmdType in ipairs(readArray(rawCmds, 16)) do
          local name = readEnumName(cmdType, SAVEPOINT_CMD_ENUM)
          if name and name ~= "None" then entries[#entries + 1] = name end
        end
      end
    end

    local focusIndex = nil
    local okCw, cmdWin = safeProp(systemActive, "CommandWindow")
    if not okCw then
      recordErr(ctx, "ui.menu.focus_index", cmdWin)
    elseif isValidObject(cmdWin) then
      local optList = getShopOptionList(cmdWin)
      if isValidObject(optList) then
        local okFi, fi = safeProp(optList, "FocuedIndex")
        if okFi then focusIndex = toInt(fi) end
        if focusIndex == nil then focusIndex = getMenuFocusIndex(optList) end
      end
    end

    return {
      kind = "savepoint",
      scene_stack = json.array({}),
      entries = json.array(entries),
      focus_index = focusIndex ~= nil and focusIndex or json.null,
      sub_state = json.null,
    }
  end

  -- 3. Main menu (WBP_MainMenuLayout_C or CPP_MainMenuSubsystem:IsOpenMenu() + CPP_VM_MainMenu.SceneDataStack)
  local mainLayout = (sysClass == "WBP_MainMenuLayout_C") and systemActive or nil
  local vmMain = nil
  if isValidObject(mainLayout) then
    local okVm, vm = safeProp(mainLayout, "CPP_VM_MainMenu")
    if okVm and isValidObject(vm) then vmMain = vm end
  end
  if not isValidObject(vmMain) and isValidObject(mainMenuSub) then
    local okOpen, isOpen = safeCall(mainMenuSub, "IsOpenMenu")
    if okOpen and isOpen == true then
      local okVm, vm = safeCall(mainMenuSub, "GetMainMenuVM")
      if okVm and isValidObject(vm) then vmMain = vm end
      if not isValidObject(mainLayout) then
        local okLay, lay = safeCall(mainMenuSub, "GetLayoutWidget")
        if okLay and isValidObject(lay) then mainLayout = lay end
      end
    end
  end
  if not isValidObject(vmMain) and isValidObject(mainLayout) then
    vmMain = getCachedSingleton(ctx, cache.gi, "CPP_VM_MainMenu")
  end

  if isValidObject(vmMain) then
    local sceneStack = {}
    local okSs, rawSs = safeProp(vmMain, "SceneDataStack")
    if not okSs then
      recordErr(ctx, "ui.menu.scene_stack", rawSs)
    elseif rawSs ~= nil then
      for _, tag in ipairs(readArray(rawSs, 16)) do
        local name = readTagName(tag)
        if name then sceneStack[#sceneStack + 1] = name end
      end
    end

    if #sceneStack > 0 then
      local topScene = sceneStack[#sceneStack]
      local entries = {}
      if topScene == "UI.MainMenu.Scene.Root" then
        local okAll, rawAll = safeProp(vmMain, "AllSceneDataList")
        if okAll and rawAll ~= nil then
          for _, sd in ipairs(readArray(rawAll, 32)) do
            if sd ~= nil then
              local okEn, en = safeProp(sd, "IsEnable")
              local okTag, stag = safeProp(sd, "SceneTag")
              local tagName = okTag and readTagName(stag) or nil
              if tagName and tagName ~= "UI.MainMenu.Scene.Root" and (not okEn or en ~= false) then
                entries[#entries + 1] = tagName
              end
            end
          end
        end
      end

      local focusIndex = nil
      if isValidObject(mainLayout) then
        local okTarget, rawTarget = safeProp(mainLayout, "InputTargetWidget")
        local targetWidget = okTarget and resolveObject(rawTarget) or nil
        if isValidObject(targetWidget) then
          for _, node in ipairs(collectMenuPathChain(targetWidget)) do
            focusIndex = getMenuFocusIndex(node)
            if focusIndex ~= nil then break end
          end
        end
      end

      local subState = collectMainMenuSubState(ctx, cache, topScene, mainLayout)
      return {
        kind = "main_menu",
        scene_stack = json.array(sceneStack),
        entries = json.array(entries),
        focus_index = focusIndex ~= nil and focusIndex or json.null,
        sub_state = subState,
      }
    end
  end

  -- 4. Title screen menu (WBP_TitleLayout_C on Stack_Game)
  if isValidObject(gameActive) and shortClassName(gameActive) == "WBP_TitleLayout_C" then
    local titleMenu = nil
    local okTarget, rawTarget = safeProp(gameActive, "InputTargetWidget")
    local target = okTarget and resolveObject(rawTarget) or nil
    if isValidObject(target) then
      local okGp, gp = safeProp(target, "DirectionGridPanel")
      if getMenuFocusIndex(target) ~= nil or (okGp and isValidObject(gp)) then
        titleMenu = target
      end
    end
    if not isValidObject(titleMenu) then
      local okDm, dm = safeProp(gameActive, "WBP_DirectionLayoutMenu")
      if okDm and isValidObject(dm) then titleMenu = dm end
    end

    local entries = {}
    local focusIndex = nil
    local entryWidgets = nil
    if isValidObject(titleMenu) then
      focusIndex = getMenuFocusIndex(titleMenu)
      local okGp, gp = safeProp(titleMenu, "DirectionGridPanel")
      if okGp and isValidObject(gp) then
        local okEw, rawEw = safeProp(gp, "EntryWidgetList")
        if okEw and rawEw ~= nil then entryWidgets = readArray(rawEw, 16) end
      end
    end

    if entryWidgets ~= nil then
      for _, item in ipairs(entryWidgets) do
        if isValidObject(item) then
          local label = nil
          local okTb, tb = safeProp(item, "CommonTextBlock_47")
          if okTb and tb ~= nil then label = readText(tb) end
          if not label or label == "" then
            local okEt, et = safeProp(item, "EntryType")
            if okEt then label = readEnumName(et, TITLE_ENTRY_ENUM) end
          end
          if label and label ~= "" then entries[#entries + 1] = label end
        end
      end
      if focusIndex == nil then
        focusIndex = findFingerCursorFocus(entryWidgets, nil)
      end
    end

    return {
      kind = "title",
      scene_stack = json.array({}),
      entries = json.array(entries),
      focus_index = focusIndex ~= nil and focusIndex or json.null,
      sub_state = json.null,
    }
  end

  return nil
end

-- Substructure #13: ui.battle_result
local function collectBattleResult(ctx, taskClass, uiBattle)
  if taskClass ~= "CPP_BtlTaskWin" then return nil end
  if not isValidObject(uiBattle) then return nil end

  local okRw, resultWidget = safeProp(uiBattle, "ResultWidget")
  if not okRw then
    recordErr(ctx, "ui.battle_result", resultWidget)
    return nil
  end
  if not isValidObject(resultWidget) then return nil end

  -- Pooling guard: ResultWidget stays non-null after exiting result screen, with IsFinishPage == true.
  local okFin, isFinish = safeProp(resultWidget, "IsFinishPage")
  if not okFin then
    recordErr(ctx, "ui.battle_result.is_finish_page", isFinish)
    return nil
  end
  if isFinish ~= false then return nil end

  local pageIndex = nil
  local hasLevelUpWindow = false

  local okPl, pageLayout = safeProp(resultWidget, "WBP_BtlResultPage_Layout")
  if not okPl then
    recordErr(ctx, "ui.battle_result.page_index", pageLayout)
  elseif isValidObject(pageLayout) then
    local okPi, rawPi = safeProp(pageLayout, "CurrentPageIndex")
    if not okPi then
      recordErr(ctx, "ui.battle_result.page_index", rawPi)
    else
      pageIndex = toInt(rawPi)
    end

    -- Check WBP_BtlResultPage_LevelUpWindow_C.IsShouldDisplay on the growth page (index 1).
    local levelUpWin = nil
    local okSw, switcher = safeProp(pageLayout, "WidgetSwitcher")
    if okSw and isValidObject(switcher) then
      local activePage = nil
      local okAp, ap = safeCall(switcher, "GetActiveWidget")
      if okAp and isValidObject(ap) then
        activePage = ap
      elseif pageIndex ~= nil then
        local okAt, at = safeCall(switcher, "GetWidgetAtIndex", pageIndex)
        if okAt and isValidObject(at) then activePage = at end
      end
      if isValidObject(activePage) then
        local okLu, lu = safeProp(activePage, "WBP_BtlResultPage_LevelUpWindow")
        if okLu and isValidObject(lu) then levelUpWin = lu end
      end
    end
    if not isValidObject(levelUpWin) then
      local okLu, lu = safeProp(pageLayout, "WBP_BtlResultPage_LevelUpWindow")
      if okLu and isValidObject(lu) then levelUpWin = lu end
    end
    if isValidObject(levelUpWin) then
      local okDisp, disp = safeProp(levelUpWin, "IsShouldDisplay")
      if okDisp and type(disp) == "boolean" then
        hasLevelUpWindow = disp
      else
        local okCall, disp2 = safeCall(levelUpWin, "GetIsShouldDisplay")
        if okCall and type(disp2) == "boolean" then hasLevelUpWindow = disp2 end
      end
    end
  end

  return {
    page_index = pageIndex ~= nil and pageIndex or json.null,
    has_level_up_window = hasLevelUpWindow,
  }
end

-- Helper: extract key name from PlayerKeyMapping / FKey / FName / string.
local function extractKeyName(mapping)
  if mapping == nil then return nil end
  local direct = toStr(mapping)
  if direct ~= nil then
    return (direct ~= "" and direct ~= "None") and direct or nil
  end
  local function readKeyStruct(keyStruct)
    if keyStruct == nil then return nil end
    local s = toStr(keyStruct)
    if s ~= nil then return (s ~= "" and s ~= "None") and s or nil end
    local okKn, kn = safeProp(keyStruct, "KeyName")
    if okKn and kn ~= nil then
      local ks = toStr(kn)
      if ks and ks ~= "" and ks ~= "None" then return ks end
    end
    return nil
  end
  -- PlayerKeyMapping: prefer CurrentKey, fallback DefaultKey.
  local okCur, curKey = safeProp(mapping, "CurrentKey")
  if okCur and curKey ~= nil then
    local name = readKeyStruct(curKey)
    if name then return name end
  end
  local okDef, defKey = safeProp(mapping, "DefaultKey")
  if okDef and defKey ~= nil then
    local name = readKeyStruct(defKey)
    if name then return name end
  end
  -- Direct FKey struct.
  return readKeyStruct(mapping)
end

-- Main per-tick collector.
function M.collect(ctx)
  local cache = ensureCache(ctx)
  syncWorldCache(ctx, cache)

  -- Fetch core singletons (served from ctx.cache on steady-state ticks).
  local gwMgr, gwErr = getCachedSingleton(ctx, cache.gi, "CPP_GameWidgetManagerSubsystem")
  if gwErr then
    recordErr(ctx, "ui.game_widget_manager", gwErr)
  elseif not isValidObject(gwMgr) then
    recordErr(ctx, "ui.game_widget_manager", "找不到有效的 CPP_GameWidgetManagerSubsystem")
  end

  local guideSub, guideErr = getCachedSingleton(ctx, cache.gi, "CPP_ActionGuideSubsystem")
  if guideErr then
    recordErr(ctx, "ui.action_guide_tag", guideErr)
  elseif not isValidObject(guideSub) then
    recordErr(ctx, "ui.action_guide_tag", "找不到有效的 CPP_ActionGuideSubsystem")
  end

  local movieSub, movieErr = getCachedSingleton(ctx, cache.gi, "CPP_MovieSubsystem")
  if movieErr then recordErr(ctx, "ui.movie", movieErr) end

  local talkSub, talkErr = getCachedSingleton(ctx, cache.gi, "CPP_TalkEventSubsystem")
  if talkErr then recordErr(ctx, "ui.talk", talkErr) end

  local tutSub, tutErr = getCachedSingleton(ctx, cache.gi, "CPP_TutorialSubsystem")
  if tutErr then recordErr(ctx, "ui.tutorial", tutErr) end

  local taskMgr, taskErr = getCachedSingleton(ctx, cache.gi, "CPP_TaskManager")
  if taskErr then recordErr(ctx, "ui.task", taskErr) end

  local uiBattle, btlErr = getCachedSingleton(ctx, cache.world, "CPP_UI_BattleSubsystem")
  if btlErr then recordErr(ctx, "ui.battle", btlErr) end

  -- CPP_MainMenuSubsystem is absent on title screen; missing is normal, only log errors.
  local mainMenuSub, mmErr = getCachedSingleton(ctx, cache.world, "CPP_MainMenuSubsystem")
  if mmErr then recordErr(ctx, "ui.menu", mmErr) end

  -- 1. Layout root (WBP_OverAllLayout_C) -> active_layer & active_widget
  local layout = nil
  local activeLayer = nil
  local activeWidget = nil
  if isValidObject(gwMgr) then
    local okLayout, rawLayout = safeProp(gwMgr, "m_GameLayout")
    if not okLayout then
      recordErr(ctx, "ui.active_layer", rawLayout)
    elseif isValidObject(rawLayout) then
      layout = rawLayout
      local layerObj = nil
      local okGetLayer, l1, existed = safeCall(layout, "GetInputTargetLayer")
      if okGetLayer and isValidObject(l1) then
        layerObj = l1
      else
        if existed and not okGetLayer and l1 ~= nil then
          recordErr(ctx, "ui.active_layer", l1)
        end
        local okCur, l2 = safeProp(layout, "CurrentInputTargetLayer")
        if not okCur then
          recordErr(ctx, "ui.active_layer", l2)
        elseif isValidObject(l2) then
          layerObj = l2
        end
      end
      if isValidObject(layerObj) then
        activeLayer = resolveLayerTag(layout, layerObj)
        local topW = getStackActiveWidget(layerObj, ctx, "ui.active_widget")
        if isValidObject(topW) then
          activeWidget = shortClassName(topW)
        end
      end
    end
  end

  -- 2. action_guide_tag
  local actionGuideTag = nil
  if isValidObject(guideSub) then
    local okTag, rawTag = safeProp(guideSub, "GuideTag")
    if not okTag then
      recordErr(ctx, "ui.action_guide_tag", rawTag)
    else
      actionGuideTag = readTagName(rawTag)
    end
  end

  -- 3. input_blocked
  local inputBlocked = collectInputBlocked(ctx, cache, gwMgr)

  -- 4. Loading, task class, battle flag
  local isLoading = false
  if isValidObject(gwMgr) then
    local okLsm, lsm = safeProp(gwMgr, "m_LoadScrManager")
    if not okLsm then
      recordErr(ctx, "ui.loading", lsm)
    elseif isValidObject(lsm) then
      local okShow, show = safeProp(lsm, "IsShowLoadingScreen")
      if not okShow then
        recordErr(ctx, "ui.loading", show)
      elseif show == true then
        isLoading = true
      end
    end
  end

  local taskClass = nil
  if isValidObject(taskMgr) then
    local okTask, task = safeProp(taskMgr, "m_pTask")
    if not okTask then
      recordErr(ctx, "ui.task", task)
    elseif isValidObject(task) then
      taskClass = shortClassName(task)
    end
  end

  local inBattle = false
  if isValidObject(uiBattle) then
    local okBtl, during = safeProp(uiBattle, "bDuringBattle")
    if not okBtl then
      recordErr(ctx, "ui.battle", during)
    elseif during == true then
      inBattle = true
    end
  end

  -- 5. Active event manager & talk widget (shared by movie subtitle, choice, talk)
  local evMgr = nil
  local talkWidget = nil
  if isValidObject(talkSub) then
    local okEv, rawEv = safeProp(talkSub, "m_pEventManager")
    if not okEv then
      recordErr(ctx, "ui.talk", rawEv)
    elseif isValidObject(rawEv) then
      evMgr = rawEv
      local okTw, tw = safeProp(evMgr, "m_pWidget")
      if not okTw then
        recordErr(ctx, "ui.talk", tw)
      elseif isValidObject(tw) then
        talkWidget = tw
      end
    end
  end

  -- 6. Read layer stacks once for menu/shop/save_ui/title
  local systemStack = findLayerStack(layout, "Stack_System", "UI.Layer.System")
  local systemActive = getStackActiveWidget(systemStack, ctx, "ui.menu")
  local gameStack = findLayerStack(layout, "Stack_Game", "UI.Layer.Game")
  local gameActive = getStackActiveWidget(gameStack, ctx, "ui.menu")

  -- 7. Collect the 8 on-demand substructures
  local movie = collectMovie(ctx, movieSub, talkWidget)
  local tutorial = collectTutorial(ctx, tutSub)
  local saveUi = collectSaveUi(ctx, layout)
  local shop = collectShop(ctx, cache, systemActive)
  local menu = collectMenu(ctx, cache, systemActive, gameActive, mainMenuSub)
  local choice = collectChoice(ctx, cache, gwMgr, talkWidget)
  local battleResult = collectBattleResult(ctx, taskClass, uiBattle)

  local isSavepoint = (menu ~= nil and menu.kind == "savepoint")
  local talk = nil
  if isValidObject(evMgr) and movie == nil and shop == nil and not isSavepoint and saveUi == nil then
    talk = collectTalk(ctx, cache, evMgr, talkWidget)
  end

  -- 8. Derive screen_kind strictly following state-map/ui.md §8.1 priority order.
  local screenKind = "field"
  if isLoading then
    screenKind = "loading"
  elseif movie ~= nil then
    screenKind = "movie"
  elseif tutorial ~= nil then
    screenKind = "tutorial"
  elseif choice ~= nil and choice.source == "dialog" then
    screenKind = "dialog"
  elseif choice ~= nil and choice.source == "event_choice" then
    screenKind = "choice"
  elseif talk ~= nil and not talk.is_cutscene then
    screenKind = "talk"
  elseif talk ~= nil and talk.is_cutscene then
    screenKind = "cutscene"
  elseif saveUi ~= nil then
    screenKind = "save_ui"
  elseif menu ~= nil and menu.kind == "system_menu" then
    screenKind = "system_menu"
  elseif shop ~= nil then
    screenKind = "shop"
  elseif isSavepoint then
    screenKind = "savepoint"
  elseif menu ~= nil and menu.kind == "main_menu" then
    screenKind = "main_menu"
  elseif battleResult ~= nil then
    screenKind = "battle_result"
  elseif inBattle then
    screenKind = "battle"
  elseif taskClass == "CPP_TitleTaskMain" or (menu ~= nil and menu.kind == "title") then
    screenKind = "title"
  end

  return {
    screen_kind = screenKind,
    active_layer = activeLayer or json.null,
    active_widget = activeWidget or json.null,
    action_guide_tag = actionGuideTag or json.null,
    input_blocked = inputBlocked,
    movie = movie ~= nil and movie or json.null,
    talk = talk ~= nil and talk or json.null,
    choice = choice ~= nil and choice or json.null,
    tutorial = tutorial ~= nil and tutorial or json.null,
    menu = menu ~= nil and menu or json.null,
    shop = shop ~= nil and shop or json.null,
    save_ui = saveUi ~= nil and saveUi or json.null,
    battle_result = battleResult ~= nil and battleResult or json.null,
  }
end

-- On-demand read-only command: input_state (for R4 input inspection and key-binding lookup).
M.commands.input_state = function(ctx, args)
  local cache = ensureCache(ctx)
  syncWorldCache(ctx, cache)

  local inputMgr, mgrErr = getCachedSingleton(ctx, cache.world, "CPP_InputManager")
  if mgrErr then
    return nil, "查找 CPP_InputManager 失败：" .. tostring(mgrErr)
  end
  if not isValidObject(inputMgr) then
    return nil, "找不到有效的 CPP_InputManager"
  end

  local gwMgr = getCachedSingleton(ctx, cache.gi, "CPP_GameWidgetManagerSubsystem")

  -- 1. active_imcs via CPP_InputManager:GetAllIMCStackList()
  local activeImcs = {}
  local seenImc = {}
  local function pushImc(name)
    if name and name ~= "" and name ~= "None" and not seenImc[name] then
      seenImc[name] = true
      activeImcs[#activeImcs + 1] = truncateText(name, TEXT_LIMIT)
    end
  end

  local okStack, rawStack = safeCall(inputMgr, "GetAllIMCStackList")
  if okStack and rawStack ~= nil then
    for _, entry in ipairs(readArray(rawStack, MAX_ARRAY_ITEMS)) do
      if entry ~= nil then
        -- Handle MUCHA_INPUT_MAPPING_CONTEXT_DATA struct { parentMappingContext, childMappingContextList }.
        local okParent, parentVal = safeProp(entry, "parentMappingContext")
        if okParent and parentVal ~= nil then
          pushImc(readEnumName(parentVal, INPUT_MAPPING_TYPE_ENUM))
          local okChild, childList = safeProp(entry, "childMappingContextList")
          if okChild and childList ~= nil then
            for _, child in ipairs(readArray(childList, 32)) do
              pushImc(readEnumName(child, INPUT_MAPPING_TYPE_ENUM))
            end
          end
        elseif isValidObject(entry) then
          pushImc(shortObjectName(entry))
        else
          pushImc(readEnumName(entry, INPUT_MAPPING_TYPE_ENUM))
        end
      end
    end
  end

  -- Fallback: EnhancedPlayerInput.AppliedInputContexts when GetAllIMCStackList returns empty.
  if #activeImcs == 0 then
    local pc = getCachedSingleton(ctx, cache.world, "PlayerController")
    if isValidObject(pc) then
      local okPi, playerInput = safeProp(pc, "PlayerInput")
      if okPi and isValidObject(playerInput) then
        local okApplied, applied = safeProp(playerInput, "AppliedInputContexts")
        if okApplied and applied ~= nil then
          forEachMap(applied, function(imcObj)
            if isValidObject(imcObj) then
              pushImc(shortObjectName(imcObj))
            else
              pushImc(toStr(imcObj))
            end
          end, 32)
        end
      end
    end
  end

  -- 2. swap_decide
  local swapDecide = json.null
  local okSwap, rawSwap = safeProp(inputMgr, "bSwapDecideButton")
  if okSwap and type(rawSwap) == "boolean" then swapDecide = rawSwap end

  -- 3. input_enabled
  local inputEnabled = json.null
  local okEn, rawEn = safeCall(inputMgr, "IsEnableInput")
  if okEn and type(rawEn) == "boolean" then inputEnabled = rawEn end

  -- 4. input_blocked
  local inputBlocked = collectInputBlocked(ctx, cache, gwMgr)

  -- 5. key_bindings via CPP_InputManager:FindCurrentMappingForSlot(MappingName, slot)
  local slot = 0
  local mappingNames = DEFAULT_MAPPING_NAMES
  if type(args) == "table" then
    if isFiniteNumber(args.slot) then slot = math.floor(args.slot) end
    if type(args.mapping_names) == "table" and #args.mapping_names > 0 then
      mappingNames = args.mapping_names
    end
  end

  local fnameCtor = rawget(_G, "FName")
  local keyBindings = {}
  for index = 1, math.min(#mappingNames, MAX_ARRAY_ITEMS) do
    local name = mappingNames[index]
    if type(name) == "string" and name ~= "" then
      local okMap, mapping = safeCall(inputMgr, "FindCurrentMappingForSlot", name, slot)
      if not okMap and type(fnameCtor) == "function" then
        local okFn, fnameObj = pcall(fnameCtor, name)
        if okFn and fnameObj ~= nil then
          okMap, mapping = safeCall(inputMgr, "FindCurrentMappingForSlot", fnameObj, slot)
        end
      end
      local keyName = (okMap and mapping ~= nil) and extractKeyName(mapping) or nil
      keyBindings[name] = keyName or json.null
    end
  end

  return {
    active_imcs = json.array(activeImcs),
    swap_decide = swapDecide,
    input_enabled = inputEnabled,
    input_blocked = inputBlocked,
    key_bindings = json.object(keyBindings),
  }, nil
end

return M
