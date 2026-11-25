-- FfrBridge v1 topic "party" (task 11-party).
-- Read-only: only reads properties and calls pure getter functions; never mutates game state or sends input.
--
-- Contract (implemented by state.lua, task 11-field):
--   M.collect(ctx) -> table|nil, called every tick on the game thread inside pcall.
--   ctx.find(className) -> object|nil      (FindFirstOf with IsValid check, cached per tick)
--   ctx.findAll(className) -> list         (FindAllOf, invalid objects dropped)
--   ctx.err(field, message)                (record one error into data.errors; never throw)
--   ctx.cache                              (table private to this topic, kept across ticks)
--   ctx.clockMs, ctx.tick                  (monotonic ms since load, tick counter)
--   M.commands[name] = function(ctx, args) -> result|nil, errorMessage   (on-demand, read-only)
--
-- Lookup cost per tick (500 ms cadence):
--   - Cold tick (first tick, or after invalidation / map change): 4 ctx.find calls
--     (CPP_MuchaSaveDataManager, CPP_ManualViewModelManager [fallback CPP_UnitVMManager],
--     CPP_ScenarioSubsystem, CPP_QuestSubsystem), 0 ctx.findAll calls.
--   - Warm tick (cached singletons valid): 0 ctx.find calls, 0 ctx.findAll calls.
--   - Low-frequency scans (every 4 ticks / 2 s, or immediately on dirty trigger):
--     party_stats (dirty on party/level/exp/max_hp/max_mp/equipment change),
--     inventory summary (dirty on array length or money change),
--     event_flags summary (dirty on progress change).
--
-- Fields under data.party (14 fields, mapped from docs/state-map/party.md §7.2):
--   1. save_ready          boolean        saveData.Progress.Progress[1] > 0 (false on title Setup placeholder)
--   2. money               int|null       saveData.Player.currentMoney
--   3. play_time           number|null    saveData.Player.PlayTime (seconds, rounded to 1 decimal)
--   4. party_ids           int[]|null     saveData.Party.partyList (DT_UnitParameter.ID, formation order, <= 4)
--   5. guest_unit_id       int|null       saveData.Party.guestUnitId (-1 -> null; DT_UnitParameter.ID)
--   6. party               Member[]|null  Active party members in party_ids order:
--        unit_id           int            DT_UnitParameter.ID
--        hp, max_hp        int|null       CPP_VM_UnitParam.HitPointVM (real-time in battle; hp falls back to SaveData)
--        mp, max_mp        int|null       CPP_VM_UnitParam.MagicPointVM (real-time in battle; mp falls back to SaveData)
--        level             int|null       LevelProgressionVM.StageVM.CurrentPoint (fallback GetLevel())
--        exp               int|null       SaveData_UnitParameter.Experience
--        weapon_id         int|null       SaveData_UnitParameter.EquipWeaponId (-1 -> null; Item/*.json ID)
--        armor_id          int|null       SaveData_UnitParameter.EquipArmorId (-1 -> null; Item/*.json ID)
--        accessory_ids     int[]|null     SaveData_UnitParameter.EquipAccessoryIdList (Item/*.json ID)
--        vision_id         int|null       SaveData_UnitParameter.equipVisionId (-1 -> null)
--        join_status       int|null       SaveData_UnitParameter.JoinStatus (eUnitJoinStatus: 0=Join..4=Lost)
--   7. party_stats         map|null       Record<string(unit_id), UnitStats|null> from ParamPointVMMap keys 5..9,11..16:
--        attack, defence, intelligence, mind, agility, equip_cost, max_equip_cost,
--        physical_attack, physical_defense, magical_attack, magical_defense, divine (cached 4 ticks / dirty check)
--   8. inventory           object|null    Summary only (full list via "inventory" command):
--        count             int            saveData.inventory:GetArrayNum() (distinct entries, includes equipped gear)
--        total_qty         int            Sum of Num across all entries
--        hash              string         8-hex-char deterministic hash of sorted (item_id, num) pairs
--        changed           Delta[]        Diff since previous scan: { item_id, before, after, delta } (capped at 32)
--   9. progress            int|null       saveData.Progress.Progress[1] (C++ Progress[0], condition {progress:0})
--  10. chapter             int|null       null if progress<=0; 0 if <1020 (prologue); else floor(progress/1000) (DT_ChapterData.ChapterNo)
--  11. destination_map_ids int[]|null     scenSub.m_CurrentDestinationMapIdList (DT_MapData ID)
--  12. event_flags         object|null    Summary of 2048 flags (full list via "event_flags" command):
--        total             int            saveData.Progress.eventFlag:GetArrayNum() (2048)
--        true_count        int            Number of true flags
--        hash              string         8-hex-char deterministic hash of true 0-based flag IDs
--        added             int[]          0-based flag IDs ({flag:<id>}) flipped false->true since last scan (capped at 64)
--        removed           int[]          0-based flag IDs ({flag:<id>}) flipped true->false since last scan (capped at 64)
--  13. main_quest_id       int|null       questSub.MainQuestId (QuestDataMap / DT_QuestData ID)
--  14. last_saved_slot     int|null       optData.LastSavedDataIndex (-1 -> null; 0=auto, 1..10=manual)
--
-- Read-only commands (M.commands):
--   - save_slots:  { slot_count, last_saved_slot, slots: [{ slot_index, is_auto_save, exists, load_status,
--                    main_progress, chapter, current_map_id, play_level, play_time, date_time_ticks,
--                    difficulty, is_cleared, party_unit_ids }] }
--   - inventory:   { count, total_qty, hash, items: [{ item_id, num }] } (untruncated full inventory)
--   - event_flags: { total, true_count, hash, true_ids: int[] } (all 0-based true flag IDs in 0..2047)

local json = require("ffrbridge.json")

local M = { name = "party", commands = {} }

local LOW_FREQ_TICKS = 4
local MAX_INV_DIFF = 32
local MAX_FLAG_DIFF = 64
local DEFAULT_SAVE_SLOT_NUM = 11
local HASH_MOD = 4294967296
local HASH_SEED = 5381

-- eUnitParameterType keys in CPP_VM_UnitParam.ParamPointVMMap (excludes 10=Level which is always -1).
local STAT_SPECS = {
  { enumKey = 5, field = "attack" },
  { enumKey = 6, field = "defence" },
  { enumKey = 7, field = "intelligence" },
  { enumKey = 8, field = "mind" },
  { enumKey = 9, field = "agility" },
  { enumKey = 11, field = "equip_cost", maxField = "max_equip_cost" },
  { enumKey = 12, field = "physical_attack" },
  { enumKey = 13, field = "physical_defense" },
  { enumKey = 14, field = "magical_attack" },
  { enumKey = 15, field = "magical_defense" },
  { enumKey = 16, field = "divine" },
}

local function isFiniteNumber(value)
  return type(value) == "number" and value == value and value ~= math.huge and value ~= -math.huge
end

local function toInt(value)
  if not isFiniteNumber(value) then return nil end
  return math.floor(value)
end

local function round1(value)
  local scaled = math.floor(value * 10 + 0.5)
  if scaled == 0 then return 0 end
  return scaled / 10
end

local function optionalId(value)
  local n = toInt(value)
  if n == nil or n < 0 then return json.null end
  return n
end

-- Convert FDateTime.Ticks (Int64) to a decimal string so JS consumers do not lose precision above 2^53.
local function formatTicks(raw)
  if raw == nil then return json.null end
  if type(raw) == "string" then
    if raw == "" or raw == "0" then return json.null end
    return raw
  end
  if not isFiniteNumber(raw) or raw <= 0 then return json.null end
  if math.type and math.type(raw) == "integer" then
    return tostring(raw)
  end
  return string.format("%.0f", raw)
end

local function hashStep(h, value)
  local n = math.floor(value) % HASH_MOD
  return (h * 131 + n + 1) % HASH_MOD
end

local function hashHex(h)
  local v = math.floor(h) % HASH_MOD
  local hi = math.floor(v / 65536)
  local lo = v % 65536
  return string.format("%04x%04x", hi, lo)
end

local function recordErr(ctx, field, message)
  if ctx and type(ctx.err) == "function" and field then
    ctx.err(field, tostring(message))
  end
end

local function isValidObject(obj)
  if obj == nil then return false, nil end
  local okFn, fn = pcall(function() return obj.IsValid end)
  if not okFn then return false, tostring(fn) end
  if type(fn) ~= "function" then return false, "对象缺少 IsValid 方法" end
  local ok, valid = pcall(fn, obj)
  if not ok then return false, tostring(valid) end
  return valid == true, nil
end

local function isStructPresent(obj)
  if obj == nil then return false, nil end
  local okFn, fn = pcall(function() return obj.IsValid end)
  if okFn and type(fn) == "function" then
    local ok, valid = pcall(fn, obj)
    if not ok then return false, tostring(valid) end
    return valid == true, nil
  end
  return true, nil
end

local function safeProp(obj, key)
  if obj == nil then return false, "对象为空" end
  return pcall(function() return obj[key] end)
end

local function unwrap(value)
  if value ~= nil then
    local okFn, getter = pcall(function() return value.get end)
    if okFn and type(getter) == "function" then
      local okVal, inner = pcall(getter, value)
      if okVal then return inner end
    end
  end
  return value
end

local function arrayLen(arr)
  if arr == nil then return nil, "数组为空" end
  local okFn, getNum = pcall(function() return arr.GetArrayNum end)
  if okFn and type(getNum) == "function" then
    local okLen, len = pcall(getNum, arr)
    if not okLen then return nil, tostring(len) end
    local n = toInt(len)
    if n == nil or n < 0 then return nil, "数组长度不合法" end
    return n, nil
  end
  if type(arr) == "table" then
    return #arr, nil
  end
  return nil, "不支持的数组对象"
end

local function arrayAt(arr, index)
  local ok, val = pcall(function() return arr[index] end)
  if not ok then return nil, tostring(val) end
  return val, nil
end

local function readIntArray(arr, dropNegative)
  local len, lenErr = arrayLen(arr)
  if len == nil then return nil, lenErr end
  local out = {}
  for i = 1, len do
    local raw, itemErr = arrayAt(arr, i)
    if itemErr then return nil, itemErr end
    local n = toInt(raw)
    if n == nil then return nil, "数组元素不是有效整数" end
    if not dropNegative or n >= 0 then
      out[#out + 1] = n
    end
  end
  return json.array(out), nil
end

local function forEachMap(mapObj, callback)
  if mapObj == nil then return false, "Map 为空" end
  local okFn, fn = pcall(function() return mapObj.ForEach end)
  if okFn and type(fn) == "function" then
    local cbErr = nil
    local okIter, iterErr = pcall(fn, mapObj, function(rawKey, rawVal)
      if cbErr ~= nil then return true end
      local okCb, err = pcall(callback, unwrap(rawKey), unwrap(rawVal))
      if not okCb then
        cbErr = tostring(err)
        return true
      end
    end)
    if not okIter then return false, tostring(iterErr) end
    if cbErr ~= nil then return false, cbErr end
    return true, nil
  end
  if type(mapObj) == "table" then
    for k, v in pairs(mapObj) do
      local okCb, err = pcall(callback, k, v)
      if not okCb then return false, tostring(err) end
    end
    return true, nil
  end
  return false, "不支持遍历 Map"
end

local function readMapToTable(mapObj)
  local out = {}
  local ok, err = forEachMap(mapObj, function(k, v)
    if k ~= nil then out[k] = v end
  end)
  if not ok then return nil, err end
  return out, nil
end

local function computeChapter(progress)
  if progress == nil or progress <= 0 then return json.null end
  if progress < 1020 then return 0 end
  return math.floor(progress / 1000)
end

local function maybeInvalidateForWorld(ctx, cache)
  local worldKey = ctx and (ctx.world or ctx.map) or nil
  if worldKey ~= nil and cache.lastWorld ~= nil and cache.lastWorld ~= worldKey then
    cache.saveMgr = nil
    cache.manualVmMgr = nil
    cache.uvmMgr = nil
    cache.scenSub = nil
    cache.questSub = nil
  end
  if worldKey ~= nil then
    cache.lastWorld = worldKey
  end
end

local function getCachedSingleton(ctx, cache, key, className, errField)
  local cached = cache[key]
  if cached ~= nil then
    local valid = isValidObject(cached)
    if valid then return cached, nil end
    cache[key] = nil
  end
  if not ctx or type(ctx.find) ~= "function" then
    local msg = "ctx.find 不可用"
    recordErr(ctx, errField, msg)
    return nil, msg
  end
  local ok, found = pcall(ctx.find, className)
  if not ok then
    local msg = tostring(found)
    recordErr(ctx, errField, msg)
    return nil, msg
  end
  local valid, validErr = isValidObject(found)
  if validErr then
    recordErr(ctx, errField, validErr)
    return nil, validErr
  end
  if not valid then
    local msg = "找不到有效的 " .. className
    recordErr(ctx, errField, msg)
    return nil, msg
  end
  cache[key] = found
  return found, nil
end

-- Resolve CPP_UnitVMManager via cached pointer, CPP_ManualViewModelManager.UnitVMManager, or direct find.
local function getUnitVmManager(ctx, cache, errField)
  if cache.uvmMgr ~= nil then
    if isValidObject(cache.uvmMgr) then return cache.uvmMgr, nil end
    cache.uvmMgr = nil
  end

  local lastErr = nil
  local manual = cache.manualVmMgr
  if manual ~= nil and not isValidObject(manual) then
    cache.manualVmMgr = nil
    manual = nil
  end
  if manual == nil and ctx and type(ctx.find) == "function" then
    local okFind, found = pcall(ctx.find, "CPP_ManualViewModelManager")
    if okFind and isValidObject(found) then
      cache.manualVmMgr = found
      manual = found
    elseif not okFind then
      lastErr = tostring(found)
    end
  end

  if manual ~= nil then
    local okProp, uvm = safeProp(manual, "UnitVMManager")
    if okProp and isValidObject(uvm) then
      cache.uvmMgr = uvm
      return uvm, nil
    elseif not okProp then
      lastErr = tostring(uvm)
    end
  end

  if ctx and type(ctx.find) == "function" then
    local okFind, found = pcall(ctx.find, "CPP_UnitVMManager")
    if okFind and isValidObject(found) then
      cache.uvmMgr = found
      return found, nil
    elseif not okFind then
      lastErr = tostring(found)
    end
  else
    lastErr = "ctx.find 不可用"
  end

  local msg = lastErr or "找不到有效的 CPP_UnitVMManager"
  recordErr(ctx, errField, msg)
  return nil, msg
end

local function getSaveData(ctx, cache, mgrErrField, dataErrField)
  local saveMgr, mgrErr = getCachedSingleton(ctx, cache, "saveMgr", "CPP_MuchaSaveDataManager", mgrErrField)
  if not saveMgr then return nil, nil, mgrErr end

  local okData, saveData = safeProp(saveMgr, "m_pGameSaveData")
  if not okData then
    local msg = tostring(saveData)
    recordErr(ctx, dataErrField, msg)
    return nil, saveMgr, msg
  end
  local valid, validErr = isValidObject(saveData)
  if validErr then
    recordErr(ctx, dataErrField, validErr)
    return nil, saveMgr, validErr
  end
  if not valid then
    local msg = "m_pGameSaveData 无效"
    recordErr(ctx, dataErrField, msg)
    return nil, saveMgr, msg
  end
  return saveData, saveMgr, nil
end

local function readLastSavedSlot(ctx, saveMgr, allowGetterFallback, errField)
  if not saveMgr then return json.null, "CPP_MuchaSaveDataManager 不可用" end
  local okOpt, optData = safeProp(saveMgr, "m_pOptionSaveData")
  if okOpt and isValidObject(optData) then
    local okIdx, idx = safeProp(optData, "LastSavedDataIndex")
    if okIdx then
      local n = toInt(idx)
      if n == nil then
        local msg = "LastSavedDataIndex 不是有效整数"
        recordErr(ctx, errField, msg)
        return json.null, msg
      end
      if n < 0 then return json.null, nil end
      return n, nil
    end
    local msg = tostring(idx)
    recordErr(ctx, errField, msg)
    return json.null, msg
  end

  if allowGetterFallback then
    local okFn, getter = safeProp(saveMgr, "GetLatestSaveSlotIndex")
    if okFn and type(getter) == "function" then
      local okVal, idx = pcall(getter, saveMgr)
      if okVal then
        local n = toInt(idx)
        if n ~= nil then
          if n < 0 then return json.null, nil end
          return n, nil
        end
      end
    end
  end

  local msg = (not okOpt) and tostring(optData) or "m_pOptionSaveData 无效"
  recordErr(ctx, errField, msg)
  return json.null, msg
end

local function resolveUnitVm(uvmMgr, fallbackMapRef, uid)
  if not uvmMgr then return nil, "UnitVMManager 不可用" end
  local getterErr = nil
  local okFn, getter = safeProp(uvmMgr, "GetUnitParamVM")
  if okFn and type(getter) == "function" then
    local okCall, vm = pcall(getter, uvmMgr, uid)
    if okCall then
      local valid = isValidObject(vm)
      if valid then return vm, nil end
    else
      getterErr = tostring(vm)
    end
  end

  -- Fallback to UnitParamVMMap once per tick if GetUnitParamVM is missing or returned nil.
  if fallbackMapRef.loaded == false then
    fallbackMapRef.loaded = true
    local okMap, rawMap = safeProp(uvmMgr, "UnitParamVMMap")
    if okMap and rawMap ~= nil then
      local tbl, mapErr = readMapToTable(rawMap)
      fallbackMapRef.map = tbl
      fallbackMapRef.err = mapErr
    else
      fallbackMapRef.err = (not okMap) and tostring(rawMap) or "UnitParamVMMap 为空"
    end
  end

  if fallbackMapRef.map then
    local vm = fallbackMapRef.map[uid] or fallbackMapRef.map[tostring(uid)]
    if isValidObject(vm) then return vm, nil end
  end

  return nil, getterErr or fallbackMapRef.err or ("找不到角色 ViewModel: " .. tostring(uid))
end

local function readPointPair(ctx, vm, propName, errPrefix)
  local okSub, pointVm = safeProp(vm, propName)
  if not okSub then
    recordErr(ctx, errPrefix, pointVm)
    return nil, nil
  end
  if not isValidObject(pointVm) then
    recordErr(ctx, errPrefix, propName .. " 无效")
    return nil, nil
  end
  local cur, maxVal = nil, nil
  local okCur, rawCur = safeProp(pointVm, "CurrentPoint")
  if okCur then
    cur = toInt(rawCur)
    if cur == nil then recordErr(ctx, errPrefix .. ".current", "CurrentPoint 不是有效整数") end
  else
    recordErr(ctx, errPrefix .. ".current", rawCur)
  end
  local okMax, rawMax = safeProp(pointVm, "MaxPoint")
  if okMax then
    maxVal = toInt(rawMax)
    if maxVal == nil then recordErr(ctx, errPrefix .. ".max", "MaxPoint 不是有效整数") end
  else
    recordErr(ctx, errPrefix .. ".max", rawMax)
  end
  return cur, maxVal
end

local function readUnitLevel(vm)
  local lastErr = nil
  local okProg, progVm = safeProp(vm, "LevelProgressionVM")
  if okProg and isValidObject(progVm) then
    local okStage, stageVm = safeProp(progVm, "StageVM")
    if okStage and isValidObject(stageVm) then
      local okCur, rawCur = safeProp(stageVm, "CurrentPoint")
      if okCur then
        local lv = toInt(rawCur)
        if lv ~= nil and lv >= 1 then return lv, nil end
      else
        lastErr = tostring(rawCur)
      end
    elseif not okStage then
      lastErr = tostring(stageVm)
    end
    local okProgFn, progGetLv = safeProp(progVm, "GetLevel")
    if okProgFn and type(progGetLv) == "function" then
      local okLv, rawLv = pcall(progGetLv, progVm)
      if okLv then
        local lv = toInt(rawLv)
        if lv ~= nil and lv >= 1 then return lv, nil end
      else
        lastErr = tostring(rawLv)
      end
    end
  elseif not okProg then
    lastErr = tostring(progVm)
  end

  local okFn, getLv = safeProp(vm, "GetLevel")
  if okFn and type(getLv) == "function" then
    local okLv, rawLv = pcall(getLv, vm)
    if okLv then
      local lv = toInt(rawLv)
      if lv ~= nil and lv >= 1 then return lv, nil end
    else
      lastErr = tostring(rawLv)
    end
  end

  return nil, lastErr or "无法读取角色等级"
end

local function readUnitStats(ctx, vm, uid)
  local okMap, rawMap = safeProp(vm, "ParamPointVMMap")
  if not okMap or rawMap == nil then
    recordErr(ctx, "party.party_stats." .. tostring(uid), okMap and "ParamPointVMMap 为空" or rawMap)
    return json.null
  end
  local points, mapErr = readMapToTable(rawMap)
  if not points then
    recordErr(ctx, "party.party_stats." .. tostring(uid), mapErr)
    return json.null
  end

  local stats = {}
  local anyValid = false
  for _, spec in ipairs(STAT_SPECS) do
    local pvm = points[spec.enumKey] or points[tostring(spec.enumKey)]
    if (pvm == nil or not isValidObject(pvm)) and spec.enumKey == 11 then
      local okFallback, costVm = safeProp(vm, "SkillEquipCostVM")
      if okFallback and isValidObject(costVm) then pvm = costVm end
    end
    if isValidObject(pvm) then
      anyValid = true
      local okCur, rawCur = safeProp(pvm, "CurrentPoint")
      local cur = okCur and toInt(rawCur) or nil
      if cur == nil then
        recordErr(ctx, "party.party_stats." .. tostring(uid) .. "." .. spec.field, okCur and "数值非法" or rawCur)
        stats[spec.field] = json.null
      else
        stats[spec.field] = cur
      end
      if spec.maxField then
        local okMax, rawMax = safeProp(pvm, "MaxPoint")
        local maxVal = okMax and toInt(rawMax) or nil
        if maxVal == nil then
          recordErr(ctx, "party.party_stats." .. tostring(uid) .. "." .. spec.maxField, okMax and "上限非法" or rawMax)
          stats[spec.maxField] = json.null
        else
          stats[spec.maxField] = maxVal
        end
      end
    else
      stats[spec.field] = json.null
      if spec.maxField then stats[spec.maxField] = json.null end
    end
  end

  if not anyValid then
    recordErr(ctx, "party.party_stats." .. tostring(uid), "ParamPointVMMap 缺少有效能力项")
    return json.null
  end
  return stats
end

local function buildSaveUnitIndex(ctx, saveData)
  local byId = {}
  local okList, list = safeProp(saveData, "unitParameterList")
  if not okList or list == nil then
    recordErr(ctx, "party.unit_parameters", okList and "unitParameterList 为空" or list)
    return byId
  end
  local len, lenErr = arrayLen(list)
  if len == nil then
    recordErr(ctx, "party.unit_parameters", lenErr)
    return byId
  end
  for i = 1, len do
    local entry, itemErr = arrayAt(list, i)
    if itemErr then
      recordErr(ctx, "party.unit_parameters", itemErr)
    elseif isStructPresent(entry) then
      local okId, rawId = safeProp(entry, "UnitId")
      local uid = okId and toInt(rawId) or nil
      if uid ~= nil then byId[uid] = entry end
    end
  end
  return byId
end

local function readSaveUnitFields(ctx, saveUnit, uid)
  if not saveUnit then
    return {
      hp = nil,
      mp = nil,
      exp = json.null,
      weapon_id = json.null,
      armor_id = json.null,
      accessory_ids = json.null,
      vision_id = json.null,
      join_status = json.null,
    }
  end

  local prefix = "party.unit." .. tostring(uid)
  local function readIntField(propName, fieldLabel)
    local ok, raw = safeProp(saveUnit, propName)
    if not ok then
      recordErr(ctx, prefix .. "." .. fieldLabel, raw)
      return nil
    end
    local n = toInt(raw)
    if n == nil then
      recordErr(ctx, prefix .. "." .. fieldLabel, propName .. " 不是有效整数")
      return nil
    end
    return n
  end

  local saveHp = nil
  local okHp, rawHp = safeProp(saveUnit, "HitPoint")
  if okHp then saveHp = toInt(rawHp) end
  local saveMp = nil
  local okMp, rawMp = safeProp(saveUnit, "MagicPoint")
  if okMp then saveMp = toInt(rawMp) end

  local exp = readIntField("Experience", "exp")
  local weaponRaw = readIntField("EquipWeaponId", "weapon_id")
  local armorRaw = readIntField("EquipArmorId", "armor_id")
  local visionRaw = readIntField("equipVisionId", "vision_id")
  local joinRaw = readIntField("JoinStatus", "join_status")

  local accessories = json.null
  local okAcc, accArr = safeProp(saveUnit, "EquipAccessoryIdList")
  if not okAcc or accArr == nil then
    recordErr(ctx, prefix .. ".accessory_ids", okAcc and "EquipAccessoryIdList 为空" or accArr)
  else
    local list, listErr = readIntArray(accArr, true)
    if list == nil then
      recordErr(ctx, prefix .. ".accessory_ids", listErr)
    else
      accessories = list
    end
  end

  return {
    hp = saveHp,
    mp = saveMp,
    exp = exp ~= nil and exp or json.null,
    weapon_id = weaponRaw ~= nil and optionalId(weaponRaw) or json.null,
    armor_id = armorRaw ~= nil and optionalId(armorRaw) or json.null,
    accessory_ids = accessories,
    vision_id = visionRaw ~= nil and optionalId(visionRaw) or json.null,
    join_status = joinRaw ~= nil and joinRaw or json.null,
  }
end

local function makeStatsSignature(members)
  local parts = {}
  for i, m in ipairs(members) do
    local acc = ""
    if type(m.accessory_ids) == "table" and not json.isNull(m.accessory_ids) then
      local ids = {}
      for j = 1, #m.accessory_ids do ids[j] = tostring(m.accessory_ids[j]) end
      acc = table.concat(ids, ",")
    end
    parts[i] = table.concat({
      tostring(m.unit_id),
      tostring(json.isNull(m.level) and "n" or m.level),
      tostring(json.isNull(m.exp) and "n" or m.exp),
      tostring(json.isNull(m.max_hp) and "n" or m.max_hp),
      tostring(json.isNull(m.max_mp) and "n" or m.max_mp),
      tostring(json.isNull(m.weapon_id) and "n" or m.weapon_id),
      tostring(json.isNull(m.armor_id) and "n" or m.armor_id),
      acc,
      tostring(json.isNull(m.vision_id) and "n" or m.vision_id),
    }, ":")
  end
  return table.concat(parts, "|")
end

local function collectPartyAndStats(ctx, cache, tick, saveData, partyIds, uvmMgr)
  if partyIds == nil or json.isNull(partyIds) then
    cache.partyStats = nil
    cache.lastStatsSig = nil
    return json.null, json.null
  end
  if #partyIds == 0 then
    local emptyStats = json.object({})
    cache.partyStats = emptyStats
    cache.lastStatsSig = ""
    cache.lastStatsTick = tick
    return json.array({}), emptyStats
  end

  local saveUnits = buildSaveUnitIndex(ctx, saveData)
  local fallbackMapRef = { loaded = false, map = nil, err = nil }
  local members = {}
  local vmsById = {}

  for index = 1, #partyIds do
    local uid = partyIds[index]
    local saveFields = readSaveUnitFields(ctx, saveUnits[uid], uid)
    local hp, maxHp, mp, maxMp, level = nil, nil, nil, nil, nil

    if uvmMgr ~= nil then
      local vm, vmErr = resolveUnitVm(uvmMgr, fallbackMapRef, uid)
      if vm ~= nil then
        vmsById[uid] = vm
        hp, maxHp = readPointPair(ctx, vm, "HitPointVM", "party.unit." .. tostring(uid) .. ".hp")
        mp, maxMp = readPointPair(ctx, vm, "MagicPointVM", "party.unit." .. tostring(uid) .. ".mp")
        local lv, lvErr = readUnitLevel(vm)
        if lv ~= nil then
          level = lv
        else
          recordErr(ctx, "party.unit." .. tostring(uid) .. ".level", lvErr)
        end
      else
        recordErr(ctx, "party.unit." .. tostring(uid) .. ".vm", vmErr)
      end
    end

    -- Fall back to SaveData HP/MP when ViewModel is unavailable (non-realtime in battle, but better than null).
    if hp == nil and saveFields.hp ~= nil then hp = saveFields.hp end
    if mp == nil and saveFields.mp ~= nil then mp = saveFields.mp end

    members[index] = {
      unit_id = uid,
      hp = hp ~= nil and hp or json.null,
      max_hp = maxHp ~= nil and maxHp or json.null,
      mp = mp ~= nil and mp or json.null,
      max_mp = maxMp ~= nil and maxMp or json.null,
      level = level ~= nil and level or json.null,
      exp = saveFields.exp,
      weapon_id = saveFields.weapon_id,
      armor_id = saveFields.armor_id,
      accessory_ids = saveFields.accessory_ids,
      vision_id = saveFields.vision_id,
      join_status = saveFields.join_status,
    }
  end

  local partyStats
  if uvmMgr == nil then
    cache.partyStats = nil
    partyStats = json.null
  else
    local sig = makeStatsSignature(members)
    local due = cache.partyStats == nil
      or cache.lastStatsSig ~= sig
      or (tick - (cache.lastStatsTick or -LOW_FREQ_TICKS)) >= LOW_FREQ_TICKS
    if due then
      local statsMap = json.object({})
      for index = 1, #partyIds do
        local uid = partyIds[index]
        local vm = vmsById[uid]
        if vm ~= nil then
          statsMap[tostring(uid)] = readUnitStats(ctx, vm, uid)
        else
          statsMap[tostring(uid)] = json.null
        end
      end
      cache.partyStats = statsMap
      cache.lastStatsSig = sig
      cache.lastStatsTick = tick
    end
    partyStats = cache.partyStats
  end

  return json.array(members), partyStats
end

local function scanInventoryRaw(saveData)
  local okArr, invArr = safeProp(saveData, "inventory")
  if not okArr or invArr == nil then
    return nil, okArr and "inventory 为空" or tostring(invArr)
  end
  local len, lenErr = arrayLen(invArr)
  if len == nil then return nil, lenErr end

  local items = {}
  local qtyById = {}
  local totalQty = 0

  for i = 1, len do
    local entry, itemErr = arrayAt(invArr, i)
    if itemErr then return nil, itemErr end
    if not isStructPresent(entry) then return nil, "inventory 第 " .. tostring(i) .. " 项无效" end
    local okId, rawId = safeProp(entry, "ItemId")
    if not okId then return nil, tostring(rawId) end
    local okNum, rawNum = safeProp(entry, "Num")
    if not okNum then return nil, tostring(rawNum) end
    local itemId = toInt(rawId)
    local num = toInt(rawNum)
    if itemId == nil or num == nil then
      return nil, "inventory 第 " .. tostring(i) .. " 项数值非法"
    end
    items[i] = { item_id = itemId, num = num }
    qtyById[itemId] = (qtyById[itemId] or 0) + num
    totalQty = totalQty + num
  end

  local sortedIds = {}
  for itemId in pairs(qtyById) do sortedIds[#sortedIds + 1] = itemId end
  table.sort(sortedIds)

  local h = hashStep(HASH_SEED, len)
  for i = 1, #sortedIds do
    local itemId = sortedIds[i]
    h = hashStep(h, itemId)
    h = hashStep(h, qtyById[itemId])
  end

  return {
    count = len,
    total_qty = totalQty,
    hash = hashHex(h),
    items = json.array(items),
    qtyById = qtyById,
    sortedIds = sortedIds,
  }, nil
end

local function diffInventory(prevMap, nextMap)
  if prevMap == nil then return json.array({}) end
  local allIds = {}
  local seen = {}
  for id in pairs(prevMap) do
    seen[id] = true
    allIds[#allIds + 1] = id
  end
  for id in pairs(nextMap) do
    if not seen[id] then
      seen[id] = true
      allIds[#allIds + 1] = id
    end
  end
  table.sort(allIds)

  local changed = {}
  for i = 1, #allIds do
    local id = allIds[i]
    local before = prevMap[id] or 0
    local after = nextMap[id] or 0
    if before ~= after then
      if #changed >= MAX_INV_DIFF then break end
      changed[#changed + 1] = {
        item_id = id,
        before = before,
        after = after,
        delta = after - before,
      }
    end
  end
  return json.array(changed)
end

local function collectInventorySummary(ctx, cache, tick, saveData, saveReadyChanged, money)
  local okArr, invArr = safeProp(saveData, "inventory")
  if not okArr or invArr == nil then
    cache.invSummary = nil
    recordErr(ctx, "party.inventory", okArr and "inventory 为空" or invArr)
    return json.null
  end
  local curLen, lenErr = arrayLen(invArr)
  if curLen == nil then
    cache.invSummary = nil
    recordErr(ctx, "party.inventory", lenErr)
    return json.null
  end

  if saveReadyChanged then
    cache.invMap = nil
    cache.invSummary = nil
  end

  local due = cache.invSummary == nil
    or curLen ~= cache.lastInvLen
    or money ~= cache.lastInvMoney
    or (tick - (cache.lastInvTick or -LOW_FREQ_TICKS)) >= LOW_FREQ_TICKS

  if not due then
    return cache.invSummary
  end

  local scanned, scanErr = scanInventoryRaw(saveData)
  if not scanned then
    cache.invSummary = nil
    recordErr(ctx, "party.inventory", scanErr)
    return json.null
  end

  local changed = diffInventory(cache.invMap, scanned.qtyById)
  local summary = {
    count = scanned.count,
    total_qty = scanned.total_qty,
    hash = scanned.hash,
    changed = changed,
  }
  cache.invMap = scanned.qtyById
  cache.invSummary = summary
  cache.lastInvLen = scanned.count
  cache.lastInvMoney = money
  cache.lastInvTick = tick
  return summary
end

local function scanEventFlagsRaw(saveData)
  local okProg, progStruct = safeProp(saveData, "Progress")
  if not okProg or not isStructPresent(progStruct) then
    return nil, okProg and "Progress 结构无效" or tostring(progStruct)
  end
  local okArr, flagArr = safeProp(progStruct, "eventFlag")
  if not okArr or flagArr == nil then
    return nil, okArr and "eventFlag 为空" or tostring(flagArr)
  end
  local len, lenErr = arrayLen(flagArr)
  if len == nil then return nil, lenErr end

  local trueIds = {}
  local trueSet = {}
  local h = hashStep(HASH_SEED, len)
  for i = 1, len do
    local raw, itemErr = arrayAt(flagArr, i)
    if itemErr then return nil, itemErr end
    if type(raw) ~= "boolean" then
      return nil, "eventFlag 第 " .. tostring(i) .. " 项不是布尔值"
    end
    if raw == true then
      local flagId = i - 1 -- C++ 0-based {flag:<id>}
      trueIds[#trueIds + 1] = flagId
      trueSet[flagId] = true
      h = hashStep(h, flagId)
    end
  end

  return {
    total = len,
    true_count = #trueIds,
    hash = hashHex(h),
    true_ids = json.array(trueIds),
    trueSet = trueSet,
  }, nil
end

local function diffEventFlags(prevSet, nextSet)
  if prevSet == nil then return json.array({}), json.array({}) end
  local added = {}
  local removed = {}
  for id in pairs(nextSet) do
    if not prevSet[id] then added[#added + 1] = id end
  end
  for id in pairs(prevSet) do
    if not nextSet[id] then removed[#removed + 1] = id end
  end
  table.sort(added)
  table.sort(removed)
  while #added > MAX_FLAG_DIFF do added[#added] = nil end
  while #removed > MAX_FLAG_DIFF do removed[#removed] = nil end
  return json.array(added), json.array(removed)
end

local function collectEventFlagsSummary(ctx, cache, tick, saveData, saveReadyChanged, progress)
  if saveReadyChanged then
    cache.flagSet = nil
    cache.flagSummary = nil
  end

  local due = cache.flagSummary == nil
    or progress ~= cache.lastFlagProgress
    or (tick - (cache.lastFlagTick or -LOW_FREQ_TICKS)) >= LOW_FREQ_TICKS

  if not due then
    return cache.flagSummary
  end

  local scanned, scanErr = scanEventFlagsRaw(saveData)
  if not scanned then
    cache.flagSummary = nil
    recordErr(ctx, "party.event_flags", scanErr)
    return json.null
  end

  local added, removed = diffEventFlags(cache.flagSet, scanned.trueSet)
  local summary = {
    total = scanned.total,
    true_count = scanned.true_count,
    hash = scanned.hash,
    added = added,
    removed = removed,
  }
  cache.flagSet = scanned.trueSet
  cache.flagSummary = summary
  cache.lastFlagProgress = progress
  cache.lastFlagTick = tick
  return summary
end

local function decodeSaveSlotSummary(slotIndex, summary)
  local exists = false
  local okExists, rawExists = safeProp(summary, "bExists")
  if okExists and type(rawExists) == "boolean" then exists = rawExists end

  local loadStatus = json.null
  local okStatus, rawStatus = safeProp(summary, "LoadStatus")
  if okStatus then
    local n = toInt(rawStatus)
    if n ~= nil then loadStatus = n end
  end

  local mainProgress = json.null
  local chapter = json.null
  local okProg, rawProg = safeProp(summary, "MainProgress")
  if okProg then
    local n = toInt(rawProg)
    if n ~= nil and n >= 0 then
      mainProgress = n
      if exists then chapter = computeChapter(n) end
    end
  end

  local currentMapId = json.null
  local okMap, rawMap = safeProp(summary, "CurrentMapId")
  if okMap then currentMapId = optionalId(rawMap) end

  local playLevel = json.null
  local okLv, rawLv = safeProp(summary, "PlayLevel")
  if okLv then playLevel = optionalId(rawLv) end

  local playTime = json.null
  local okTime, rawTime = safeProp(summary, "PlayTimeSec")
  if okTime and isFiniteNumber(rawTime) and (exists or rawTime > 0) then
    playTime = round1(rawTime)
  end

  local ticks = json.null
  local okDt, dt = safeProp(summary, "DateTime")
  if okDt and isStructPresent(dt) then
    local okTicks, rawTicks = safeProp(dt, "Ticks")
    if okTicks then ticks = formatTicks(rawTicks) end
  end

  local difficulty = json.null
  local okDiff, rawDiff = safeProp(summary, "difficulty")
  if okDiff then
    local n = toInt(rawDiff)
    if n ~= nil and n >= 0 then difficulty = n end
  end

  local isCleared = json.null
  local okCleared, rawCleared = safeProp(summary, "bIsCleared")
  if okCleared and type(rawCleared) == "boolean" then isCleared = rawCleared end

  local partyUnitIds = json.array({})
  local okIds, rawIds = safeProp(summary, "PartyUnitIds")
  if okIds and rawIds ~= nil then
    local parsed = readIntArray(rawIds, true)
    if parsed ~= nil then partyUnitIds = parsed end
  end

  return {
    slot_index = slotIndex,
    is_auto_save = slotIndex == 0,
    exists = exists,
    load_status = loadStatus,
    main_progress = mainProgress,
    chapter = chapter,
    current_map_id = currentMapId,
    play_level = playLevel,
    play_time = playTime,
    date_time_ticks = ticks,
    difficulty = difficulty,
    is_cleared = isCleared,
    party_unit_ids = partyUnitIds,
  }
end

function M.collect(ctx)
  local context = ctx or {}
  local cache = context.cache or {}
  local tick = toInt(context.tick)
  if tick == nil then
    cache._tick = (cache._tick or 0) + 1
    tick = cache._tick
  end

  maybeInvalidateForWorld(context, cache)

  local saveData, saveMgr = getSaveData(context, cache, "party.save_manager", "party.save_data")

  -- 1, 9, 10: save_ready, progress, chapter
  local progress = nil
  local progressOut = json.null
  local chapterOut = json.null
  local saveReady = false
  if saveData then
    local okProgStruct, progStruct = safeProp(saveData, "Progress")
    if not okProgStruct or not isStructPresent(progStruct) then
      recordErr(context, "party.progress", okProgStruct and "Progress 结构无效" or progStruct)
    else
      local okArr, progArr = safeProp(progStruct, "Progress")
      if not okArr or progArr == nil then
        recordErr(context, "party.progress", okArr and "Progress 数组为空" or progArr)
      else
        local rawP0, p0Err = arrayAt(progArr, 1) -- Lua 1-based -> C++ Progress[0]
        if p0Err then
          recordErr(context, "party.progress", p0Err)
        else
          local p0 = toInt(rawP0)
          if p0 == nil then
            recordErr(context, "party.progress", "Progress[0] 不是有效整数")
          else
            progress = p0
            progressOut = p0
            saveReady = p0 > 0
            chapterOut = computeChapter(p0)
          end
        end
      end
    end
  end

  local saveReadyChanged = cache.lastSaveReady ~= nil and cache.lastSaveReady ~= saveReady
  cache.lastSaveReady = saveReady

  -- 2, 3: money, play_time
  local moneyVal = nil
  local moneyOut = json.null
  local playTimeOut = json.null
  if saveData then
    local okPlayer, player = safeProp(saveData, "Player")
    if not okPlayer or not isStructPresent(player) then
      recordErr(context, "party.player", okPlayer and "Player 结构无效" or player)
    else
      local okMoney, rawMoney = safeProp(player, "currentMoney")
      if not okMoney then
        recordErr(context, "party.money", rawMoney)
      else
        local m = toInt(rawMoney)
        if m == nil then
          recordErr(context, "party.money", "currentMoney 不是有效整数")
        else
          moneyVal = m
          moneyOut = m
        end
      end

      local okTime, rawTime = safeProp(player, "PlayTime")
      if not okTime then
        recordErr(context, "party.play_time", rawTime)
      elseif not isFiniteNumber(rawTime) then
        recordErr(context, "party.play_time", "PlayTime 不是有限数字")
      else
        playTimeOut = round1(rawTime)
      end
    end
  end

  -- 4, 5: party_ids, guest_unit_id
  local partyIds = nil
  local partyIdsOut = json.null
  local guestUnitIdOut = json.null
  if saveData then
    local okParty, partyStruct = safeProp(saveData, "Party")
    if not okParty or not isStructPresent(partyStruct) then
      recordErr(context, "party.party_struct", okParty and "Party 结构无效" or partyStruct)
    else
      local okList, rawList = safeProp(partyStruct, "partyList")
      if not okList or rawList == nil then
        recordErr(context, "party.party_ids", okList and "partyList 为空" or rawList)
      else
        local parsed, parseErr = readIntArray(rawList, false)
        if parsed == nil then
          recordErr(context, "party.party_ids", parseErr)
        else
          partyIds = parsed
          partyIdsOut = parsed
        end
      end

      local okGuest, rawGuest = safeProp(partyStruct, "guestUnitId")
      if not okGuest then
        recordErr(context, "party.guest_unit_id", rawGuest)
      else
        local g = toInt(rawGuest)
        if g == nil then
          recordErr(context, "party.guest_unit_id", "guestUnitId 不是有效整数")
        else
          guestUnitIdOut = g < 0 and json.null or g
        end
      end
    end
  end

  -- 6, 7: party, party_stats
  local partyOut = json.null
  local partyStatsOut = json.null
  if saveData and partyIds ~= nil then
    local uvmMgr = getUnitVmManager(context, cache, "party.unit_vm_manager")
    partyOut, partyStatsOut = collectPartyAndStats(context, cache, tick, saveData, partyIds, uvmMgr)
  elseif not saveData then
    cache.partyStats = nil
    cache.lastStatsSig = nil
  end

  -- 8: inventory summary + diff
  local inventoryOut = json.null
  if saveData then
    inventoryOut = collectInventorySummary(context, cache, tick, saveData, saveReadyChanged, moneyVal)
  else
    cache.invSummary = nil
  end

  -- 11: destination_map_ids
  local destinationMapIdsOut = json.null
  local scenSub = getCachedSingleton(context, cache, "scenSub", "CPP_ScenarioSubsystem", "party.scenario_subsystem")
  if scenSub then
    local okDest, rawDest = safeProp(scenSub, "m_CurrentDestinationMapIdList")
    if not okDest or rawDest == nil then
      recordErr(context, "party.destination_map_ids", okDest and "m_CurrentDestinationMapIdList 为空" or rawDest)
    else
      local parsed, parseErr = readIntArray(rawDest, false)
      if parsed == nil then
        recordErr(context, "party.destination_map_ids", parseErr)
      else
        destinationMapIdsOut = parsed
      end
    end
  end

  -- 12: event_flags summary + diff
  local eventFlagsOut = json.null
  if saveData then
    eventFlagsOut = collectEventFlagsSummary(context, cache, tick, saveData, saveReadyChanged, progress)
  else
    cache.flagSummary = nil
  end

  -- 13: main_quest_id
  local mainQuestIdOut = json.null
  local questSub = getCachedSingleton(context, cache, "questSub", "CPP_QuestSubsystem", "party.quest_subsystem")
  if questSub then
    local okQuest, rawQuest = safeProp(questSub, "MainQuestId")
    if not okQuest then
      recordErr(context, "party.main_quest_id", rawQuest)
    else
      local qid = toInt(rawQuest)
      if qid == nil then
        recordErr(context, "party.main_quest_id", "MainQuestId 不是有效整数")
      elseif qid >= 0 then
        mainQuestIdOut = qid
      end
    end
  end

  -- 14: last_saved_slot
  local lastSavedSlotOut = json.null
  if saveMgr then
    lastSavedSlotOut = readLastSavedSlot(context, saveMgr, false, "party.last_saved_slot")
  end

  return {
    save_ready = saveReady,
    money = moneyOut,
    play_time = playTimeOut,
    party_ids = partyIdsOut,
    guest_unit_id = guestUnitIdOut,
    party = partyOut,
    party_stats = partyStatsOut,
    inventory = inventoryOut,
    progress = progressOut,
    chapter = chapterOut,
    destination_map_ids = destinationMapIdsOut,
    event_flags = eventFlagsOut,
    main_quest_id = mainQuestIdOut,
    last_saved_slot = lastSavedSlotOut,
  }
end

-- On-demand command: full save slot occupancy list (slots 0..10).
M.commands.save_slots = function(ctx, _args)
  local context = ctx or {}
  local cache = context.cache or {}
  local saveMgr, mgrErr = getCachedSingleton(context, cache, "saveMgr", "CPP_MuchaSaveDataManager", nil)
  if not saveMgr then return nil, mgrErr or "找不到有效的 CPP_MuchaSaveDataManager" end

  local slots = {}
  local loaded = false

  local okMap, rawMap = safeProp(saveMgr, "GameSaveDataSummaryMap")
  if okMap and rawMap ~= nil then
    local mapOk = forEachMap(rawMap, function(k, v)
      local slotIdx = toInt(k)
      if slotIdx ~= nil and isStructPresent(v) then
        slots[#slots + 1] = decodeSaveSlotSummary(slotIdx, v)
      end
    end)
    if mapOk and #slots > 0 then loaded = true end
  end

  -- Fallback to FindGameSaveDataSummary(slotIndex, outSummary) when summary map is unavailable.
  if not loaded then
    local okFindFn, findFn = safeProp(saveMgr, "FindGameSaveDataSummary")
    if not okFindFn or type(findFn) ~= "function" then
      return nil, "无法读取 GameSaveDataSummaryMap，且 FindGameSaveDataSummary 不可用"
    end
    local slotNum = DEFAULT_SAVE_SLOT_NUM
    local okNumFn, numFn = safeProp(saveMgr, "GetSaveSlotNum")
    if okNumFn and type(numFn) == "function" then
      local okNum, rawNum = pcall(numFn, saveMgr)
      local n = okNum and toInt(rawNum) or nil
      if n ~= nil and n > 0 and n <= 64 then slotNum = n end
    end
    for slotIdx = 0, slotNum - 1 do
      local out = {}
      local okCall, found, retOut = pcall(findFn, saveMgr, slotIdx, out)
      if okCall and found == true then
        local summary = (type(retOut) == "table" and retOut)
          or (type(out.OutSummary) == "table" and out.OutSummary)
          or out
        if isStructPresent(summary) then
          slots[#slots + 1] = decodeSaveSlotSummary(slotIdx, summary)
        end
      end
    end
    if #slots == 0 then
      return nil, "无法读取任何存档槽摘要"
    end
  end

  table.sort(slots, function(a, b) return a.slot_index < b.slot_index end)
  local lastSavedSlot = readLastSavedSlot(context, saveMgr, true, nil)

  return {
    slot_count = #slots,
    last_saved_slot = lastSavedSlot,
    slots = json.array(slots),
  }, nil
end

-- On-demand command: full untruncated inventory list.
M.commands.inventory = function(ctx, _args)
  local context = ctx or {}
  local cache = context.cache or {}
  local saveData, _, err = getSaveData(context, cache, nil, nil)
  if not saveData then return nil, err or "无法读取存档对象" end

  local scanned, scanErr = scanInventoryRaw(saveData)
  if not scanned then return nil, "读取物品栏失败：" .. tostring(scanErr) end

  return {
    count = scanned.count,
    total_qty = scanned.total_qty,
    hash = scanned.hash,
    items = scanned.items,
  }, nil
end

-- On-demand command: full list of 0-based true event flag IDs (across all 2048 flags).
M.commands.event_flags = function(ctx, _args)
  local context = ctx or {}
  local cache = context.cache or {}
  local saveData, _, err = getSaveData(context, cache, nil, nil)
  if not saveData then return nil, err or "无法读取存档对象" end

  local scanned, scanErr = scanEventFlagsRaw(saveData)
  if not scanned then return nil, "读取事件旗标失败：" .. tostring(scanErr) end

  return {
    total = scanned.total,
    true_count = scanned.true_count,
    hash = scanned.hash,
    true_ids = scanned.true_ids,
  }, nil
end

return M
