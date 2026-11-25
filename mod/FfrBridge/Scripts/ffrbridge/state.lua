-- Read-only v1 game state sampler for FINAL FANTASY RESONANCE.
-- Keeps all v0 top-level fields for compatibility with ffr-module and collects
-- structured state from the four v1 topic modules (field, battle, ui, party).

local json = require("ffrbridge.json")
local util = require("ffrbridge.util")

local M = {}

M.BRIDGE_VERSION = "1"
M.ERROR_LIMIT = 200
M.TOPIC_ORDER = { "field", "battle", "ui", "party" }

M.round1 = util.round1
M.truncate = util.truncate

local NOT_FOUND = {}

-- Extract the short map name from a UE World full name such as
-- "World /Game/Maps/Field01.Field01" -> "Field01".
function M.extractMap(worldFullName)
  if type(worldFullName) ~= "string" then return nil end
  local trimmed = worldFullName:match("^%s*(.-)%s*$")
  if trimmed == "" then return nil end
  local path = trimmed:match("^%S+%s+(.+)$") or trimmed
  local shortName = path:match("([^./:\\]+)$")
  if not shortName or shortName == "" then return nil end
  return shortName
end

function M.loadTopics()
  local loaded = {}
  for _, topicName in ipairs(M.TOPIC_ORDER) do
    loaded[topicName] = require("ffrbridge.topics." .. topicName)
  end
  return loaded
end

local function normalizeTopics(rawTopics)
  local byName = {}
  local ordered = {}
  if rawTopics == nil then
    rawTopics = M.loadTopics()
  end
  if type(rawTopics) ~= "table" then
    error("topics 必须是表")
  end

  if rawTopics[1] ~= nil then
    for index, mod in ipairs(rawTopics) do
      if type(mod) ~= "table" or type(mod.name) ~= "string" or mod.name == "" then
        error("第 " .. tostring(index) .. " 个主题缺少有效的 name")
      end
      if byName[mod.name] then
        error("重复的主题名：" .. mod.name)
      end
      byName[mod.name] = mod
      ordered[#ordered + 1] = { name = mod.name, mod = mod }
    end
    return byName, ordered
  end

  local seen = {}
  for _, name in ipairs(M.TOPIC_ORDER) do
    local mod = rawTopics[name]
    if mod ~= nil then
      if type(mod) ~= "table" then
        error("主题 " .. name .. " 必须是表")
      end
      byName[name] = mod
      ordered[#ordered + 1] = { name = name, mod = mod }
      seen[name] = true
    end
  end

  local extras = {}
  for name in pairs(rawTopics) do
    if not seen[name] then
      extras[#extras + 1] = tostring(name)
    end
  end
  table.sort(extras)
  for _, name in ipairs(extras) do
    local mod = rawTopics[name]
    if type(mod) ~= "table" then
      error("主题 " .. name .. " 必须是表")
    end
    byName[name] = mod
    ordered[#ordered + 1] = { name = name, mod = mod }
  end

  return byName, ordered
end

local function makeFinder(opts, recordError)
  local findCache = {}

  local function resolveFindFirstOf()
    if opts.findFirstOf ~= nil then return opts.findFirstOf end
    return rawget(_G, "FindFirstOf")
  end

  local function resolveFindAllOf()
    if opts.findAllOf ~= nil then return opts.findAllOf end
    return rawget(_G, "FindAllOf")
  end

  local function resolveStaticFindObject()
    if opts.staticFindObject ~= nil then return opts.staticFindObject end
    return rawget(_G, "StaticFindObject")
  end

  local function findRaw(ueClass)
    local cached = findCache[ueClass]
    if cached == NOT_FOUND then return nil, nil end
    if cached ~= nil then return cached, nil end

    local findFirstOf = resolveFindFirstOf()
    if type(findFirstOf) ~= "function" then
      findCache[ueClass] = NOT_FOUND
      return nil, "FindFirstOf 不可用"
    end
    local ok, obj = pcall(findFirstOf, ueClass)
    if not ok then
      findCache[ueClass] = NOT_FOUND
      return nil, tostring(obj)
    end
    local valid, validErr = util.isValidObject(obj)
    if validErr then
      findCache[ueClass] = NOT_FOUND
      return nil, validErr
    end
    if not valid then
      findCache[ueClass] = NOT_FOUND
      return nil, nil
    end
    findCache[ueClass] = obj
    return obj, nil
  end

  local function ctxFind(className)
    if type(className) ~= "string" or className == "" then return nil end
    local obj, err = findRaw(className)
    if err and err ~= "FindFirstOf 不可用" then
      recordError("find." .. className, err)
    end
    return obj
  end

  local function ctxFindAll(className)
    local list = {}
    if type(className) ~= "string" or className == "" then return list end
    local findAllOf = resolveFindAllOf()
    if type(findAllOf) ~= "function" then return list end
    local ok, raw = pcall(findAllOf, className)
    if not ok then
      recordError("findAll." .. className, raw)
      return list
    end
    if type(raw) ~= "table" then return list end
    for index = 1, #raw do
      local item = raw[index]
      local valid = util.isValidObject(item)
      if valid then
        list[#list + 1] = item
      end
    end
    return list
  end

  local function ctxStaticFindObject(path)
    if type(path) ~= "string" or path == "" then return nil end
    local staticFind = resolveStaticFindObject()
    if type(staticFind) ~= "function" then return nil end
    local ok, obj = pcall(staticFind, path)
    if not ok then return nil end
    local valid = util.isValidObject(obj)
    if not valid then return nil end
    return obj
  end

  return {
    findRaw = findRaw,
    find = ctxFind,
    findAll = ctxFindAll,
    staticFindObject = ctxStaticFindObject,
  }
end

function M.create(options)
  local opts = options or {}
  local topicsByName, topicList = normalizeTopics(opts.topics)
  local topicCaches = {}
  if type(opts.caches) == "table" then
    for key, value in pairs(opts.caches) do
      topicCaches[key] = value
    end
  end
  for _, entry in ipairs(topicList) do
    if type(topicCaches[entry.name]) ~= "table" then
      topicCaches[entry.name] = {}
    end
  end

  local tick = math.max(0, math.floor(opts.tick or 0))
  local lastWorld = nil
  local lastMap = nil

  local sampler = {
    topics = topicsByName,
    topicList = topicList,
  }

  function sampler.getCache(topicName)
    if type(topicCaches[topicName]) ~= "table" then
      topicCaches[topicName] = {}
    end
    return topicCaches[topicName]
  end

  function sampler.makeCommandContext(topicName, clockMs)
    local cmdErrors = {}
    local function recordError(field, message)
      local ok, formatted = pcall(function()
        return util.truncate(tostring(field) .. ": " .. tostring(message), M.ERROR_LIMIT)
      end)
      cmdErrors[#cmdErrors + 1] = ok and formatted or "command: 未知错误"
    end
    local finder = makeFinder(opts, recordError)
    return {
      find = finder.find,
      findAll = finder.findAll,
      staticFindObject = finder.staticFindObject,
      err = recordError,
      cache = sampler.getCache(topicName),
      clockMs = math.max(0, math.floor(clockMs or 0)),
      tick = tick,
      world = lastWorld,
      map = lastMap,
      errors = cmdErrors,
    }
  end

  function sampler.sample(clockMs, tickOverride)
    if tickOverride ~= nil then
      tick = math.max(0, math.floor(tickOverride))
    else
      tick = tick + 1
    end
    local currentClockMs = math.max(0, math.floor(clockMs or 0))

    local errors = {}
    local function recordError(field, message)
      local ok, formatted = pcall(function()
        return util.truncate(tostring(field) .. ": " .. tostring(message), M.ERROR_LIMIT)
      end)
      errors[#errors + 1] = ok and formatted or "unknown: 未知错误"
    end

    local finder = makeFinder(opts, recordError)

    local function findV0(ueClass, field)
      local obj, err = finder.findRaw(ueClass)
      if err then
        recordError(field, err)
        return nil
      end
      if not obj then
        recordError(field, "找不到有效的 " .. ueClass)
        return nil
      end
      return obj
    end

    local world = json.null
    local map = json.null
    local worldStr = nil
    local mapStr = nil
    local worldObj = findV0("World", "world")
    if worldObj then
      local name, err = util.fullNameOf(worldObj)
      if name then
        world = name
        worldStr = name
        local shortName = M.extractMap(name)
        if shortName then
          map = shortName
          mapStr = shortName
        end
      else
        recordError("world", err or "无法获取 World 完整名")
      end
    end

    local playerControllerClass = json.null
    local pc = findV0("PlayerController", "player_controller")
    if pc then
      local clsName, err = util.classFullNameOf(pc)
      if clsName then
        playerControllerClass = clsName
      else
        recordError("player_controller_class", err)
      end
    end

    local pawnClass = json.null
    local pawnLocation = json.null
    if pc then
      local okPawn, pawn = pcall(function() return pc.Pawn end)
      if not okPawn then
        recordError("pawn", pawn)
      else
        local valid, validErr = util.isValidObject(pawn)
        if validErr then
          recordError("pawn", validErr)
        elseif not valid then
          recordError("pawn", "找不到有效的 Pawn")
        else
          local clsName, clsErr = util.classFullNameOf(pawn)
          if clsName then
            pawnClass = clsName
          else
            recordError("pawn_class", clsErr)
          end

          local okLoc, loc = pcall(function() return pawn:K2_GetActorLocation() end)
          if not okLoc then
            recordError("pawn_location", loc)
          elseif loc == nil then
            recordError("pawn_location", "K2_GetActorLocation 返回空值")
          else
            local okCoords, x, y, z = pcall(function() return loc.X, loc.Y, loc.Z end)
            if not okCoords then
              recordError("pawn_location", x)
            elseif not util.isFiniteNumber(x) or not util.isFiniteNumber(y) or not util.isFiniteNumber(z) then
              recordError("pawn_location", "Pawn 坐标不是有限数字")
            else
              pawnLocation = {
                x = util.round1(x),
                y = util.round1(y),
                z = util.round1(z),
              }
            end
          end
        end
      end
    else
      recordError("pawn", "无 PlayerController，无法获取 Pawn")
    end

    local gameModeClass = json.null
    local gameMode = findV0("GameModeBase", "game_mode")
    if gameMode then
      local clsName, err = util.classFullNameOf(gameMode)
      if clsName then
        gameModeClass = clsName
      else
        recordError("game_mode_class", err)
      end
    end

    local gameInstanceClass = json.null
    local gameInstance = findV0("GameInstance", "game_instance")
    if gameInstance then
      local clsName, err = util.classFullNameOf(gameInstance)
      if clsName then
        gameInstanceClass = clsName
      else
        recordError("game_instance_class", err)
      end
    end

    lastWorld = worldStr
    lastMap = mapStr

    local data = {
      world = world,
      map = map,
      player_controller_class = playerControllerClass,
      pawn_class = pawnClass,
      game_mode_class = gameModeClass,
      game_instance_class = gameInstanceClass,
      pawn_location = pawnLocation,
      field = json.null,
      battle = json.null,
      ui = json.null,
      party = json.null,
      errors = json.array(errors),
      bridge_version = M.BRIDGE_VERSION,
    }

    for _, entry in ipairs(topicList) do
      local topicName = entry.name
      local topicMod = entry.mod
      if type(topicMod.collect) == "function" then
        local ctx = {
          find = finder.find,
          findAll = finder.findAll,
          staticFindObject = finder.staticFindObject,
          err = recordError,
          cache = sampler.getCache(topicName),
          clockMs = currentClockMs,
          tick = tick,
          world = worldStr,
          map = mapStr,
        }
        local okCollect, collected = pcall(topicMod.collect, ctx)
        if not okCollect then
          recordError(topicName, collected)
          data[topicName] = json.null
        elseif collected == nil or json.isNull(collected) then
          data[topicName] = json.null
        elseif type(collected) ~= "table" then
          recordError(topicName, "collect 返回值必须是表或 nil")
          data[topicName] = json.null
        else
          data[topicName] = collected
        end
      else
        recordError(topicName, "主题缺少 collect 函数")
        data[topicName] = json.null
      end
    end

    return data
  end

  return sampler
end

function M.sample(options)
  local opts = options or {}
  local sampler = M.create(opts)
  return sampler.sample(opts.clockMs or 0, opts.tick or 1)
end

return M
