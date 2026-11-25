-- FfrBridge v1 topic "battle" (task 11-battle). Read-only: only read properties and call getters;
-- never call functions that change game state and never send input.
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
-- Lookups per tick (ctx.find / ctx.findAll):
--   - ctx.findAll: 0 (never scans large/pooled classes).
--   - Out of battle (in_battle ~= true): 4 ctx.find on cold cache / map change, 0 on warm ticks.
--   - In battle (normal/Target): up to 8 ctx.find on cold cache (4 resident + 4 battle singletons),
--     0 on warm ticks.
--   - In battle (cmd List open): +1 ctx.find for WBP_BattleLayout_C on first open, 0 on warm ticks.
--   - In battle (victory result active): +1 ctx.find for CPP_VM_BattleResult on first active tick,
--     0 on warm ticks.
--   - Cached singletons are re-validated with IsValid() every tick, cleared on ctx.world/ctx.map
--     change, and battle-scoped singletons are cleared on in_battle transitions. Non-singleton child
--     objects (tasks, units, widgets, jobs) are never cached across ticks.
--
-- Output fields (data.battle; based on docs/state-map/battle.md §11):
--   | Field                         | Type               | Source / Mapping                                                                 | Sampling Condition |
--   | ----------------------------- | ------------------ | -------------------------------------------------------------------------------- | ------------------ |
--   | in_battle                     | boolean | null     | CPP_UI_BattleSubsystem.bDuringBattle                                             | Every tick         |
--   | task_class                    | string | null      | CPP_TaskManager.m_pTask short class name (e.g. "CPP_BtlTaskCommand")             | Every tick         |
--   | in_talk_event                 | boolean | null     | CPP_TalkEventSubsystem.m_pEventManager:IsValid()                                 | Every tick         |
--   | in_tutorial                   | boolean | null     | CPP_TutorialSubsystem.m_TutorialWidget:IsValid()                                 | Every tick         |
--   | phase                         | string | null      | uiBattle.BtlPhaseType -> "Normal"|"Additional"|"Finish"                          | in_battle == true  |
--   | is_event                      | boolean | null     | CPP_VM_Battle.bIsEvent                                                           | in_battle == true  |
--   | is_ui_visible                 | boolean | null     | CPP_VM_Battle.bIsDisplayBattleWidget                                             | in_battle == true  |
--   | working_job_count             | number | null      | #m_WorkingJobList + #m_WaitingJobList on CPP_BtlManager.m_pBattleJobManager      | in_battle == true  |
--   | current_actor_uid             | number | null      | uiBattle.CurrentActionUnitUniqueId (-1 when none)                                | in_battle == true  |
--   | timeline                      | number[] | null    | VM_Timeline_C.TimelineUnified.UnifiedIds (battle UniqueIDs)                      | in_battle == true  |
--   | risk_uids                     | number[] | null    | VM_Timeline_C.RiskUnitUniqueIdList (battle UniqueIDs)                            | in_battle == true  |
--   | cmd                           | object | null      | VM_BtlCmd_C + uiBattle.BtlCmdManager + WBP_BattleLayout_C                        | in_battle == true  |
--   | cmd.display                   | boolean | null     | vmCmd.bDisplayCmd                                                                | in_battle == true  |
--   | cmd.type                      | string | null      | cmdMgr.CmdType -> "Normal"|"Additional"|"Finish"                                 | in_battle == true  |
--   | cmd.state                     | string | null      | cmdMgr.CmdState -> "List"|"Target"                                               | in_battle == true  |
--   | cmd.ancestors                 | number[] | null    | vmCmd.AncestorIdList (DT_CommandSkillData.ID / DT_SkillData.ID)                  | in_battle == true  |
--   | cmd.focus_index               | number | null      | vmCmd.FocusCmdData.EntryIndex                                                    | in_battle == true  |
--   | cmd.focus_action_id           | number | null      | vmCmd.FocusCmdData.Value.ActionKey.ID (DT_CommandSkillData.ID / DT_SkillData.ID) | in_battle == true  |
--   | cmd.primary_ids               | number[] | null    | WBP_BattleLayout_C...PrimaryList.WBP_List.ActionKeyList[*].ID                    | display & List     |
--   | cmd.secondary_ids             | number[] | null    | WBP_BattleLayout_C...SecondaryList.WBP_List.ActionKeyList[*].ID                  | display & List & #ancestors > 0 |
--   | cmd.target                    | object | null      | vmCmd.UnitSelectVM                                                               | display & Target   |
--   | cmd.target.skill_id           | number | null      | sel.CmdValue.ActionKey.ID (DT_SkillData.ID / Item/* ID)                          | display & Target   |
--   | cmd.target.focus_uid          | number | null      | sel.FocusBaseUniqueId                                                            | display & Target   |
--   | cmd.target.target_uids        | number[] | null    | sel.targetUniqueIdList                                                           | display & Target   |
--   | cmd.target.all_target_uids    | number[] | null    | sel.AllTargetUniqueIdList                                                        | display & Target   |
--   | cmd.target.can_aoe            | boolean | null     | sel.bCanBeAoE                                                                    | display & Target   |
--   | cmd.target.is_all             | boolean | null     | sel.CmdValue.IsAll                                                               | display & Target   |
--   | units                         | object[] | null    | uiBattle.BtlUnitManager.UnitParamVMMap (sorted by uid asc)                       | in_battle == true  |
--   | units[].uid                   | number | null      | vm.UniqueID                                                                      | in_battle == true  |
--   | units[].unit_id               | number | null      | vm.UnitId (-> DT_UnitParameter.ID)                                               | in_battle == true  |
--   | units[].name                  | string | null      | vm.ShortName (original text, truncated to 200 UTF-8 chars)                       | in_battle == true  |
--   | units[].relation              | number | null      | param:GetRelation() (fallback vm:GetRelation(); 1=Friendlies, 2=Enemies)         | in_battle == true  |
--   | units[].is_boss               | boolean | null     | param:IsBoss() (fallback vm:GetIsBoss())                                         | in_battle == true  |
--   | units[].is_dead               | boolean | null     | param:IsDead() (fallback not vm.bIsVisible)                                      | in_battle == true  |
--   | units[].hp / max_hp           | number | null      | vm.HitPointVM.{CurrentPoint,MaxPoint} (fallback param:GetHitPoint/GetMaxHitPoint)| in_battle == true  |
--   | units[].mp / max_mp           | number | null      | vm.MagicPointVM.{CurrentPoint,MaxPoint} (fallback param:GetMagicPoint/GetMax...) | in_battle == true  |
--   | units[].bp / max_bp           | number | null      | vm.BreakPointVM.{CurrentPoint,MaxPoint} (fallback param:GetBreakPoint/GetMax...) | in_battle == true  |
--   | units[].lb / lb_max / lb_level| number | null      | vm.LimitPointVM.{CurrentPoint,MaxPoint,CurrentLevel}                             | in_battle == true  |
--   | units[].is_break              | boolean | null     | param:IsBreak() (fallback m_ContEffectList contains CPP_ContFxBreak)             | in_battle == true  |
--   | units[].is_risky              | boolean | null     | vm:IsScheduledHighRiskSkill() or (uid in risk_uids)                              | in_battle == true  |
--   | units[].scheduled_skill_id    | number | null      | param.m_pOwnerBtlUnit.m_pAI.m_ScheduledSkillId (-> DT_SkillData.ID)              | Enemies only       |
--   | units[].in_action             | boolean | null     | vm.ActionVM.bIsInAction                                                          | in_battle == true  |
--   | units[].in_ex_phase           | boolean | null     | vm.ActionVM.bIsInExPhase                                                         | in_battle == true  |
--   | units[].target_state          | string | null      | vm.TargetSelectState -> "None"|"Main"|"Sub"                                      | in_battle == true  |
--   | units[].weaknesses            | object[] | null    | vm.ElemResistDataList -> {element,resistance,analyzed} (non-None only)           | Enemies only       |
--   | result                        | object | null      | uiBattle.ResultWidget + CPP_VM_BattleResult.BtlResultData                        | task_class=="CPP_BtlTaskWin" & !IsFinishPage |
--   | result.page_index             | number | null      | ResultWidget.WBP_BtlResultPage_Layout.CurrentPageIndex (0=reward, 1=growth)      | result active      |
--   | result.total_exp              | number | null      | BtlResultData.totalExperience                                                    | result active      |
--   | result.total_money            | number | null      | BtlResultData.totalMoney                                                         | result active      |
--   | result.total_ap               | number | null      | BtlResultData.totalAp                                                            | result active      |
--   | result.enemy_ids              | number[] | null    | BtlResultData.enemyDataList (-> DT_UnitParameter.ID)                             | result active      |
--   | result.items                  | object[] | null    | BtlResultData.itemDataList -> {item_id (-> Item/* ID), num}                      | result active      |

local json = require("ffrbridge.json")

local M = { name = "battle", commands = {} }

local TEXT_LIMIT = 200
local MAX_ARRAY_LEN = 256

local BATTLE_SCOPED_SINGLETONS = {
  "CPP_BtlManager",
  "CPP_VM_Battle",
  "VM_BtlCmd_C",
  "VM_Timeline_C",
  "WBP_BattleLayout_C",
  "CPP_VM_BattleResult",
}

local BTL_PHASE_TYPE = {
  [0] = "Normal",
  [1] = "Additional",
  [2] = "Finish",
}

local BTL_CMD_TYPE = {
  [0] = "Normal",
  [1] = "Additional",
  [2] = "Finish",
}

local BTL_CMD_STATE = {
  [0] = "List",
  [1] = "Target",
}

local TARGET_SELECT_STATE = {
  [0] = "None",
  [1] = "Main",
  [2] = "Sub",
}

local RELATION_BY_NAME = {
  None = 0,
  Friendlies = 1,
  Enemies = 2,
  All = 3,
}

local ELEMENT_ENUM = {
  [0] = "None",
  [1] = "Fire",
  [2] = "Ice",
  [3] = "Wind",
  [4] = "Earth",
  [5] = "Thunder",
  [6] = "Water",
  [7] = "Light",
  [8] = "Dark",
}

local RESISTANCE_ENUM = {
  [0] = "None",
  [1] = "Weakness",
  [2] = "Halved",
  [3] = "Invalid",
  [4] = "Absorb",
  [5] = "Reflect",
}

local function makeNameSet(enumMap)
  local set = {}
  for _, name in pairs(enumMap) do
    set[name] = true
  end
  return set
end

local BTL_PHASE_NAMES = makeNameSet(BTL_PHASE_TYPE)
local BTL_CMD_TYPE_NAMES = makeNameSet(BTL_CMD_TYPE)
local BTL_CMD_STATE_NAMES = makeNameSet(BTL_CMD_STATE)
local TARGET_SELECT_NAMES = makeNameSet(TARGET_SELECT_STATE)
local ELEMENT_NAMES = makeNameSet(ELEMENT_ENUM)
local RESISTANCE_NAMES = makeNameSet(RESISTANCE_ENUM)

local function utf8Step(value, index)
  local byte = value:byte(index)
  if byte < 0x80 then return 1 end
  if byte < 0xE0 then return 2 end
  if byte < 0xF0 then return 3 end
  return 4
end

local function truncateUtf8(value, limit)
  local text = tostring(value)
  if limit < 1 then return "" end
  local count = 0
  local index = 1
  local previousEnd = 0
  while index <= #text do
    count = count + 1
    local nextIndex = index + utf8Step(text, index)
    if count == limit then
      if nextIndex > #text then return text end
      return text:sub(1, previousEnd) .. "…"
    end
    previousEnd = nextIndex - 1
    index = nextIndex
  end
  return text
end

local function recordErr(ctx, field, message)
  if type(ctx) == "table" and type(ctx.err) == "function" then
    pcall(ctx.err, field, tostring(message))
  end
end

local function isFiniteNumber(value)
  return type(value) == "number" and value == value and value ~= math.huge and value ~= -math.huge
end

local function toInt(value)
  if not isFiniteNumber(value) then return nil end
  local floored = math.floor(value)
  if math.tointeger then
    return math.tointeger(floored) or floored
  end
  return floored
end

local function isValidObject(obj)
  if obj == nil then return false, nil end
  local ok, valid = pcall(function() return obj:IsValid() end)
  if not ok then return false, tostring(valid) end
  return valid == true, nil
end

local function getCallable(obj, method)
  if obj == nil then return nil end
  local ok, fn = pcall(function() return obj[method] end)
  if not ok or fn == nil then return nil end
  local kind = type(fn)
  if kind == "function" or kind == "userdata" then return fn end
  return nil
end

local function unwrap(value)
  local getter = getCallable(value, "get")
  if getter ~= nil then
    local ok, inner = pcall(getter, value)
    if ok then return inner end
  end
  return value
end

local function readProp(obj, prop)
  return pcall(function() return obj[prop] end)
end

local function callMethod(obj, method)
  return pcall(function()
    local fn = obj[method]
    if fn == nil then error("缺少方法 " .. method) end
    return fn(obj)
  end)
end

local function readBoolProp(ctx, obj, prop, field)
  local ok, val = readProp(obj, prop)
  if not ok then
    recordErr(ctx, field, val)
    return json.null
  end
  if type(val) ~= "boolean" then
    recordErr(ctx, field, prop .. " 不是布尔值")
    return json.null
  end
  return val
end

local function readIntProp(ctx, obj, prop, field)
  local ok, val = readProp(obj, prop)
  if not ok then
    recordErr(ctx, field, val)
    return json.null
  end
  local num = toInt(val)
  if num == nil then
    recordErr(ctx, field, prop .. " 不是有效整数")
    return json.null
  end
  return num
end

local function decodeEnum(value, enumMap, nameSet)
  local num = toInt(value)
  if num ~= nil then
    return enumMap[num]
  end
  if type(value) == "string" then
    local short = value:match("::([^:]+)$") or value
    if nameSet[short] then return short end
  end
  return nil
end

local function readEnumProp(ctx, obj, prop, enumMap, nameSet, field)
  local ok, raw = readProp(obj, prop)
  if not ok then
    recordErr(ctx, field, raw)
    return json.null
  end
  local mapped = decodeEnum(raw, enumMap, nameSet)
  if mapped == nil then
    recordErr(ctx, field, prop .. " 枚举值无效：" .. tostring(raw))
    return json.null
  end
  return mapped
end

local function readText(value)
  if value == nil then return nil, "文本为空" end
  if type(value) == "string" then
    return truncateUtf8(value, TEXT_LIMIT), nil
  end
  local ok, text = callMethod(value, "ToString")
  if not ok then return nil, tostring(text) end
  if type(text) ~= "string" then return nil, "ToString 未返回字符串" end
  return truncateUtf8(text, TEXT_LIMIT), nil
end

local function shortClassName(obj)
  local okCls, cls = callMethod(obj, "GetClass")
  if not okCls then return nil, tostring(cls) end
  local validCls, validErr = isValidObject(cls)
  if validErr then return nil, validErr end
  if not validCls then return nil, "GetClass 返回无效类" end

  local fnameFn = getCallable(cls, "GetFName")
  if fnameFn ~= nil then
    local okFn, fname = pcall(fnameFn, cls)
    if okFn and fname ~= nil then
      if type(fname) == "string" and fname ~= "" then
        return fname, nil
      end
      local toStr = getCallable(fname, "ToString")
      if toStr ~= nil then
        local okStr, str = pcall(toStr, fname)
        if okStr and type(str) == "string" and str ~= "" then
          return str, nil
        end
      end
    end
  end

  local okFull, fullName = callMethod(cls, "GetFullName")
  if not okFull then return nil, tostring(fullName) end
  if type(fullName) ~= "string" or fullName == "" then return nil, "GetFullName 返回空值" end
  local short = fullName:match("([^.%s/\\]+)$")
  if not short or short == "" then return nil, "无法提取短类名：" .. fullName end
  return short, nil
end

-- Read TArray length and elements (supports UE4SS TArray userdata and plain Lua tables).
local function arrayLength(arr)
  if arr == nil then return nil, "数组为空" end
  local numFn = getCallable(arr, "GetArrayNum")
  if numFn ~= nil then
    local ok, n = pcall(numFn, arr)
    if not ok then return nil, tostring(n) end
    local count = toInt(n)
    if count == nil or count < 0 then return nil, "GetArrayNum 返回非法长度" end
    if count > MAX_ARRAY_LEN then return nil, "数组长度异常：" .. tostring(count) end
    return count, nil
  end
  local okLen, n = pcall(function() return #arr end)
  if not okLen then return nil, tostring(n) end
  local count = toInt(n)
  if count == nil or count < 0 then return nil, "数组长度非法" end
  if count > MAX_ARRAY_LEN then return nil, "数组长度异常：" .. tostring(count) end
  return count, nil
end

local function readArrayItems(arr)
  local len, lenErr = arrayLength(arr)
  if lenErr then return nil, lenErr end
  local items = {}
  for index = 1, len do
    local ok, item = pcall(function() return arr[index] end)
    if not ok then return nil, tostring(item) end
    items[index] = unwrap(item)
  end
  return items, nil
end

local function readIntArray(ctx, arr, field)
  local rawItems, err = readArrayItems(arr)
  if err then
    recordErr(ctx, field, err)
    return json.null
  end
  local out = {}
  for index = 1, #rawItems do
    local num = toInt(rawItems[index])
    if num == nil then
      recordErr(ctx, field, "数组元素不是有效整数（索引 " .. tostring(index) .. "）")
      return json.null
    end
    out[index] = num
  end
  return json.array(out)
end

-- Iterate UE4SS TMap (via ForEach + RemoteUnrealParam unwrap) or plain Lua table.
local function readMapEntries(mapObj)
  if mapObj == nil then return nil, "Map 为空" end
  local entries = {}
  local forEachFn = getCallable(mapObj, "ForEach")
  if forEachFn ~= nil then
    local ok, err = pcall(forEachFn, mapObj, function(key, val)
      entries[#entries + 1] = { unwrap(key), unwrap(val) }
    end)
    if not ok then return nil, tostring(err) end
    return entries, nil
  end
  if type(mapObj) == "table" then
    for key, val in pairs(mapObj) do
      entries[#entries + 1] = { key, val }
    end
    return entries, nil
  end
  return nil, "对象不支持 Map 遍历"
end

local function ensureCache(ctx)
  local cache = type(ctx) == "table" and type(ctx.cache) == "table" and ctx.cache or {}
  if type(cache.singletons) ~= "table" then
    cache.singletons = {}
  end
  local worldKey = type(ctx) == "table" and (ctx.world or ctx.map) or nil
  if worldKey ~= nil then
    if cache.worldKey ~= nil and cache.worldKey ~= worldKey then
      cache.singletons = {}
    end
    cache.worldKey = worldKey
  end
  return cache
end

local function clearBattleSingletons(cache)
  for index = 1, #BATTLE_SCOPED_SINGLETONS do
    cache.singletons[BATTLE_SCOPED_SINGLETONS[index]] = nil
  end
end

local function getSingleton(ctx, cache, className)
  local cached = cache.singletons[className]
  if cached ~= nil then
    local valid = isValidObject(cached)
    if valid then return cached, nil end
    cache.singletons[className] = nil
  end
  if type(ctx) ~= "table" or type(ctx.find) ~= "function" then
    return nil, "ctx.find 不可用"
  end
  local ok, obj = pcall(ctx.find, className)
  if not ok then
    return nil, tostring(obj)
  end
  local valid, validErr = isValidObject(obj)
  if validErr then
    return nil, validErr
  end
  if not valid then
    return nil, "找不到有效的 " .. className
  end
  cache.singletons[className] = obj
  return obj, nil
end

local function readActionKeyIds(ctx, listHolder, field)
  local okList, listWidget = readProp(listHolder, "WBP_List")
  if not okList then
    recordErr(ctx, field, listWidget)
    return json.null
  end
  local validList, validErr = isValidObject(listWidget)
  if validErr then
    recordErr(ctx, field, validErr)
    return json.null
  end
  if not validList then
    recordErr(ctx, field, "WBP_List 无效")
    return json.null
  end
  local okArr, actionKeys = readProp(listWidget, "ActionKeyList")
  if not okArr then
    recordErr(ctx, field, actionKeys)
    return json.null
  end
  local items, itemsErr = readArrayItems(actionKeys)
  if itemsErr then
    recordErr(ctx, field, itemsErr)
    return json.null
  end
  local ids = {}
  for index = 1, #items do
    local entry = items[index]
    local okId, rawId = readProp(entry, "ID")
    if not okId then
      recordErr(ctx, field, rawId)
      return json.null
    end
    local id = toInt(rawId)
    if id == nil then
      recordErr(ctx, field, "ActionKey.ID 不是有效整数")
      return json.null
    end
    ids[index] = id
  end
  return json.array(ids)
end

local function collectCmdTarget(ctx, vmCmd)
  local okSel, sel = readProp(vmCmd, "UnitSelectVM")
  if not okSel then
    recordErr(ctx, "battle.cmd.target", sel)
    return json.null
  end
  local validSel, validErr = isValidObject(sel)
  if validErr then
    recordErr(ctx, "battle.cmd.target", validErr)
    return json.null
  end
  if not validSel then
    recordErr(ctx, "battle.cmd.target", "UnitSelectVM 无效")
    return json.null
  end

  local skillId = json.null
  local isAll = json.null
  local okCmdVal, cmdVal = readProp(sel, "CmdValue")
  if not okCmdVal then
    recordErr(ctx, "battle.cmd.target.skill_id", cmdVal)
  elseif cmdVal == nil then
    recordErr(ctx, "battle.cmd.target.skill_id", "CmdValue 为空")
  else
    local okKey, actionKey = readProp(cmdVal, "ActionKey")
    if not okKey then
      recordErr(ctx, "battle.cmd.target.skill_id", actionKey)
    elseif actionKey == nil then
      recordErr(ctx, "battle.cmd.target.skill_id", "CmdValue.ActionKey 为空")
    else
      skillId = readIntProp(ctx, actionKey, "ID", "battle.cmd.target.skill_id")
    end
    isAll = readBoolProp(ctx, cmdVal, "IsAll", "battle.cmd.target.is_all")
  end

  local focusUid = readIntProp(ctx, sel, "FocusBaseUniqueId", "battle.cmd.target.focus_uid")

  local targetUids = json.null
  local okTgt, rawTgt = readProp(sel, "targetUniqueIdList")
  if not okTgt then
    recordErr(ctx, "battle.cmd.target.target_uids", rawTgt)
  else
    targetUids = readIntArray(ctx, rawTgt, "battle.cmd.target.target_uids")
  end

  local allTargetUids = json.null
  local okAll, rawAll = readProp(sel, "AllTargetUniqueIdList")
  if not okAll then
    recordErr(ctx, "battle.cmd.target.all_target_uids", rawAll)
  else
    allTargetUids = readIntArray(ctx, rawAll, "battle.cmd.target.all_target_uids")
  end

  local canAoe = readBoolProp(ctx, sel, "bCanBeAoE", "battle.cmd.target.can_aoe")

  return {
    skill_id = skillId,
    focus_uid = focusUid,
    target_uids = targetUids,
    all_target_uids = allTargetUids,
    can_aoe = canAoe,
    is_all = isAll,
  }
end

local function collectCmd(ctx, cache, uiBattle)
  local vmCmd, vmCmdErr = getSingleton(ctx, cache, "VM_BtlCmd_C")

  local cmdMgr = nil
  local cmdMgrErr = nil
  local okMgr, rawMgr = readProp(uiBattle, "BtlCmdManager")
  if not okMgr then
    cmdMgrErr = tostring(rawMgr)
  else
    local validMgr, vErr = isValidObject(rawMgr)
    if vErr then
      cmdMgrErr = vErr
    elseif validMgr then
      cmdMgr = rawMgr
    else
      cmdMgrErr = "BtlCmdManager 无效"
    end
  end

  if vmCmd == nil and cmdMgr == nil then
    recordErr(ctx, "battle.cmd", (vmCmdErr or "") .. "; " .. (cmdMgrErr or ""))
    return json.null
  end

  local display = json.null
  local ancestors = json.null
  local focusIndex = json.null
  local focusActionId = json.null

  if vmCmd ~= nil then
    display = readBoolProp(ctx, vmCmd, "bDisplayCmd", "battle.cmd.display")
    local okAnc, rawAnc = readProp(vmCmd, "AncestorIdList")
    if not okAnc then
      recordErr(ctx, "battle.cmd.ancestors", rawAnc)
    else
      ancestors = readIntArray(ctx, rawAnc, "battle.cmd.ancestors")
    end

    local okFocus, focusData = readProp(vmCmd, "FocusCmdData")
    if not okFocus then
      recordErr(ctx, "battle.cmd.focus", focusData)
    elseif focusData == nil then
      recordErr(ctx, "battle.cmd.focus", "FocusCmdData 为空")
    else
      focusIndex = readIntProp(ctx, focusData, "EntryIndex", "battle.cmd.focus_index")
      local okVal, val = readProp(focusData, "Value")
      if not okVal then
        recordErr(ctx, "battle.cmd.focus_action_id", val)
      elseif val == nil then
        recordErr(ctx, "battle.cmd.focus_action_id", "FocusCmdData.Value 为空")
      else
        local okKey, actionKey = readProp(val, "ActionKey")
        if not okKey then
          recordErr(ctx, "battle.cmd.focus_action_id", actionKey)
        elseif actionKey == nil then
          recordErr(ctx, "battle.cmd.focus_action_id", "FocusCmdData.Value.ActionKey 为空")
        else
          focusActionId = readIntProp(ctx, actionKey, "ID", "battle.cmd.focus_action_id")
        end
      end
    end
  else
    recordErr(ctx, "battle.cmd.vm", vmCmdErr or "找不到有效的 VM_BtlCmd_C")
  end

  local cmdType = json.null
  local cmdState = json.null
  if cmdMgr ~= nil then
    cmdType = readEnumProp(ctx, cmdMgr, "CmdType", BTL_CMD_TYPE, BTL_CMD_TYPE_NAMES, "battle.cmd.type")
    cmdState = readEnumProp(ctx, cmdMgr, "CmdState", BTL_CMD_STATE, BTL_CMD_STATE_NAMES, "battle.cmd.state")
  else
    recordErr(ctx, "battle.cmd.manager", cmdMgrErr or "BtlCmdManager 无效")
  end

  local primaryIds = json.null
  local secondaryIds = json.null
  local target = json.null

  if display == true and cmdState == "List" then
    local battleLayout, layoutErr = getSingleton(ctx, cache, "WBP_BattleLayout_C")
    if battleLayout == nil then
      recordErr(ctx, "battle.cmd.primary_ids", layoutErr or "找不到有效的 WBP_BattleLayout_C")
    else
      local okCmdLayout, cmdLayout = readProp(battleLayout, "WBP_BtlCmdLayout")
      local validCmdLayout, validCmdLayoutErr = false, nil
      if okCmdLayout then
        validCmdLayout, validCmdLayoutErr = isValidObject(cmdLayout)
      end
      if not okCmdLayout then
        recordErr(ctx, "battle.cmd.primary_ids", cmdLayout)
      elseif validCmdLayoutErr or not validCmdLayout then
        recordErr(ctx, "battle.cmd.primary_ids", validCmdLayoutErr or "WBP_BtlCmdLayout 无效")
      else
        local okContent, content = readProp(cmdLayout, "ContentWidget")
        local validContent, validContentErr = false, nil
        if okContent then
          validContent, validContentErr = isValidObject(content)
        end
        if not okContent then
          recordErr(ctx, "battle.cmd.primary_ids", content)
        elseif validContentErr or not validContent then
          recordErr(ctx, "battle.cmd.primary_ids", validContentErr or "ContentWidget 无效")
        else
          local okPrimary, primaryWidget = readProp(content, "WBP_BtlCmd_PrimaryList")
          if not okPrimary then
            recordErr(ctx, "battle.cmd.primary_ids", primaryWidget)
          else
            local validPrimary, validPrimaryErr = isValidObject(primaryWidget)
            if validPrimaryErr or not validPrimary then
              recordErr(ctx, "battle.cmd.primary_ids", validPrimaryErr or "WBP_BtlCmd_PrimaryList 无效")
            else
              primaryIds = readActionKeyIds(ctx, primaryWidget, "battle.cmd.primary_ids")
            end
          end

          if type(ancestors) == "table" and not json.isNull(ancestors) and #ancestors > 0 then
            local okSecondary, secondaryWidget = readProp(content, "WBP_BtlCmd_SecondaryList")
            if not okSecondary then
              recordErr(ctx, "battle.cmd.secondary_ids", secondaryWidget)
            else
              local validSecondary, validSecondaryErr = isValidObject(secondaryWidget)
              if validSecondaryErr or not validSecondary then
                recordErr(
                  ctx,
                  "battle.cmd.secondary_ids",
                  validSecondaryErr or "WBP_BtlCmd_SecondaryList 无效"
                )
              else
                secondaryIds = readActionKeyIds(ctx, secondaryWidget, "battle.cmd.secondary_ids")
              end
            end
          end
        end
      end
    end
  elseif display == true and cmdState == "Target" and vmCmd ~= nil then
    target = collectCmdTarget(ctx, vmCmd)
  end

  return {
    display = display,
    type = cmdType,
    state = cmdState,
    ancestors = ancestors,
    focus_index = focusIndex,
    focus_action_id = focusActionId,
    primary_ids = primaryIds,
    secondary_ids = secondaryIds,
    target = target,
  }
end

local function readPointPair(ctx, vm, vmProp, param, curGetter, maxGetter, curField, maxField)
  local curVal = nil
  local maxVal = nil
  local primaryCurErr = nil
  local primaryMaxErr = nil

  local okPointVm, pointVm = readProp(vm, vmProp)
  if okPointVm then
    local validVm, vErr = isValidObject(pointVm)
    if validVm then
      local okCur, rawCur = readProp(pointVm, "CurrentPoint")
      if okCur then
        curVal = toInt(rawCur)
        if curVal == nil then primaryCurErr = "CurrentPoint 不是有效整数" end
      else
        primaryCurErr = tostring(rawCur)
      end

      local okMax, rawMax = readProp(pointVm, "MaxPoint")
      if okMax then
        maxVal = toInt(rawMax)
        if maxVal == nil then primaryMaxErr = "MaxPoint 不是有效整数" end
      else
        primaryMaxErr = tostring(rawMax)
      end
    else
      primaryCurErr = vErr or (vmProp .. " 无效")
      primaryMaxErr = primaryCurErr
    end
  else
    primaryCurErr = tostring(pointVm)
    primaryMaxErr = primaryCurErr
  end

  if curVal == nil and param ~= nil and curGetter ~= nil then
    local okFallback, rawFallback = callMethod(param, curGetter)
    if okFallback then
      curVal = toInt(rawFallback)
    elseif primaryCurErr == nil then
      primaryCurErr = tostring(rawFallback)
    end
  end

  if maxVal == nil and param ~= nil and maxGetter ~= nil then
    local okFallback, rawFallback = callMethod(param, maxGetter)
    if okFallback then
      maxVal = toInt(rawFallback)
    elseif primaryMaxErr == nil then
      primaryMaxErr = tostring(rawFallback)
    end
  end

  if curVal == nil then
    recordErr(ctx, curField, primaryCurErr or "无法读取当前值")
    curVal = json.null
  end
  if maxVal == nil then
    recordErr(ctx, maxField, primaryMaxErr or "无法读取最大值")
    maxVal = json.null
  end
  return curVal, maxVal
end

local function decodeRelation(raw)
  local num = toInt(raw)
  if num ~= nil then return num end
  if type(raw) == "string" then
    local short = raw:match("::([^:]+)$") or raw
    return RELATION_BY_NAME[short]
  end
  return nil
end

local function readUnitIsBreak(ctx, param, field)
  if param == nil then
    recordErr(ctx, field, "BtlUnitParam 无效")
    return json.null
  end
  local okCall, val = callMethod(param, "IsBreak")
  if okCall and type(val) == "boolean" then
    return val
  end
  local primaryErr = okCall and "IsBreak 未返回布尔值" or tostring(val)

  -- Fallback: inspect m_ContEffectList for a valid CPP_ContFxBreak instance.
  local okList, effectList = readProp(param, "m_ContEffectList")
  if okList and effectList ~= nil then
    local items, itemsErr = readArrayItems(effectList)
    if not itemsErr then
      for index = 1, #items do
        local effect = items[index]
        if isValidObject(effect) then
          local clsName = shortClassName(effect)
          if clsName == "CPP_ContFxBreak" then
            return true
          end
        end
      end
      return false
    end
  end

  recordErr(ctx, field, primaryErr)
  return json.null
end

local function readUnitWeaknesses(ctx, vm, prefix)
  local field = prefix .. ".weaknesses"
  local okList, rawList = readProp(vm, "ElemResistDataList")
  if not okList then
    recordErr(ctx, field, rawList)
    return json.null
  end
  local items, itemsErr = readArrayItems(rawList)
  if itemsErr then
    recordErr(ctx, field, itemsErr)
    return json.null
  end
  local out = {}
  for index = 1, #items do
    local entry = items[index]
    local okEl, rawEl = readProp(entry, "element")
    local okRes, rawRes = readProp(entry, "resistance")
    local okAn, rawAn = readProp(entry, "bIsAnalyzed")
    if not okEl or not okRes or not okAn then
      recordErr(
        ctx,
        field,
        tostring(not okEl and rawEl or (not okRes and rawRes or rawAn))
      )
    else
      local element = decodeEnum(rawEl, ELEMENT_ENUM, ELEMENT_NAMES)
      local resistance = decodeEnum(rawRes, RESISTANCE_ENUM, RESISTANCE_NAMES)
      if element == nil or resistance == nil then
        recordErr(
          ctx,
          field,
          "弱点枚举无效：element=" .. tostring(rawEl) .. " resistance=" .. tostring(rawRes)
        )
      elseif type(rawAn) ~= "boolean" then
        recordErr(ctx, field, "bIsAnalyzed 不是布尔值")
      elseif element ~= "None" and resistance ~= "None" then
        out[#out + 1] = {
          element = element,
          resistance = resistance,
          analyzed = rawAn,
        }
      end
    end
  end
  return json.array(out)
end

local function readScheduledSkillId(ctx, param, isDead, prefix)
  local field = prefix .. ".scheduled_skill_id"
  if param == nil then
    if isDead == true then return json.null end
    recordErr(ctx, field, "BtlUnitParam 无效")
    return json.null
  end
  local okOwner, owner = readProp(param, "m_pOwnerBtlUnit")
  if not okOwner then
    recordErr(ctx, field, owner)
    return json.null
  end
  local validOwner, ownerErr = isValidObject(owner)
  if ownerErr then
    recordErr(ctx, field, ownerErr)
    return json.null
  end
  if not validOwner then
    if isDead == true then return json.null end
    recordErr(ctx, field, "m_pOwnerBtlUnit 无效")
    return json.null
  end
  local okAi, ai = readProp(owner, "m_pAI")
  if not okAi then
    recordErr(ctx, field, ai)
    return json.null
  end
  local validAi, aiErr = isValidObject(ai)
  if aiErr then
    recordErr(ctx, field, aiErr)
    return json.null
  end
  if not validAi then
    if isDead == true then return json.null end
    recordErr(ctx, field, "m_pAI 无效")
    return json.null
  end
  return readIntProp(ctx, ai, "m_ScheduledSkillId", field)
end

local function collectUnit(ctx, vm, mapKey, riskUidSet)
  local uid = readIntProp(ctx, vm, "UniqueID", "battle.units.uid")
  local keyTag = not json.isNull(uid) and tostring(uid) or tostring(mapKey)
  local prefix = "battle.units[" .. keyTag .. "]"

  local unitId = readIntProp(ctx, vm, "UnitId", prefix .. ".unit_id")

  local name = json.null
  local okName, rawName = readProp(vm, "ShortName")
  if not okName then
    recordErr(ctx, prefix .. ".name", rawName)
  else
    local text, textErr = readText(rawName)
    if textErr then
      recordErr(ctx, prefix .. ".name", textErr)
    else
      name = text
    end
  end

  local param = nil
  local okParam, rawParam = readProp(vm, "BtlUnitParam")
  if okParam and isValidObject(rawParam) then
    param = rawParam
  elseif not okParam then
    recordErr(ctx, prefix .. ".param", rawParam)
  end

  -- relation: param:GetRelation() fallback vm:GetRelation()
  local relation = json.null
  do
    local relVal = nil
    local relErr = nil
    if param ~= nil then
      local okRel, rawRel = callMethod(param, "GetRelation")
      if okRel then
        relVal = decodeRelation(rawRel)
        if relVal == nil then relErr = "GetRelation 返回无效值：" .. tostring(rawRel) end
      else
        relErr = tostring(rawRel)
      end
    end
    if relVal == nil then
      local okVmRel, rawVmRel = callMethod(vm, "GetRelation")
      if okVmRel then
        relVal = decodeRelation(rawVmRel)
        if relVal == nil and relErr == nil then
          relErr = "VM GetRelation 返回无效值：" .. tostring(rawVmRel)
        end
      elseif relErr == nil then
        relErr = tostring(rawVmRel)
      end
    end
    if relVal ~= nil then
      relation = relVal
    else
      recordErr(ctx, prefix .. ".relation", relErr or "无法读取阵营")
    end
  end

  -- is_boss: param:IsBoss() fallback vm:GetIsBoss()
  local isBoss = json.null
  do
    local bossVal = nil
    local bossErr = nil
    if param ~= nil then
      local okBoss, rawBoss = callMethod(param, "IsBoss")
      if okBoss and type(rawBoss) == "boolean" then
        bossVal = rawBoss
      else
        bossErr = okBoss and "IsBoss 未返回布尔值" or tostring(rawBoss)
      end
    end
    if bossVal == nil then
      local okVmBoss, rawVmBoss = callMethod(vm, "GetIsBoss")
      if okVmBoss and type(rawVmBoss) == "boolean" then
        bossVal = rawVmBoss
      elseif bossErr == nil then
        bossErr = okVmBoss and "GetIsBoss 未返回布尔值" or tostring(rawVmBoss)
      end
    end
    if bossVal ~= nil then
      isBoss = bossVal
    else
      recordErr(ctx, prefix .. ".is_boss", bossErr or "无法读取 Boss 标志")
    end
  end

  -- is_dead: param:IsDead() fallback (not vm.bIsVisible)
  local isDead = json.null
  do
    local deadVal = nil
    local deadErr = nil
    if param ~= nil then
      local okDead, rawDead = callMethod(param, "IsDead")
      if okDead and type(rawDead) == "boolean" then
        deadVal = rawDead
      else
        deadErr = okDead and "IsDead 未返回布尔值" or tostring(rawDead)
      end
    end
    if deadVal == nil then
      local okVis, vis = readProp(vm, "bIsVisible")
      if okVis and type(vis) == "boolean" then
        deadVal = not vis
      elseif deadErr == nil then
        deadErr = okVis and "bIsVisible 不是布尔值" or tostring(vis)
      end
    end
    if deadVal ~= nil then
      isDead = deadVal
    else
      recordErr(ctx, prefix .. ".is_dead", deadErr or "无法读取存活状态")
    end
  end

  local hp, maxHp = readPointPair(
    ctx,
    vm,
    "HitPointVM",
    param,
    "GetHitPoint",
    "GetMaxHitPoint",
    prefix .. ".hp",
    prefix .. ".max_hp"
  )
  local mp, maxMp = readPointPair(
    ctx,
    vm,
    "MagicPointVM",
    param,
    "GetMagicPoint",
    "GetMaxMagicPoint",
    prefix .. ".mp",
    prefix .. ".max_mp"
  )
  local bp, maxBp = readPointPair(
    ctx,
    vm,
    "BreakPointVM",
    param,
    "GetBreakPoint",
    "GetMaxBreakPoint",
    prefix .. ".bp",
    prefix .. ".max_bp"
  )

  local lb = json.null
  local lbMax = json.null
  local lbLevel = json.null
  local okLbVm, lbVm = readProp(vm, "LimitPointVM")
  if not okLbVm then
    recordErr(ctx, prefix .. ".lb", lbVm)
  else
    local validLb, lbErr = isValidObject(lbVm)
    if lbErr or not validLb then
      recordErr(ctx, prefix .. ".lb", lbErr or "LimitPointVM 无效")
    else
      lb = readIntProp(ctx, lbVm, "CurrentPoint", prefix .. ".lb")
      lbMax = readIntProp(ctx, lbVm, "MaxPoint", prefix .. ".lb_max")
      lbLevel = readIntProp(ctx, lbVm, "CurrentLevel", prefix .. ".lb_level")
    end
  end

  local isBreak = readUnitIsBreak(ctx, param, prefix .. ".is_break")

  -- is_risky: vm:IsScheduledHighRiskSkill() or (uid in risk_uids)
  local isRisky = json.null
  do
    local inRiskSet = riskUidSet ~= nil and not json.isNull(uid) and riskUidSet[uid] == true
    local okRisk, rawRisk = callMethod(vm, "IsScheduledHighRiskSkill")
    if okRisk and type(rawRisk) == "boolean" then
      isRisky = rawRisk or inRiskSet
    elseif riskUidSet ~= nil and not json.isNull(uid) then
      isRisky = inRiskSet
    else
      recordErr(
        ctx,
        prefix .. ".is_risky",
        okRisk and "IsScheduledHighRiskSkill 未返回布尔值" or tostring(rawRisk)
      )
    end
  end

  local isEnemy = relation == 2
  local scheduledSkillId = isEnemy and readScheduledSkillId(ctx, param, isDead, prefix) or json.null

  local inAction = json.null
  local inExPhase = json.null
  local okActVm, actVm = readProp(vm, "ActionVM")
  if not okActVm then
    recordErr(ctx, prefix .. ".action", actVm)
  else
    local validAct, actErr = isValidObject(actVm)
    if actErr or not validAct then
      recordErr(ctx, prefix .. ".action", actErr or "ActionVM 无效")
    else
      inAction = readBoolProp(ctx, actVm, "bIsInAction", prefix .. ".in_action")
      inExPhase = readBoolProp(ctx, actVm, "bIsInExPhase", prefix .. ".in_ex_phase")
    end
  end

  local targetState = readEnumProp(
    ctx,
    vm,
    "TargetSelectState",
    TARGET_SELECT_STATE,
    TARGET_SELECT_NAMES,
    prefix .. ".target_state"
  )

  local weaknesses = isEnemy and readUnitWeaknesses(ctx, vm, prefix) or json.null

  return {
    uid = uid,
    unit_id = unitId,
    name = name,
    relation = relation,
    is_boss = isBoss,
    is_dead = isDead,
    hp = hp,
    max_hp = maxHp,
    mp = mp,
    max_mp = maxMp,
    bp = bp,
    max_bp = maxBp,
    lb = lb,
    lb_max = lbMax,
    lb_level = lbLevel,
    is_break = isBreak,
    is_risky = isRisky,
    scheduled_skill_id = scheduledSkillId,
    in_action = inAction,
    in_ex_phase = inExPhase,
    target_state = targetState,
    weaknesses = weaknesses,
  }
end

local function collectUnits(ctx, uiBattle, riskUidSet)
  local okMgr, unitMgr = readProp(uiBattle, "BtlUnitManager")
  if not okMgr then
    recordErr(ctx, "battle.units", unitMgr)
    return json.null
  end
  local validMgr, mgrErr = isValidObject(unitMgr)
  if mgrErr or not validMgr then
    recordErr(ctx, "battle.units", mgrErr or "BtlUnitManager 无效")
    return json.null
  end

  local okMap, vmMap = readProp(unitMgr, "UnitParamVMMap")
  if not okMap then
    recordErr(ctx, "battle.units", vmMap)
    return json.null
  end
  local entries, entriesErr = readMapEntries(vmMap)
  if entriesErr then
    recordErr(ctx, "battle.units", entriesErr)
    return json.null
  end

  local collected = {}
  for index = 1, #entries do
    local mapKey = entries[index][1]
    local vm = entries[index][2]
    local validVm, vmErr = isValidObject(vm)
    if not validVm then
      recordErr(
        ctx,
        "battle.units[" .. tostring(mapKey) .. "]",
        vmErr or "CPP_VM_BtlUnitParam 无效"
      )
    else
      local unit = collectUnit(ctx, vm, mapKey, riskUidSet)
      local sortUid = type(unit.uid) == "number" and unit.uid or math.huge
      collected[#collected + 1] = { sortUid = sortUid, order = index, unit = unit }
    end
  end

  table.sort(collected, function(left, right)
    if left.sortUid ~= right.sortUid then
      return left.sortUid < right.sortUid
    end
    return left.order < right.order
  end)

  local units = {}
  for index = 1, #collected do
    units[index] = collected[index].unit
  end
  return json.array(units)
end

local function collectResult(ctx, cache, uiBattle, taskClass)
  if taskClass ~= "CPP_BtlTaskWin" then
    return json.null
  end

  local okWidget, resWidget = readProp(uiBattle, "ResultWidget")
  if not okWidget then
    recordErr(ctx, "battle.result", resWidget)
    return json.null
  end
  local validWidget, validErr = isValidObject(resWidget)
  if validErr then
    recordErr(ctx, "battle.result", validErr)
    return json.null
  end
  if not validWidget then
    return json.null
  end

  -- ResultWidget stays pooled across battles; IsFinishPage == false proves it is active.
  local okFinish, isFinish = readProp(resWidget, "IsFinishPage")
  if not okFinish then
    recordErr(ctx, "battle.result", isFinish)
    return json.null
  end
  if type(isFinish) ~= "boolean" then
    recordErr(ctx, "battle.result", "IsFinishPage 不是布尔值")
    return json.null
  end
  if isFinish then
    return json.null
  end

  local pageIndex = json.null
  local okLayout, pageLayout = readProp(resWidget, "WBP_BtlResultPage_Layout")
  if not okLayout then
    recordErr(ctx, "battle.result.page_index", pageLayout)
  else
    local validLayout, layoutErr = isValidObject(pageLayout)
    if layoutErr or not validLayout then
      recordErr(ctx, "battle.result.page_index", layoutErr or "WBP_BtlResultPage_Layout 无效")
    else
      pageIndex = readIntProp(ctx, pageLayout, "CurrentPageIndex", "battle.result.page_index")
    end
  end

  local totalExp = json.null
  local totalMoney = json.null
  local totalAp = json.null
  local enemyIds = json.null
  local items = json.null

  local vmRes, vmResErr = getSingleton(ctx, cache, "CPP_VM_BattleResult")
  if vmRes == nil then
    recordErr(ctx, "battle.result.data", vmResErr or "找不到有效的 CPP_VM_BattleResult")
  else
    local okData, data = readProp(vmRes, "BtlResultData")
    if not okData then
      recordErr(ctx, "battle.result.data", data)
    elseif data == nil then
      recordErr(ctx, "battle.result.data", "BtlResultData 为空")
    else
      totalExp = readIntProp(ctx, data, "totalExperience", "battle.result.total_exp")
      totalMoney = readIntProp(ctx, data, "totalMoney", "battle.result.total_money")
      totalAp = readIntProp(ctx, data, "totalAp", "battle.result.total_ap")

      local okEnemies, rawEnemies = readProp(data, "enemyDataList")
      if not okEnemies then
        recordErr(ctx, "battle.result.enemy_ids", rawEnemies)
      else
        enemyIds = readIntArray(ctx, rawEnemies, "battle.result.enemy_ids")
      end

      local okItems, rawItems = readProp(data, "itemDataList")
      if not okItems then
        recordErr(ctx, "battle.result.items", rawItems)
      else
        local itemEntries, itemEntriesErr = readArrayItems(rawItems)
        if itemEntriesErr then
          recordErr(ctx, "battle.result.items", itemEntriesErr)
        else
          local parsedItems = {}
          local itemFailed = false
          for index = 1, #itemEntries do
            local entry = itemEntries[index]
            local okId, rawId = readProp(entry, "ItemId")
            local okNum, rawNum = readProp(entry, "Num")
            local id = okId and toInt(rawId) or nil
            local num = okNum and toInt(rawNum) or nil
            if id == nil or num == nil then
              recordErr(
                ctx,
                "battle.result.items",
                not okId and rawId or (not okNum and rawNum or "掉落道具字段不是有效整数")
              )
              itemFailed = true
              break
            end
            parsedItems[index] = { item_id = id, num = num }
          end
          if not itemFailed then
            items = json.array(parsedItems)
          end
        end
      end
    end
  end

  return {
    page_index = pageIndex,
    total_exp = totalExp,
    total_money = totalMoney,
    total_ap = totalAp,
    enemy_ids = enemyIds,
    items = items,
  }
end

function M.collect(ctx)
  local cache = ensureCache(ctx)

  -- 1. in_battle (resident)
  local inBattle = json.null
  local uiBattle, uiBattleErr = getSingleton(ctx, cache, "CPP_UI_BattleSubsystem")
  if uiBattle == nil then
    recordErr(ctx, "battle.in_battle", uiBattleErr or "找不到有效的 CPP_UI_BattleSubsystem")
  else
    inBattle = readBoolProp(ctx, uiBattle, "bDuringBattle", "battle.in_battle")
  end

  -- 2. task_class (resident)
  local taskClass = json.null
  local taskMgr, taskMgrErr = getSingleton(ctx, cache, "CPP_TaskManager")
  if taskMgr == nil then
    recordErr(ctx, "battle.task_class", taskMgrErr or "找不到有效的 CPP_TaskManager")
  else
    local okTask, task = readProp(taskMgr, "m_pTask")
    if not okTask then
      recordErr(ctx, "battle.task_class", task)
    else
      local validTask, validErr = isValidObject(task)
      if validErr then
        recordErr(ctx, "battle.task_class", validErr)
      elseif validTask then
        local clsName, clsErr = shortClassName(task)
        if clsName then
          taskClass = clsName
        else
          recordErr(ctx, "battle.task_class", clsErr or "无法获取任务类名")
        end
      end
    end
  end

  -- 3. in_talk_event (resident)
  local inTalkEvent = json.null
  local talkSub, talkSubErr = getSingleton(ctx, cache, "CPP_TalkEventSubsystem")
  if talkSub == nil then
    recordErr(ctx, "battle.in_talk_event", talkSubErr or "找不到有效的 CPP_TalkEventSubsystem")
  else
    local okEv, evMgr = readProp(talkSub, "m_pEventManager")
    if not okEv then
      recordErr(ctx, "battle.in_talk_event", evMgr)
    else
      local validEv, validErr = isValidObject(evMgr)
      if validErr then
        recordErr(ctx, "battle.in_talk_event", validErr)
      else
        inTalkEvent = validEv
      end
    end
  end

  -- 4. in_tutorial (resident)
  local inTutorial = json.null
  local tutSub, tutSubErr = getSingleton(ctx, cache, "CPP_TutorialSubsystem")
  if tutSub == nil then
    recordErr(ctx, "battle.in_tutorial", tutSubErr or "找不到有效的 CPP_TutorialSubsystem")
  else
    local okTut, tutWidget = readProp(tutSub, "m_TutorialWidget")
    if not okTut then
      recordErr(ctx, "battle.in_tutorial", tutWidget)
    else
      local validTut, validErr = isValidObject(tutWidget)
      if validErr then
        recordErr(ctx, "battle.in_tutorial", validErr)
      else
        inTutorial = validTut
      end
    end
  end

  -- Clear battle-scoped singletons when entering or leaving a battle so fresh instances are bound.
  local activeBattle = inBattle == true
  if cache.wasInBattle ~= nil and cache.wasInBattle ~= activeBattle then
    clearBattleSingletons(cache)
  end
  cache.wasInBattle = activeBattle

  if not activeBattle then
    return {
      in_battle = inBattle,
      task_class = taskClass,
      in_talk_event = inTalkEvent,
      in_tutorial = inTutorial,
      phase = json.null,
      is_event = json.null,
      is_ui_visible = json.null,
      working_job_count = json.null,
      current_actor_uid = json.null,
      timeline = json.null,
      risk_uids = json.null,
      cmd = json.null,
      units = json.null,
      result = json.null,
    }
  end

  -- 5. phase
  local phase = readEnumProp(
    ctx,
    uiBattle,
    "BtlPhaseType",
    BTL_PHASE_TYPE,
    BTL_PHASE_NAMES,
    "battle.phase"
  )

  -- 6-7. is_event & is_ui_visible
  local isEvent = json.null
  local isUiVisible = json.null
  local vmBattle, vmBattleErr = getSingleton(ctx, cache, "CPP_VM_Battle")
  if vmBattle == nil then
    recordErr(ctx, "battle.vm_battle", vmBattleErr or "找不到有效的 CPP_VM_Battle")
  else
    isEvent = readBoolProp(ctx, vmBattle, "bIsEvent", "battle.is_event")
    isUiVisible = readBoolProp(ctx, vmBattle, "bIsDisplayBattleWidget", "battle.is_ui_visible")
  end

  -- 8. working_job_count
  local workingJobCount = json.null
  local btlMgr, btlMgrErr = getSingleton(ctx, cache, "CPP_BtlManager")
  if btlMgr == nil then
    recordErr(ctx, "battle.working_job_count", btlMgrErr or "找不到有效的 CPP_BtlManager")
  else
    local okJobMgr, jobMgr = readProp(btlMgr, "m_pBattleJobManager")
    if not okJobMgr then
      recordErr(ctx, "battle.working_job_count", jobMgr)
    else
      local validJobMgr, validErr = isValidObject(jobMgr)
      if validErr or not validJobMgr then
        recordErr(ctx, "battle.working_job_count", validErr or "m_pBattleJobManager 无效")
      else
        local okW, workingList = readProp(jobMgr, "m_WorkingJobList")
        local okQ, waitingList = readProp(jobMgr, "m_WaitingJobList")
        if not okW then
          recordErr(ctx, "battle.working_job_count", workingList)
        elseif not okQ then
          recordErr(ctx, "battle.working_job_count", waitingList)
        else
          local workingLen, workingErr = arrayLength(workingList)
          local waitingLen, waitingErr = arrayLength(waitingList)
          if workingErr then
            recordErr(ctx, "battle.working_job_count", workingErr)
          elseif waitingErr then
            recordErr(ctx, "battle.working_job_count", waitingErr)
          else
            workingJobCount = workingLen + waitingLen
          end
        end
      end
    end
  end

  -- 9. current_actor_uid
  local currentActorUid = readIntProp(
    ctx,
    uiBattle,
    "CurrentActionUnitUniqueId",
    "battle.current_actor_uid"
  )

  -- 10-11. timeline & risk_uids
  local timeline = json.null
  local riskUids = json.null
  local riskUidSet = nil
  local vmTimeline, vmTimelineErr = getSingleton(ctx, cache, "VM_Timeline_C")
  if vmTimeline == nil then
    recordErr(ctx, "battle.timeline", vmTimelineErr or "找不到有效的 VM_Timeline_C")
  else
    local okUni, unified = readProp(vmTimeline, "TimelineUnified")
    if not okUni then
      recordErr(ctx, "battle.timeline", unified)
    elseif unified == nil then
      recordErr(ctx, "battle.timeline", "TimelineUnified 为空")
    else
      local okIds, rawIds = readProp(unified, "UnifiedIds")
      if not okIds then
        recordErr(ctx, "battle.timeline", rawIds)
      else
        timeline = readIntArray(ctx, rawIds, "battle.timeline")
      end
    end

    local okRisk, rawRisk = readProp(vmTimeline, "RiskUnitUniqueIdList")
    if not okRisk then
      recordErr(ctx, "battle.risk_uids", rawRisk)
    else
      riskUids = readIntArray(ctx, rawRisk, "battle.risk_uids")
      if type(riskUids) == "table" and not json.isNull(riskUids) then
        riskUidSet = {}
        for index = 1, #riskUids do
          riskUidSet[riskUids[index]] = true
        end
      end
    end
  end

  -- 12. cmd
  local cmd = collectCmd(ctx, cache, uiBattle)

  -- 13. units
  local units = collectUnits(ctx, uiBattle, riskUidSet)

  -- 14. result
  local result = collectResult(ctx, cache, uiBattle, taskClass)

  return {
    in_battle = inBattle,
    task_class = taskClass,
    in_talk_event = inTalkEvent,
    in_tutorial = inTutorial,
    phase = phase,
    is_event = isEvent,
    is_ui_visible = isUiVisible,
    working_job_count = workingJobCount,
    current_actor_uid = currentActorUid,
    timeline = timeline,
    risk_uids = riskUids,
    cmd = cmd,
    units = units,
    result = result,
  }
end

return M
