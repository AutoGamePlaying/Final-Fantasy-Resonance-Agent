-- FfrBridge v1 topic "field" (task 11-field). Read-only: only read properties and call getters;
-- never call functions that change game state and never send input.
--
-- Performance (lookups per 500 ms tick):
--   - Cold tick after map load: up to 11 ctx.find() calls (10 net new when run via state.lua,
--     which already caches PlayerController per tick), 0 FindAllOf calls.
--   - Steady-state tick on same map: 0 ctx.find() calls and 0 FindAllOf calls (all GI/World
--     singletons are reused from ctx.cache while IsValid(); World-scoped references and
--     low-frequency actor lists are dropped immediately on World change or is_loading == true).
--   - Low-frequency refresh (on map load completion, map/area change, event/dialog end, or every
--     5 s): reads lightweight quest/landmark arrays and scans current map's CPP_MapManager lists
--     in a single pass (typically < 30 actors).
--
-- Exported fields (21 fields from docs/state-map/field.md §8.1-8.2 + camera helper fields):
--   High-frequency (every 500 ms):
--     map_id                   integer|null  CPP_MiniMapSubsystem.PropertyKey.mapId -> CDT_MapData_Prod.ID
--     area_id                  integer|null  CPP_MiniMapSubsystem.AreaManager.CurrentAreaId -> NavMeshBoundsVolume.m_AreaId
--     is_loading               boolean|null  CPP_LoadingScreenManager.IsShowLoadingScreen
--     in_battle                boolean|null  CPP_UI_BattleSubsystem.bDuringBattle
--     in_event                 boolean|null  CPP_TalkEventSubsystem.m_pEventManager:IsValid()
--     in_dialog                boolean|null  CPP_DialogManager.DialogWidget:IsValid()
--     in_menu                  boolean|null  CPP_MainMenuSubsystem:IsOpenMenu() (fallback CPP_VM_MainMenu.SceneDataStack)
--     can_control              boolean       derived safety gate for field movement input
--     camera_deg               number|null   VM_MiniMap_C.CameraDirectionDeg
--     camera_internal_rotation boolean|null  BP_MapUnit_Player_C.m_MapCameraInternalRotation
--     player.location          {x,y,z}|null  Pawn:K2_GetActorLocation()
--     player.visible           boolean|null  Pawn.m_IsVisible (false in cutscene staging & battle)
--     player.unit_id           integer|null  Pawn.m_MapUnitId -> DT_MapUnitPlayableData / DT_UnitParameter.ID
--     player.direction         string|null   VM_MiniMap_C.PlayerDirection (eMapDirectionType)
--     player.direction_deg     number|null   VM_MiniMap_C.PlayerDirectionDegree
--     player.movement_method   string|null   Pawn.m_MapMovementMethod (eMapMovementMethod)
--     player.speed_2d          number|null   PawnMovementComponent.Velocity 2D norm (fallback delta pos / dt)
--     interact_target          object|null   Pawn.m_pInteractMapGimmickList[1] or m_InteractMapUnitList[1]
--   Low-frequency (map change / event end / 5 s):
--     quest_targets            object        {main_quest_id, preferred_quest_id, destination_map_ids[]}
--     unlocked_landmarks       integer[]     VM_WldMapLandmarkList_C.AllLandmarkIdList -> CDT_MapData_Prod.ID
--     chests                   object[]      CPP_MapManager gimmicks (TreasureBox) -> CDT_MapGimmickTreasureBox
--     save_points              object[]      CPP_MapManager gimmicks (SavePoint)
--     transitions_and_npcs     object        {triggers: [...], npcs: [...]}
--   On-demand read-only command:
--     query_nav_path           (args: {start:{x,y,z}, goal:{x,y,z}}) -> {valid, partial, length, points[]}

local json = require("ffrbridge.json")
local util = require("ffrbridge.util")

local M = { name = "field", commands = {} }

local LOW_FREQ_INTERVAL_MS = 5000

-- Cutscene staging coordinate where BP_MapUnit_Player_C is parked off-screen.
local STAGING_X = 25408.0
local STAGING_Y = 384.0
local STAGING_Z = -500.0
local STAGING_EPS = 5.0

local DIRECTION_ENUM = {
  [0] = "None",
  [1] = "North",
  [2] = "South",
  [3] = "East",
  [4] = "West",
  [5] = "NorthEast",
  [6] = "SouthEast",
  [7] = "SouthWest",
  [8] = "NorthWest",
}

local MOVEMENT_METHOD_ENUM = {
  [0] = "None",
  [1] = "Walking",
  [2] = "AirShip",
  [3] = "Chocobo",
  [4] = "Ship",
  [5] = "MagickArmor",
}

local SAVE_POINT_TYPE_ENUM = {
  [0] = "None",
  [1] = "Boss",
}

local OBJECT_TYPE_ENUM = {
  [0] = "None",
  [1] = "NPC",
  [2] = "Gimmick",
  [3] = "TransitionPoint",
  [4] = "EncountArea",
  [5] = "DestinationArea",
  [6] = "Point",
}

local INTERACT_TYPE_ENUM = {
  [0] = "None",
  [1] = "Talk",
  [2] = "Talk_Crowd",
  [3] = "Talk_TalkPoint",
  [4] = "Check",
  [5] = "Check_Field",
  [6] = "Check_Map",
  [7] = "Check_Point",
  [8] = "Check_Gimmick",
  [9] = "Check_MerchantShip",
  [10] = "Battle",
  [11] = "Treasure",
  [12] = "MythicalBeast",
  [13] = "Door",
  [14] = "Vehicle",
  [15] = "Vehicle_Ship",
  [16] = "Landing",
  [17] = "Shop",
  [18] = "Shop_Weapon",
  [19] = "Shop_Armor",
  [20] = "Shop_Accessory",
  [21] = "Inn",
  [22] = "QuestBoard",
  [23] = "Bank",
  [24] = "ChocoboShop",
  [25] = "ChocoboRace",
  [26] = "MusicShop",
  [27] = "DLCShop",
  [28] = "Fortuneteller",
  [29] = "Colosseum_Rank",
  [30] = "Colosseum_Hunt",
  [31] = "Colosseum_Boss",
  [32] = "Shop_Vision",
  [33] = "SavePoint",
  [34] = "Transition",
  [35] = "Transition_Event",
  [36] = "Landmark",
  [37] = "City",
  [38] = "Dungeon",
  [39] = "Nawatobi_Jump",
  [40] = "GameOfTag_Tagged",
}

local BLOCKING_LAYER_NAMES = {
  Stack_System = true,
  Stack_System_Front1 = true,
  Stack_System_Front2 = true,
  Stack_Modal = true,
  Stack_Event = true,
  Stack_Game_Front = true,
}

local function ensureCache(ctx)
  local cache = ctx.cache
  if type(cache) ~= "table" then cache = {} end
  if type(cache.gi) ~= "table" then cache.gi = {} end
  if type(cache.world) ~= "table" then cache.world = {} end
  return cache
end

local function clearWorldState(cache, keepWorldObj)
  cache.world = keepWorldObj and { worldObj = keepWorldObj } or {}
  cache.prevLoc = nil
  cache.prevClockMs = nil
  cache.mapActorsReady = false
  cache.lastActorScanMs = nil
  cache.chests = nil
  cache.savePoints = nil
  cache.transitionsAndNpcs = nil
end

local function getCached(ctx, bucket, key, className, allowStore)
  local cached = bucket[key]
  if cached ~= nil then
    local valid = util.isValidObject(cached)
    if valid then return cached end
    bucket[key] = nil
  end
  local found = ctx.find(className)
  if found ~= nil then
    local valid = util.isValidObject(found)
    if valid then
      if allowStore ~= false then
        bucket[key] = found
      end
      return found
    end
  end
  return nil
end

local function isStagingLocation(loc)
  if type(loc) ~= "table" or json.isNull(loc) then return false end
  return math.abs(loc.x - STAGING_X) < STAGING_EPS
    and math.abs(loc.y - STAGING_Y) < STAGING_EPS
    and math.abs(loc.z - STAGING_Z) < STAGING_EPS
end

local function extractLayerName(layer)
  if layer == nil or json.isNull(layer) then return nil end
  if type(layer) == "string" then return layer end
  local valid = util.isValidObject(layer)
  if not valid then return nil end
  local okShort, shortName = pcall(function()
    if type(layer.GetFName) == "function" then
      local fname = layer:GetFName()
      if fname ~= nil and type(fname.ToString) == "function" then
        return fname:ToString()
      end
    end
    return nil
  end)
  if okShort and type(shortName) == "string" and shortName ~= "" then
    return shortName
  end
  local fullName = util.fullNameOf(layer)
  if type(fullName) == "string" then
    return fullName:match("([^.:/%s]+)$") or fullName
  end
  return nil
end

local function isBlockingInputLayer(widgetMgr)
  if widgetMgr == nil then return false end
  local layout = util.readProp(widgetMgr, "m_GameLayout")
  if layout == nil or not util.isValidObject(layout) then return false end

  local layer = nil
  local gotLayer, getterErr = util.callMethod(layout, "GetInputTargetLayer")
  if getterErr == nil then
    layer = gotLayer
  else
    layer = util.readProp(layout, "CurrentInputTargetLayer")
  end
  if layer == nil or json.isNull(layer) then return false end

  local stackGame = util.readProp(layout, "Stack_Game")
  if stackGame ~= nil and layer == stackGame then return false end

  local name = extractLayerName(layer)
  if not name or name == "" or name == "None" or name == "Stack_Game" or name == "UI.Layer.Game" then
    return false
  end
  if BLOCKING_LAYER_NAMES[name] then return true end
  if name:find("^UI%.Layer%.System") or name:find("^UI%.Layer%.Modal") or name:find("^UI%.Layer%.Event") or name:find("^UI%.Layer%.Game%.Front") then
    return true
  end
  return false
end

local function readEventId(pawn, ctx)
  local structVal, structErr = util.readProp(pawn, "m_InteractEventId")
  if structErr then
    ctx.err("field.interact_target.event_id", structErr)
    return json.null
  end
  if structVal == nil then return json.null end
  local rawId, idErr = util.readProp(structVal, "EventId")
  if idErr then
    ctx.err("field.interact_target.event_id", idErr)
    return json.null
  end
  local str, strErr = util.toStr(rawId)
  if strErr then
    ctx.err("field.interact_target.event_id", strErr)
    return json.null
  end
  if not str or str == "" or str == "None" then return json.null end
  return util.truncate(str, util.TEXT_LIMIT)
end

local function isTreasureBox(obj, interactTypeName, clsName)
  if interactTypeName == "Treasure" then return true end
  if type(clsName) == "string" and clsName:find("TreasureBox", 1, true) then return true end
  return false
end

local function isSavePoint(obj, interactTypeName, clsName)
  if interactTypeName == "SavePoint" then return true end
  if type(clsName) == "string" and clsName:find("SavePoint", 1, true) then return true end
  return false
end

local function readGimmickInteractType(gimmick)
  local raw = util.readProp(gimmick, "m_MapInteractType")
  local decoded = util.decodeEnum(raw, INTERACT_TYPE_ENUM)
  if decoded and decoded ~= "None" then return decoded end
  local fallback = util.readProp(gimmick, "interactType")
  return util.decodeEnum(fallback, INTERACT_TYPE_ENUM) or decoded
end

local function firstValidFromArray(arr, ctx, errField)
  if arr == nil then return nil end
  local len, lenErr = util.arrayLen(arr)
  if lenErr then
    ctx.err(errField, lenErr)
    return nil
  end
  for index = 1, len do
    local item, itemErr = util.arrayAt(arr, index)
    if itemErr then
      ctx.err(errField, itemErr)
      return nil
    end
    if util.isValidObject(item) then
      return item
    end
  end
  return nil
end

local function readInteractTarget(pawn, ctx)
  local gimmickList, gimmickErr = util.readProp(pawn, "m_pInteractMapGimmickList")
  if gimmickErr then
    ctx.err("field.interact_target", gimmickErr)
  end
  local gimmick = firstValidFromArray(gimmickList, ctx, "field.interact_target")
  if gimmick then
    local clsName = util.classFullNameOf(gimmick)
    local interactType = readGimmickInteractType(gimmick)
    local uniqueIdRaw, uidErr = util.readProp(gimmick, "m_UniqueId")
    if uidErr then ctx.err("field.interact_target.unique_id", uidErr) end
    local loc, locErr = util.readActorLocation(gimmick)
    if locErr then ctx.err("field.interact_target.location", locErr) end

    local kind = "gimmick"
    if isTreasureBox(gimmick, interactType, clsName) then
      kind = "treasure_box"
    elseif isSavePoint(gimmick, interactType, clsName) then
      kind = "save_point"
    end

    local target = {
      kind = kind,
      unique_id = util.toInt(uniqueIdRaw) or json.null,
      event_id = readEventId(pawn, ctx),
      interact_type = interactType or json.null,
      location = loc or json.null,
    }

    if kind == "treasure_box" then
      local isOpen, openErr = util.readProp(gimmick, "m_IsOpen")
      if openErr then
        ctx.err("field.interact_target.is_open", openErr)
        target.is_open = json.null
      elseif type(isOpen) == "boolean" then
        target.is_open = isOpen
      else
        local fnOpen = util.callMethod(gimmick, "IsOpen")
        target.is_open = util.boolOrNull(fnOpen)
      end
    elseif kind == "save_point" then
      local canTp, tpErr = util.callMethod(gimmick, "CanTeleprot")
      if tpErr then
        ctx.err("field.interact_target.can_teleport", tpErr)
        target.can_teleport = json.null
      else
        target.can_teleport = util.boolOrNull(canTp)
      end
    end
    return target
  end

  local unitList, unitErr = util.readProp(pawn, "m_InteractMapUnitList")
  if unitErr then
    ctx.err("field.interact_target", unitErr)
  end
  local unit = firstValidFromArray(unitList, ctx, "field.interact_target")
  if unit then
    local uniqueIdRaw, uidErr = util.readProp(unit, "m_UniqueId")
    if uidErr then ctx.err("field.interact_target.unique_id", uidErr) end
    local rawInteract, intErr = util.readProp(unit, "m_MapInteractType")
    if intErr then ctx.err("field.interact_target.interact_type", intErr) end
    local loc, locErr = util.readActorLocation(unit)
    if locErr then ctx.err("field.interact_target.location", locErr) end

    local target = {
      kind = "npc",
      unique_id = util.toInt(uniqueIdRaw) or json.null,
      event_id = readEventId(pawn, ctx),
      interact_type = util.decodeEnum(rawInteract, INTERACT_TYPE_ENUM) or json.null,
      location = loc or json.null,
    }

    local shopEntry = util.readProp(unit, "ShopEntry")
    if shopEntry ~= nil then
      local shopId = util.toInt(util.readProp(shopEntry, "ShopID"))
      if shopId ~= nil and shopId >= 0 then
        target.shop_id = shopId
      end
    end
    local innEntry = util.readProp(unit, "InnEntry")
    if innEntry ~= nil then
      local innId = util.toInt(util.readProp(innEntry, "InnID"))
      if innId ~= nil and innId >= 0 then
        target.inn_id = innId
      end
    end
    return target
  end

  return json.null
end

local function readIntArray(arr, ctx, errField)
  local out = {}
  if arr == nil then return json.array(out) end
  local len, lenErr = util.arrayLen(arr)
  if lenErr then
    ctx.err(errField, lenErr)
    return json.array(out)
  end
  for index = 1, len do
    local item, itemErr = util.arrayAt(arr, index)
    if itemErr then
      ctx.err(errField, itemErr)
      break
    end
    local num = util.toInt(item)
    if num ~= nil then
      out[#out + 1] = num
    end
  end
  return json.array(out)
end

local function collectQuestTargets(ctx, cache)
  local mainQuestId = json.null
  local preferredQuestId = json.null
  local destMapIds = json.array({})

  local questSub = getCached(ctx, cache.gi, "questSub", "CPP_QuestSubsystem", true)
  if questSub then
    local rawMain, mainErr = util.readProp(questSub, "MainQuestId")
    if mainErr then
      ctx.err("field.quest_targets.main_quest_id", mainErr)
    else
      mainQuestId = util.toInt(rawMain) or json.null
    end
    local rawPref, prefErr = util.readProp(questSub, "PreferredQuestId")
    if prefErr then
      ctx.err("field.quest_targets.preferred_quest_id", prefErr)
    else
      preferredQuestId = util.toInt(rawPref) or json.null
    end
  else
    ctx.err("field.quest_targets", "找不到有效的 CPP_QuestSubsystem")
  end

  local scenSub = getCached(ctx, cache.gi, "scenSub", "CPP_ScenarioSubsystem", true)
  if scenSub then
    local rawDest, destErr = util.readProp(scenSub, "m_CurrentDestinationMapIdList")
    if destErr then
      ctx.err("field.quest_targets.destination_map_ids", destErr)
    else
      destMapIds = readIntArray(rawDest, ctx, "field.quest_targets.destination_map_ids")
    end
  else
    ctx.err("field.quest_targets", "找不到有效的 CPP_ScenarioSubsystem")
  end

  return {
    main_quest_id = mainQuestId,
    preferred_quest_id = preferredQuestId,
    destination_map_ids = destMapIds,
  }
end

local function collectUnlockedLandmarks(ctx, cache, allowWorldStore)
  local vm = getCached(ctx, cache.world, "landmarkVm", "VM_WldMapLandmarkList_C", allowWorldStore)
  if not vm then
    -- VM_WldMapLandmarkList_C is created once the player opens the World Map (M).
    return json.array({})
  end
  local rawList, listErr = util.readProp(vm, "AllLandmarkIdList")
  if listErr then
    ctx.err("field.unlocked_landmarks", listErr)
    return json.array({})
  end
  return readIntArray(rawList, ctx, "field.unlocked_landmarks")
end

local function collectMapActors(ctx, cache)
  local chests = {}
  local savePoints = {}
  local triggers = {}
  local npcs = {}

  local mapMgr = getCached(ctx, cache.world, "mapMgr", "CPP_MapManager", true)
  if not mapMgr then
    return json.array(chests), json.array(savePoints), {
      triggers = json.array(triggers),
      npcs = json.array(npcs),
    }, false
  end

  -- 1. Gimmicks: chests and save points in a single pass.
  local gimmickMgr, gMgrErr = util.readProp(mapMgr, "m_MapGimmickManager")
  if gMgrErr then
    ctx.err("field.chests", gMgrErr)
  elseif util.isValidObject(gimmickMgr) then
    local gimmickList, gListErr = util.readProp(gimmickMgr, "m_MapGimmickList")
    if gListErr then
      ctx.err("field.chests", gListErr)
    elseif gimmickList ~= nil then
      local len, lenErr = util.arrayLen(gimmickList)
      if lenErr then
        ctx.err("field.chests", lenErr)
      else
        for index = 1, len do
          local item, itemErr = util.arrayAt(gimmickList, index)
          if itemErr then
            ctx.err("field.chests", itemErr)
            break
          end
          if util.isValidObject(item) then
            local clsName = util.classFullNameOf(item)
            local interactType = readGimmickInteractType(item)

            if isTreasureBox(item, interactType, clsName) then
              local uniqueId = util.toInt(util.readProp(item, "m_UniqueId"))
              local chestId = util.toInt(util.readProp(item, "m_Id"))
              local isOpenVal, openErr = util.readProp(item, "m_IsOpen")
              if openErr then
                ctx.err("field.chests.is_open", openErr)
                isOpenVal = nil
              elseif type(isOpenVal) ~= "boolean" then
                local fnOpen = util.callMethod(item, "IsOpen")
                isOpenVal = type(fnOpen) == "boolean" and fnOpen or nil
              end

              local itemId = json.null
              local itemNum = json.null
              local money = json.null
              -- Always read m_TreasureBoxData (legacy m_ItemId can disagree with actual drop).
              local tbData, tbErr = util.readProp(item, "m_TreasureBoxData")
              if tbErr then
                ctx.err("field.chests.data", tbErr)
              elseif tbData ~= nil then
                itemId = util.toInt(util.readProp(tbData, "ItemId")) or json.null
                itemNum = util.toInt(util.readProp(tbData, "ItemNum")) or json.null
                money = util.toInt(util.readProp(tbData, "Money")) or json.null
              end

              local loc = util.readActorLocation(item)
              chests[#chests + 1] = {
                unique_id = uniqueId or json.null,
                chest_id = chestId or json.null,
                is_open = util.boolOrNull(isOpenVal),
                item_id = itemId,
                item_num = itemNum,
                money = money,
                location = loc or json.null,
              }
            elseif isSavePoint(item, interactType, clsName) then
              local uniqueId = util.toInt(util.readProp(item, "m_UniqueId"))
              local spType = util.decodeEnum(util.readProp(item, "m_Type"), SAVE_POINT_TYPE_ENUM)
              local isTeleport = util.readProp(item, "m_IsTeleport")
              local canTeleport, tpErr = util.callMethod(item, "CanTeleprot")
              if tpErr then
                ctx.err("field.save_points.can_teleport", tpErr)
                canTeleport = nil
              end
              local targetMapId = util.toInt(util.readProp(item, "m_TranditionMapId"))
              local targetPointId = util.toInt(util.readProp(item, "m_TranditionPoint"))
              local loc = util.readActorLocation(item)

              savePoints[#savePoints + 1] = {
                unique_id = uniqueId or json.null,
                type = spType or json.null,
                is_teleport = util.boolOrNull(isTeleport),
                can_teleport = util.boolOrNull(canTeleport),
                target_map_id = targetMapId or json.null,
                target_point_id = targetPointId or json.null,
                location = loc or json.null,
              }
            end
          end
        end
      end
    end
  end

  -- 2. Transition triggers.
  local triggerStorage, tStorErr = util.readProp(mapMgr, "m_pMapTransitionTriggerStorage")
  if tStorErr then
    ctx.err("field.transitions_and_npcs.triggers", tStorErr)
  elseif util.isValidObject(triggerStorage) then
    local triggerList, tListErr = util.readProp(triggerStorage, "m_TransitionTriggerList")
    if tListErr then
      ctx.err("field.transitions_and_npcs.triggers", tListErr)
    elseif triggerList ~= nil then
      local len, lenErr = util.arrayLen(triggerList)
      if lenErr then
        ctx.err("field.transitions_and_npcs.triggers", lenErr)
      else
        for index = 1, len do
          local trig, trigErr = util.arrayAt(triggerList, index)
          if trigErr then
            ctx.err("field.transitions_and_npcs.triggers", trigErr)
            break
          end
          if util.isValidObject(trig) then
            local uniqueId = util.toInt(util.readProp(trig, "m_UniqueId"))
            local targetMapId = util.toInt(util.readProp(trig, "m_MapId"))
            local pointId = util.toInt(util.readProp(trig, "m_PointID"))
            local isSameMap = util.readProp(trig, "m_IsSameMapTrasition")
            local isAuto = util.readProp(trig, "m_IsAutoTransition")
            local loc = util.readActorLocation(trig)
            triggers[#triggers + 1] = {
              unique_id = uniqueId or json.null,
              map_id = targetMapId or json.null,
              point_id = pointId or json.null,
              is_same_map = util.boolOrNull(isSameMap),
              is_auto = util.boolOrNull(isAuto),
              location = loc or json.null,
            }
          end
        end
      end
    end
  end

  -- 3. Visible NPCs from m_MapUnitStorage.m_MapUnits.
  local unitStorage, uStorErr = util.readProp(mapMgr, "m_MapUnitStorage")
  if uStorErr then
    ctx.err("field.transitions_and_npcs.npcs", uStorErr)
  elseif util.isValidObject(unitStorage) then
    local units, uListErr = util.readProp(unitStorage, "m_MapUnits")
    if uListErr then
      ctx.err("field.transitions_and_npcs.npcs", uListErr)
    elseif units ~= nil then
      local len, lenErr = util.arrayLen(units)
      if lenErr then
        ctx.err("field.transitions_and_npcs.npcs", lenErr)
      else
        for index = 1, len do
          local unit, unitErr = util.arrayAt(units, index)
          if unitErr then
            ctx.err("field.transitions_and_npcs.npcs", unitErr)
            break
          end
          if util.isValidObject(unit) then
            local objType = util.decodeEnum(util.readProp(unit, "m_MapObjectType"), OBJECT_TYPE_ENUM)
            local clsName = util.classFullNameOf(unit) or ""
            local isNpc = (objType == "NPC")
              or (clsName:find("NPC", 1, true) ~= nil and clsName:find("Player", 1, true) == nil)
            if isNpc then
              local isVisible = util.readProp(unit, "m_IsVisible")
              if isVisible == true then
                local uniqueId = util.toInt(util.readProp(unit, "m_UniqueId"))
                local unitId = util.toInt(util.readProp(unit, "m_MapUnitId"))
                local interactType = util.decodeEnum(util.readProp(unit, "m_MapInteractType"), INTERACT_TYPE_ENUM)
                local shopId = json.null
                local shopEntry = util.readProp(unit, "ShopEntry")
                if shopEntry ~= nil then
                  shopId = util.toInt(util.readProp(shopEntry, "ShopID")) or json.null
                end
                local innId = json.null
                local innEntry = util.readProp(unit, "InnEntry")
                if innEntry ~= nil then
                  innId = util.toInt(util.readProp(innEntry, "InnID")) or json.null
                end
                local loc = util.readActorLocation(unit)
                npcs[#npcs + 1] = {
                  unique_id = uniqueId or json.null,
                  unit_id = unitId or json.null,
                  interact_type = interactType or json.null,
                  shop_id = shopId,
                  inn_id = innId,
                  location = loc or json.null,
                }
              end
            end
          end
        end
      end
    end
  end

  return json.array(chests), json.array(savePoints), {
    triggers = json.array(triggers),
    npcs = json.array(npcs),
  }, true
end

function M.collect(ctx)
  local cache = ensureCache(ctx)
  local nowMs = ctx.clockMs or 0

  -- Detect World changes and immediately drop per-world object references.
  local worldName = nil
  local standaloneWorldObj = nil
  if type(ctx.world) == "string" then
    worldName = ctx.world
  else
    standaloneWorldObj = getCached(ctx, cache.world, "worldObj", "World", true)
    if standaloneWorldObj then
      worldName = util.fullNameOf(standaloneWorldObj)
    end
  end

  local worldChanged = cache.worldName ~= nil and worldName ~= cache.worldName
  if worldChanged then
    clearWorldState(cache, standaloneWorldObj)
  end
  cache.worldName = worldName

  -- 1. GameWidgetManagerSubsystem -> is_loading & in_dialog (GI singleton).
  local widgetMgr = getCached(ctx, cache.gi, "widgetMgr", "CPP_GameWidgetManagerSubsystem", true)

  local isLoading = json.null
  local loadMgr = nil
  if widgetMgr then
    local rawLoadMgr, loadMgrErr = util.readProp(widgetMgr, "m_LoadScrManager")
    if loadMgrErr then
      ctx.err("field.is_loading", loadMgrErr)
    elseif util.isValidObject(rawLoadMgr) then
      loadMgr = rawLoadMgr
    end
  end
  if not loadMgr then
    loadMgr = getCached(ctx, cache.gi, "loadScrMgr", "CPP_LoadingScreenManager", true)
  end
  if loadMgr then
    local val, err = util.readProp(loadMgr, "IsShowLoadingScreen")
    if err then
      ctx.err("field.is_loading", err)
    elseif type(val) == "boolean" then
      isLoading = val
    else
      ctx.err("field.is_loading", "IsShowLoadingScreen 不是布尔值")
    end
  else
    ctx.err("field.is_loading", "找不到有效的 CPP_LoadingScreenManager")
  end

  -- While loading, discard all per-world caches immediately to avoid dangling UObject refs.
  local allowWorldStore = isLoading ~= true
  if isLoading == true then
    clearWorldState(cache, standaloneWorldObj)
  end

  local inDialog = json.null
  local dialogMgr = nil
  if widgetMgr then
    local rawDlgMgr, dlgMgrErr = util.readProp(widgetMgr, "m_DialogManager")
    if dlgMgrErr then
      ctx.err("field.in_dialog", dlgMgrErr)
    elseif util.isValidObject(rawDlgMgr) then
      dialogMgr = rawDlgMgr
    end
  end
  if not dialogMgr then
    dialogMgr = getCached(ctx, cache.gi, "dialogMgr", "CPP_DialogManager", true)
  end
  if dialogMgr then
    local dlgWidget, dlgErr = util.readProp(dialogMgr, "DialogWidget")
    if dlgErr then
      ctx.err("field.in_dialog", dlgErr)
    else
      inDialog = util.isValidObject(dlgWidget)
    end
  else
    ctx.err("field.in_dialog", "找不到有效的 CPP_DialogManager")
  end

  -- 2. map_id & area_id via CPP_MiniMapSubsystem (fallback CPP_MapManager).
  local mapId = json.null
  local areaId = json.null
  local miniMapSub = getCached(ctx, cache.world, "miniMapSub", "CPP_MiniMapSubsystem", allowWorldStore)
  if miniMapSub then
    local propKey, pkErr = util.readProp(miniMapSub, "PropertyKey")
    if pkErr then
      ctx.err("field.map_id", pkErr)
    elseif propKey ~= nil then
      local rawMapId, idErr = util.readProp(propKey, "mapId")
      if idErr then
        ctx.err("field.map_id", idErr)
      else
        local intId = util.toInt(rawMapId)
        if intId ~= nil then
          mapId = intId
        else
          ctx.err("field.map_id", "PropertyKey.mapId 不是有限数字")
        end
      end
    end

    local areaMgr, amErr = util.readProp(miniMapSub, "AreaManager")
    if amErr then
      ctx.err("field.area_id", amErr)
    elseif util.isValidObject(areaMgr) then
      local rawAreaId, aErr = util.readProp(areaMgr, "CurrentAreaId")
      if aErr then
        ctx.err("field.area_id", aErr)
      else
        local intArea = util.toInt(rawAreaId)
        if intArea ~= nil then
          areaId = intArea
        else
          ctx.err("field.area_id", "AreaManager.CurrentAreaId 不是有限数字")
        end
      end
    end
  end

  if json.isNull(mapId) or json.isNull(areaId) then
    local mapMgr = getCached(ctx, cache.world, "mapMgr", "CPP_MapManager", allowWorldStore)
    if mapMgr then
      if json.isNull(mapId) then
        local rawMapId, err = util.callMethod(mapMgr, "GetMapId")
        if err then
          ctx.err("field.map_id", err)
        else
          mapId = util.toInt(rawMapId) or json.null
        end
      end
      if json.isNull(areaId) then
        local rawAreaId, err = util.callMethod(mapMgr, "GetMapAreaId")
        if err then
          ctx.err("field.area_id", err)
        else
          areaId = util.toInt(rawAreaId) or json.null
        end
      end
    elseif not miniMapSub then
      ctx.err("field.map_id", "找不到有效的 CPP_MiniMapSubsystem")
    end
  end

  -- 3. in_battle via CPP_UI_BattleSubsystem.
  local inBattle = json.null
  local btlSub = getCached(ctx, cache.world, "btlSub", "CPP_UI_BattleSubsystem", allowWorldStore)
  if btlSub then
    local val, err = util.readProp(btlSub, "bDuringBattle")
    if err then
      ctx.err("field.in_battle", err)
    elseif type(val) == "boolean" then
      inBattle = val
    else
      ctx.err("field.in_battle", "bDuringBattle 不是布尔值")
    end
  else
    ctx.err("field.in_battle", "找不到有效的 CPP_UI_BattleSubsystem")
  end

  -- 4. in_event via CPP_TalkEventSubsystem.
  local inEvent = json.null
  local talkSub = getCached(ctx, cache.gi, "talkSub", "CPP_TalkEventSubsystem", true)
  if talkSub then
    local evMgr, err = util.readProp(talkSub, "m_pEventManager")
    if err then
      ctx.err("field.in_event", err)
    else
      inEvent = util.isValidObject(evMgr)
    end
  else
    ctx.err("field.in_event", "找不到有效的 CPP_TalkEventSubsystem")
  end

  -- 5. in_menu via CPP_MainMenuSubsystem:IsOpenMenu() (fallback CPP_VM_MainMenu.SceneDataStack).
  local inMenu = json.null
  local mainMenuSub = getCached(ctx, cache.world, "mainMenuSub", "CPP_MainMenuSubsystem", allowWorldStore)
  if mainMenuSub then
    local openVal, openErr = util.callMethod(mainMenuSub, "IsOpenMenu")
    if openErr then
      ctx.err("field.in_menu", openErr)
    elseif type(openVal) == "boolean" then
      inMenu = openVal
    end
  end
  if json.isNull(inMenu) then
    local vmMenu = getCached(ctx, cache.world, "vmMainMenu", "CPP_VM_MainMenu", allowWorldStore)
    if vmMenu then
      local stack, stackErr = util.readProp(vmMenu, "SceneDataStack")
      if stackErr then
        ctx.err("field.in_menu", stackErr)
      else
        local len, lenErr = util.arrayLen(stack)
        if lenErr then
          ctx.err("field.in_menu", lenErr)
        else
          inMenu = len > 0
        end
      end
    elseif not mainMenuSub then
      ctx.err("field.in_menu", "找不到有效的 CPP_MainMenuSubsystem")
    end
  end

  -- 6. Player Pawn fields + MiniMap ViewModel direction/camera.
  local pc = getCached(ctx, cache.world, "pc", "PlayerController", allowWorldStore)
  local pawn = nil
  if pc then
    local rawPawn, pawnErr = util.readProp(pc, "Pawn")
    if pawnErr then
      ctx.err("field.player", pawnErr)
    elseif util.isValidObject(rawPawn) then
      pawn = rawPawn
    end
  end

  local playerLoc = json.null
  local playerVisible = json.null
  local playerUnitId = json.null
  local playerDirection = json.null
  local playerDirectionDeg = json.null
  local playerMovementMethod = json.null
  local playerSpeed2d = json.null
  local cameraDeg = json.null
  local cameraInternalRotation = json.null
  local interactTarget = json.null
  local enableUnitInput = false
  local enableMovement = false

  local miniMapVm = getCached(ctx, cache.world, "miniMapVm", "VM_MiniMap_C", allowWorldStore)
  if miniMapVm then
    local rawDir, dirErr = util.readProp(miniMapVm, "PlayerDirection")
    if dirErr then
      ctx.err("field.player.direction", dirErr)
    else
      playerDirection = util.decodeEnum(rawDir, DIRECTION_ENUM) or json.null
    end

    local rawDeg, degErr = util.readProp(miniMapVm, "PlayerDirectionDegree")
    if degErr then
      ctx.err("field.player.direction_deg", degErr)
    elseif util.isFiniteNumber(rawDeg) then
      playerDirectionDeg = util.round1(rawDeg)
    end

    local rawCamDeg, camErr = util.readProp(miniMapVm, "CameraDirectionDeg")
    if camErr then
      ctx.err("field.camera_deg", camErr)
    elseif util.isFiniteNumber(rawCamDeg) then
      cameraDeg = util.round1(rawCamDeg)
    end
  elseif pawn ~= nil then
    ctx.err("field.player.direction", "找不到有效的 VM_MiniMap_C")
  end

  if pawn then
    local loc, locErr = util.readActorLocation(pawn)
    if locErr then
      ctx.err("field.player.location", locErr)
    else
      playerLoc = loc
    end

    local vis, visErr = util.readProp(pawn, "m_IsVisible")
    if visErr then
      ctx.err("field.player.visible", visErr)
    elseif type(vis) == "boolean" then
      playerVisible = vis
    end

    local uid, uidErr = util.readProp(pawn, "m_MapUnitId")
    if uidErr then
      ctx.err("field.player.unit_id", uidErr)
    else
      playerUnitId = util.toInt(uid) or json.null
    end

    local rawMethod, methodErr = util.readProp(pawn, "m_MapMovementMethod")
    if methodErr then
      ctx.err("field.player.movement_method", methodErr)
    else
      playerMovementMethod = util.decodeEnum(rawMethod, MOVEMENT_METHOD_ENUM) or json.null
    end

    local enInput, enInputErr = util.readProp(pawn, "m_IsEnableUnitInput")
    if enInputErr then
      ctx.err("field.can_control", enInputErr)
    else
      enableUnitInput = enInput == true
    end

    local enMove, enMoveErr = util.readProp(pawn, "m_IsEnableMovement")
    if enMoveErr then
      ctx.err("field.can_control", enMoveErr)
    else
      enableMovement = enMove == true
    end

    local camRot, camRotErr = util.readProp(pawn, "m_MapCameraInternalRotation")
    if camRotErr then
      ctx.err("field.camera_internal_rotation", camRotErr)
    elseif type(camRot) == "boolean" then
      cameraInternalRotation = camRot
    end

    -- Speed 2D: prefer CharacterMovementComponent.Velocity; fall back to delta position / dt.
    local moveComp, mcErr = util.readProp(pawn, "m_pPawnMovementComponent")
    if mcErr then
      ctx.err("field.player.speed_2d", mcErr)
    elseif util.isValidObject(moveComp) then
      local vel, velErr = util.readProp(moveComp, "Velocity")
      if velErr then
        ctx.err("field.player.speed_2d", velErr)
      elseif vel ~= nil then
        local okVel, vx, vy = pcall(function() return vel.X, vel.Y end)
        if okVel and util.isFiniteNumber(vx) and util.isFiniteNumber(vy) then
          playerSpeed2d = util.round1(math.sqrt(vx * vx + vy * vy))
        elseif not okVel then
          ctx.err("field.player.speed_2d", vx)
        end
      end
    end

    local hasPlayerLoc = type(playerLoc) == "table" and not json.isNull(playerLoc)
    if json.isNull(playerSpeed2d) and hasPlayerLoc then
      if type(cache.prevLoc) == "table" and type(cache.prevClockMs) == "number" then
        local dtMs = nowMs - cache.prevClockMs
        if dtMs > 0 then
          local dx = playerLoc.x - cache.prevLoc.x
          local dy = playerLoc.y - cache.prevLoc.y
          playerSpeed2d = util.round1(math.sqrt(dx * dx + dy * dy) / (dtMs / 1000))
        end
      end
    end

    if hasPlayerLoc and allowWorldStore then
      cache.prevLoc = playerLoc
      cache.prevClockMs = nowMs
    else
      cache.prevLoc = nil
      cache.prevClockMs = nil
    end

    interactTarget = readInteractTarget(pawn, ctx)
  else
    cache.prevLoc = nil
    cache.prevClockMs = nil
  end

  local layerBlocked = isBlockingInputLayer(widgetMgr)
  local canControl = pawn ~= nil
    and playerVisible == true
    and enableUnitInput
    and enableMovement
    and type(playerLoc) == "table"
    and not json.isNull(playerLoc)
    and not isStagingLocation(playerLoc)
    and isLoading == false
    and inBattle == false
    and inEvent == false
    and inDialog == false
    and inMenu == false
    and not layerBlocked

  -- 7. Low-frequency refresh triggers.
  local mapIdKey = json.isNull(mapId) and nil or mapId
  local areaIdKey = json.isNull(areaId) and nil or areaId
  local mapIdChanged = cache.lastMapId ~= nil and mapIdKey ~= cache.lastMapId
  local areaIdChanged = cache.lastAreaId ~= nil and areaIdKey ~= cache.lastAreaId
  local eventEnded = cache.lastInEvent == true and inEvent == false
  local dialogEnded = cache.lastInDialog == true and inDialog == false

  local needMetaRefresh = cache.questTargets == nil
    or worldChanged
    or mapIdChanged
    or eventEnded
    or dialogEnded
    or (cache.lastMetaScanMs ~= nil and (nowMs - cache.lastMetaScanMs) >= LOW_FREQ_INTERVAL_MS)

  if needMetaRefresh then
    cache.questTargets = collectQuestTargets(ctx, cache)
    cache.unlockedLandmarks = collectUnlockedLandmarks(ctx, cache, allowWorldStore)
    cache.lastMetaScanMs = nowMs
  end

  if isLoading == true then
    cache.chests = json.array({})
    cache.savePoints = json.array({})
    cache.transitionsAndNpcs = { triggers = json.array({}), npcs = json.array({}) }
    cache.mapActorsReady = false
  else
    local needActorRefresh = (not cache.mapActorsReady)
      or worldChanged
      or mapIdChanged
      or areaIdChanged
      or eventEnded
      or dialogEnded
      or (cache.lastActorScanMs ~= nil and (nowMs - cache.lastActorScanMs) >= LOW_FREQ_INTERVAL_MS)

    if needActorRefresh then
      if mapIdKey == -1 then
        -- Title map has no CPP_MapManager; avoid rescanning every tick.
        cache.chests = json.array({})
        cache.savePoints = json.array({})
        cache.transitionsAndNpcs = { triggers = json.array({}), npcs = json.array({}) }
        cache.mapActorsReady = true
        cache.lastActorScanMs = nowMs
      else
        local chests, savePoints, transitionsAndNpcs, foundMgr = collectMapActors(ctx, cache)
        cache.chests = chests
        cache.savePoints = savePoints
        cache.transitionsAndNpcs = transitionsAndNpcs
        cache.mapActorsReady = foundMgr
        cache.lastActorScanMs = nowMs
      end
    end
  end

  if mapIdKey ~= nil then cache.lastMapId = mapIdKey end
  if areaIdKey ~= nil then cache.lastAreaId = areaIdKey end
  cache.lastInEvent = inEvent == true
  cache.lastInDialog = inDialog == true

  return {
    map_id = mapId,
    area_id = areaId,
    is_loading = isLoading,
    in_battle = inBattle,
    in_event = inEvent,
    in_dialog = inDialog,
    in_menu = inMenu,
    can_control = canControl,
    camera_deg = cameraDeg,
    camera_internal_rotation = cameraInternalRotation,
    player = {
      location = playerLoc,
      visible = playerVisible,
      unit_id = playerUnitId,
      direction = playerDirection,
      direction_deg = playerDirectionDeg,
      movement_method = playerMovementMethod,
      speed_2d = playerSpeed2d,
    },
    interact_target = interactTarget,
    quest_targets = cache.questTargets,
    unlocked_landmarks = cache.unlockedLandmarks,
    chests = cache.chests,
    save_points = cache.savePoints,
    transitions_and_npcs = cache.transitionsAndNpcs,
  }
end

local function parsePointArg(point, label)
  if type(point) ~= "table" or json.isNull(point) then
    return nil, label .. " 必须是包含 x, y, z 的对象"
  end
  local x = point.x ~= nil and point.x or point.X
  local y = point.y ~= nil and point.y or point.Y
  local z = point.z ~= nil and point.z or point.Z
  if not util.isFiniteNumber(x) or not util.isFiniteNumber(y) or not util.isFiniteNumber(z) then
    return nil, label .. " 的 x, y, z 必须是有限数字"
  end
  return { X = x, Y = y, Z = z }, nil
end

-- On-demand read-only NavMesh path query (docs/state-map/field.md §8.3).
-- args: { start = { x, y, z }, goal = { x, y, z } }
-- returns: { valid, partial, length, points = [ { x, y, z }, ... ] }
function M.commands.query_nav_path(ctx, args)
  if type(args) ~= "table" or json.isNull(args) then
    return nil, "query_nav_path 参数必须是对象"
  end
  local startVec, startErr = parsePointArg(args.start, "start")
  if startErr then return nil, startErr end
  local goalVec, goalErr = parsePointArg(args.goal, "goal")
  if goalErr then return nil, goalErr end

  local world = ctx.find("World")
  if not world then
    return nil, "找不到有效的 World，无法查询导航路径"
  end

  local navSys = ctx.find("NavigationSystemV1")
  if not navSys and type(ctx.staticFindObject) == "function" then
    navSys = ctx.staticFindObject("/Script/NavigationSystem.Default__NavigationSystemV1")
  end
  if not navSys then
    return nil, "找不到有效的 NavigationSystemV1，当前地图可能没有导航网格"
  end

  local navPath, callErr = util.callMethod(
    navSys,
    "FindPathToLocationSynchronously",
    world,
    startVec,
    goalVec
  )
  if callErr then
    return nil, "寻路查询失败：" .. callErr
  end
  if not util.isValidObject(navPath) then
    return {
      valid = false,
      partial = false,
      length = 0,
      points = json.array({}),
    }, nil
  end

  local isValidPath, validErr = util.callMethod(navPath, "IsValid")
  if validErr then
    return nil, "读取导航路径有效状态失败：" .. validErr
  end
  local isPartial, partialErr = util.callMethod(navPath, "IsPartial")
  if partialErr then
    isPartial = false
  end
  local rawLen, lenErr = util.callMethod(navPath, "GetPathLength")
  local pathLength = (not lenErr and util.isFiniteNumber(rawLen)) and util.round1(rawLen) or 0

  local points = {}
  local rawPoints, ptsErr = util.readProp(navPath, "PathPoints")
  if ptsErr then
    return nil, "读取导航路径点失败：" .. ptsErr
  end
  if rawPoints ~= nil then
    local count, countErr = util.arrayLen(rawPoints)
    if countErr then
      return nil, "读取导航路径长度失败：" .. countErr
    end
    for index = 1, count do
      local pt, ptErr = util.arrayAt(rawPoints, index)
      if ptErr then
        return nil, "读取导航路径点失败：" .. ptErr
      end
      local vec, vecErr = util.readVector(pt)
      if vecErr then
        return nil, "解析导航路径点失败：" .. vecErr
      end
      points[#points + 1] = vec
    end
  end

  return {
    valid = isValidPath == true,
    partial = isPartial == true,
    length = pathLength,
    points = json.array(points),
  }, nil
end

return M
