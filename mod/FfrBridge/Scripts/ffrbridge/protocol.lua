-- File-bridge protocol v1 controller for FfrBridge v1.
-- Manages session/seq state publishing, command parsing, topic command dispatch,
-- deduplication, and ack replies.

local bridgeIo = require("ffrbridge.io")
local json = require("ffrbridge.json")
local state = require("ffrbridge.state")

local M = {}

M.VERSION = 1
M.SESSION_LENGTH = 6
M.MAX_SEEN_IDS = 256
M.ERROR_INTERVAL_SECONDS = 60

local ALPHABET = "abcdefghijklmnopqrstuvwxyz0123456789"
local seeded = false

local function seedRandom()
  if seeded then return end
  seeded = true
  local addrHex = tostring({}):match("0x(%x+)") or "0"
  local addr = tonumber(addrHex:sub(-7), 16) or 0
  local timePart = (os.time() or 0) % 10000000
  local clockPart = math.floor((os.clock() or 0) * 1000000) % 10000000
  local seed = (timePart * 131 + clockPart + (addr % 10000000)) % 2147483647
  math.randomseed(seed)
  math.random()
  math.random()
end

function M.generateSession(randomFn)
  local nextInt = randomFn
  if not nextInt then
    seedRandom()
    nextInt = math.random
  end
  local chars = {}
  local alphabetLen = #ALPHABET
  for index = 1, M.SESSION_LENGTH do
    local pick = nextInt(1, alphabetLen)
    chars[index] = ALPHABET:sub(pick, pick)
  end
  return table.concat(chars)
end

function M.encodeState(session, seq, clockMs, data)
  return json.encode({
    v = M.VERSION,
    session = session,
    seq = math.floor(seq),
    clock_ms = math.max(0, math.floor(clockMs)),
    data = data,
  })
end

function M.encodeAck(id, ok, result, errMsg)
  local success = ok == true
  return json.encode({
    v = M.VERSION,
    id = id,
    ok = success,
    result = success and (result ~= nil and result or json.object({})) or json.null,
    error = success and json.null or tostring(errMsg or "未知错误"),
  })
end

local function extractFallbackId(raw)
  if type(raw) ~= "string" then return nil end
  local id = raw:match('"id"%s*:%s*"([^"\\\r\n]+)"')
  if type(id) == "string" and id ~= "" then return id end
  return nil
end

local function parseJsonId(raw)
  local decoded, decodeErr = json.decode(raw)
  if decodeErr then
    return nil, "JSON 解析失败：" .. tostring(decodeErr)
  end
  if type(decoded) ~= "table" or json.isNull(decoded) or json.isArray(decoded) then
    return nil, "顶层必须是 JSON 对象"
  end
  local id = decoded.id
  if type(id) ~= "string" or id == "" then
    return nil, "缺少有效的 id"
  end
  return id, nil
end

function M.parseCommand(raw)
  local decoded, decodeErr = json.decode(raw)
  if decodeErr then
    return nil, "命令 JSON 解析失败：" .. tostring(decodeErr), extractFallbackId(raw)
  end
  if type(decoded) ~= "table" or json.isNull(decoded) or json.isArray(decoded) then
    return nil, "命令顶层必须是 JSON 对象", nil
  end
  local id = decoded.id
  if type(id) ~= "string" or id == "" then
    return nil, "命令缺少有效的 id", nil
  end
  if decoded.v ~= M.VERSION then
    return nil, "不支持的协议版本：" .. tostring(decoded.v), id
  end
  local op = decoded.op
  if type(op) ~= "string" or op == "" then
    return nil, "命令缺少有效的 op", id
  end
  return {
    v = decoded.v,
    id = id,
    op = op,
    args = decoded.args,
  }, nil, id
end

local function fallbackData(err)
  return {
    world = json.null,
    map = json.null,
    player_controller_class = json.null,
    pawn_class = json.null,
    game_mode_class = json.null,
    game_instance_class = json.null,
    pawn_location = json.null,
    field = json.null,
    battle = json.null,
    ui = json.null,
    party = json.null,
    errors = json.array({ "sample: " .. tostring(err) }),
    bridge_version = state.BRIDGE_VERSION,
  }
end

local function registerModuleCommands(registry, names, topicName, commands)
  if commands == nil then return end
  if type(commands) ~= "table" then
    error("主题 " .. topicName .. " 的 commands 必须是表")
  end
  local opNames = {}
  for opName in pairs(commands) do
    opNames[#opNames + 1] = opName
  end
  table.sort(opNames, function(left, right) return tostring(left) < tostring(right) end)
  for _, opName in ipairs(opNames) do
    local handler = commands[opName]
    if type(opName) ~= "string" or opName == "" then
      error("主题 " .. topicName .. " 注册了无效的命令名")
    end
    if type(handler) ~= "function" then
      error("主题 " .. topicName .. " 的命令 " .. opName .. " 必须是函数")
    end
    if opName == "ping" then
      error("命令名冲突：ping 为内置保留命令，不能在主题 " .. topicName .. " 中注册")
    end
    if registry[opName] then
      error(
        "命令名冲突："
          .. opName
          .. " 同时注册在主题 "
          .. registry[opName].topicName
          .. " 和 "
          .. topicName
      )
    end
    registry[opName] = { topicName = topicName, fn = handler }
    names[#names + 1] = opName
  end
end

local function buildCommandRegistry(topicList, extraModules)
  local registry = {}
  local names = {}
  for _, entry in ipairs(topicList) do
    registerModuleCommands(registry, names, entry.name, entry.mod.commands)
  end
  if type(extraModules) == "table" then
    for _, entry in ipairs(extraModules) do
      registerModuleCommands(registry, names, entry.name, entry.mod.commands)
    end
  end
  table.sort(names)
  return registry, names
end

function M.create(options)
  local opts = options or {}
  local directory = opts.directory
  if type(directory) ~= "string" or directory == "" then
    error("必须提供桥目录 directory")
  end

  local fs = opts.fs or bridgeIo.defaultFs()
  local clock = opts.clock or os.clock
  local time = opts.time or os.time
  local log = opts.log or print
  local tag = opts.tag or "[FfrBridge] "
  local errorInterval = opts.errorInterval or M.ERROR_INTERVAL_SECONDS
  local maxSeenIds = opts.maxSeenIds or M.MAX_SEEN_IDS

  local sampler = state.create({
    topics = opts.topics,
    findFirstOf = opts.findFirstOf,
    findAllOf = opts.findAllOf,
    staticFindObject = opts.staticFindObject,
  })
  local extraModules = {}
  if opts.inputModule ~= false then
    local inputMod = opts.inputModule or require("ffrbridge.topics.input")
    local inputName = (type(inputMod.name) == "string" and inputMod.name ~= "")
      and inputMod.name or "input"
    extraModules[#extraModules + 1] = { name = inputName, mod = inputMod }
  end
  local commandRegistry, commandNames = buildCommandRegistry(sampler.topicList, extraModules)

  local statePath = bridgeIo.join(directory, "state.json")
  local commandPath = bridgeIo.join(directory, "command.json")
  local ackPath = bridgeIo.join(directory, "ack.json")

  local session = opts.session or M.generateSession(opts.random)
  local startedClock = clock()
  local seq = 0
  local seenIds = {}
  local seenOrder = {}
  local lastErrors = {}

  local instance = {
    session = session,
    directory = directory,
    commands = commandNames,
  }

  local function currentClockMs()
    return math.max(0, math.floor((clock() - startedClock) * 1000))
  end

  function instance.getSeq()
    return seq
  end

  function instance.logError(kind, err)
    local now = time()
    if lastErrors[kind] and now - lastErrors[kind] < errorInterval then return end
    lastErrors[kind] = now
    log(tag .. kind .. " " .. tostring(err) .. "\n")
  end

  function instance.hasDirectory()
    return bridgeIo.directoryExists(directory, fs)
  end

  local function rememberId(id)
    if seenIds[id] then return end
    seenIds[id] = true
    seenOrder[#seenOrder + 1] = id
    while #seenOrder > maxSeenIds do
      local oldest = table.remove(seenOrder, 1)
      seenIds[oldest] = nil
    end
  end

  local function readFileSafe(path)
    local openOk, file, openErr, openCode = pcall(fs.open, path, "rb")
    if not openOk then
      return nil, tostring(file), nil
    end
    if not file then
      return nil, openErr, openCode
    end
    local readOk, body, readErr = pcall(function() return file:read("*a") end)
    pcall(function() file:close() end)
    if not readOk then
      return nil, tostring(body), nil
    end
    if body == nil then
      return nil, readErr or "读取返回空内容", nil
    end
    return body, nil, nil
  end

  local function warnIfOverwritingOtherAck(nextId)
    local existingRaw = readFileSafe(ackPath)
    if existingRaw == nil then return end
    local existingId = parseJsonId(existingRaw)
    if existingId and existingId ~= nextId then
      log(
        tag
          .. "ack_overwrite 覆盖未取走的 ack.json（旧 id="
          .. existingId
          .. "，新 id="
          .. tostring(nextId)
          .. "）\n"
      )
    end
  end

  local function writeAck(id, ok, result, errMsg)
    local encodeOk, payload = pcall(M.encodeAck, id, ok, result, errMsg)
    if not encodeOk then
      instance.logError("ack_encode", payload)
      payload = M.encodeAck(id, false, nil, "命令结果 JSON 编码失败：" .. tostring(payload))
    end
    warnIfOverwritingOtherAck(id)
    local writeOk, writeErr = pcall(bridgeIo.writeAtomic, ackPath, payload, fs)
    if not writeOk then
      instance.logError("ack", writeErr)
    end
  end

  local function deleteCommandFile()
    local removeOk, removed, removeErr, removeCode = pcall(fs.remove, commandPath)
    if not removeOk then
      instance.logError("command_remove", "无法删除 command.json：" .. tostring(removed))
      return
    end
    if not removed and removeErr and removeCode ~= 2 then
      instance.logError("command_remove", "无法删除 command.json：" .. tostring(removeErr))
    end
  end

  -- Re-read command.json and delete only when its id still matches expectedId.
  local function removeCommandIfIdMatches(expectedId)
    local currentRaw, readErr, readCode = readFileSafe(commandPath)
    if currentRaw == nil then
      if readCode == 2 then return end
      instance.logError(
        "command_remove_read",
        "删除前重读 command.json 失败：" .. tostring(readErr or "未知错误")
      )
      return
    end
    local currentId, parseErr = parseJsonId(currentRaw)
    if not currentId then
      instance.logError(
        "command_remove_parse",
        "删除前解析 command.json 失败：" .. tostring(parseErr)
      )
      return
    end
    if currentId ~= expectedId then return end
    deleteCommandFile()
  end

  -- For malformed commands without a valid JSON id, delete only if the file is unchanged.
  local function removeCommandIfRawMatches(expectedRaw)
    local currentRaw, readErr, readCode = readFileSafe(commandPath)
    if currentRaw == nil then
      if readCode == 2 then return end
      instance.logError(
        "command_remove_read",
        "删除前重读 command.json 失败：" .. tostring(readErr or "未知错误")
      )
      return
    end
    if currentRaw ~= expectedRaw then return end
    deleteCommandFile()
  end

  function instance.writeState()
    local nextSeq = seq + 1
    local clockMs = currentClockMs()
    local okSample, sampled = pcall(sampler.sample, clockMs, nextSeq)
    local data = okSample and sampled or fallbackData(sampled)
    local payload = M.encodeState(session, nextSeq, clockMs, data)
    bridgeIo.writeAtomic(statePath, payload, fs)
    seq = nextSeq
    return nextSeq, data
  end

  function instance.handleCommand()
    local raw = bridgeIo.readFile(commandPath, fs)
    if raw == nil then return false end

    local cmd, parseErr, id = M.parseCommand(raw)
    local hasParsedId = cmd ~= nil or parseJsonId(raw) ~= nil
    local function cleanupCommand()
      if hasParsedId and id then
        removeCommandIfIdMatches(id)
      else
        removeCommandIfRawMatches(raw)
      end
    end

    if id and seenIds[id] then
      cleanupCommand()
      return false
    end

    if not cmd then
      instance.logError("command", parseErr)
      if id then
        rememberId(id)
        writeAck(id, false, nil, parseErr)
      end
      cleanupCommand()
      return true
    end

    rememberId(cmd.id)
    if cmd.op == "ping" then
      writeAck(cmd.id, true, { pong = true, seq = seq }, nil)
    else
      local entry = commandRegistry[cmd.op]
      if not entry then
        writeAck(cmd.id, false, nil, "不认识的命令 " .. cmd.op)
      else
        local cmdCtx = sampler.makeCommandContext(entry.topicName, currentClockMs())
        cmdCtx.directory = directory
        cmdCtx.fs = fs
        cmdCtx.log = function(message)
          log(tag .. tostring(message) .. "\n")
        end
        cmdCtx.nowMs = currentClockMs
        cmdCtx.scheduleDelayed = opts.scheduleDelayed
        cmdCtx.minIntervalMs = opts.minIntervalMs
        cmdCtx.freshFind = function(className)
          return sampler.makeCommandContext(entry.topicName, currentClockMs()).find(className)
        end
        cmdCtx.sampleUi = function()
          local uiMod = sampler.topics.ui
          if type(uiMod) ~= "table" or type(uiMod.collect) ~= "function" then
            local okReq, loaded = pcall(require, "ffrbridge.topics.ui")
            if not okReq or type(loaded) ~= "table" or type(loaded.collect) ~= "function" then
              return nil
            end
            uiMod = loaded
          end
          local uiCtx = sampler.makeCommandContext("ui", currentClockMs())
          local okSample, sampled = pcall(uiMod.collect, uiCtx)
          if okSample and type(sampled) == "table" and not json.isNull(sampled) then
            return sampled
          end
          return nil
        end
        local cmdArgs = (type(cmd.args) == "table" and not json.isNull(cmd.args)) and cmd.args or {}
        local okCall, result, errMsg = pcall(entry.fn, cmdCtx, cmdArgs)
        if not okCall then
          writeAck(cmd.id, false, nil, "命令 " .. cmd.op .. " 执行失败：" .. tostring(result))
        elseif errMsg ~= nil then
          writeAck(cmd.id, false, nil, tostring(errMsg))
        elseif result == nil then
          writeAck(cmd.id, false, nil, "命令 " .. cmd.op .. " 未返回结果")
        else
          writeAck(cmd.id, true, result, nil)
        end
      end
    end
    cleanupCommand()
    return true
  end

  function instance.tick()
    if not instance.hasDirectory() then return false end
    local okState, stateErr = pcall(instance.writeState)
    if not okState then
      instance.logError("state", stateErr)
    end
    local okCmd, cmdErr = pcall(instance.handleCommand)
    if not okCmd then
      instance.logError("command", cmdErr)
    end
    return true
  end

  return instance
end

return M
