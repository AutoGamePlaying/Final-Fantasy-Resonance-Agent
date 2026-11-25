-- In-game hotkey handler for FfrDump:
--   F9: capture an immediate batched snapshot into OUT/hotkey/ with a job-id prefix
--       (e.g. hotkey-0003-20261125-023045-objects.jsonl).
--   F10: append a single JSON timestamp marker to OUT/hotkey/markers.jsonl without taking a snapshot.
-- Strictly read-only: passively listens for keybinds via UE4SS RegisterKeyBind and never injects
-- keys or mutates game state.

local json = require("ffrdump.json")
local jobs = require("ffrdump.jobs")
local protocol = require("ffrdump.protocol")

local M = {}

local TAG = "[FfrDump] "

local function defaultFs()
  return {
    open = io.open,
    remove = os.remove,
    rename = os.rename,
  }
end

local function trimTrailingSeparator(path)
  if path == "/" or path == "\\" then return path end
  if path:match("^[A-Za-z]:[/\\]$") then return path end
  return (path:gsub("[/\\]+$", ""))
end

local function join(directory, name)
  local clean = trimTrailingSeparator(directory)
  local separator = clean:find("\\", 1, true) and not clean:find("/", 1, true)
    and "\\" or "/"
  return clean .. separator .. name
end

local function temporaryPath(path)
  local base = path:match("^(.*)%.[^%.\\/]*$")
  return (base or path) .. ".tmp"
end

local function directoryExists(path, fs)
  if type(path) ~= "string" or path == "" then return false end
  local clean = trimTrailingSeparator(path)
  local ok, _, code = fs.rename(clean, clean)
  return ok == true or code == 13
end

local function writeAtomic(path, content, fs)
  local temporary = temporaryPath(path)
  local file, err = fs.open(temporary, "wb")
  if not file then error("无法打开输出文件：" .. tostring(err)) end
  local ok, writeError = file:write(content)
  if not ok then
    pcall(function() file:close() end)
    fs.remove(temporary)
    error("写输出文件失败：" .. tostring(writeError))
  end
  local closed, closeError = file:close()
  if not closed then
    fs.remove(temporary)
    error("关闭输出文件失败：" .. tostring(closeError))
  end
  fs.remove(path)
  local renamed, renameError = fs.rename(temporary, path)
  if not renamed then error("改名输出文件失败：" .. tostring(renameError)) end
end

local function appendLine(path, line, fs)
  local file, err = fs.open(path, "ab")
  if not file then error("无法打开标记文件：" .. tostring(err)) end
  local ok, writeError = file:write(line .. "\n")
  if not ok then
    pcall(function() file:close() end)
    error("写标记文件失败：" .. tostring(writeError))
  end
  local closed, closeError = file:close()
  if not closed then error("关闭标记文件失败：" .. tostring(closeError)) end
end

local function readFile(path, fs)
  local file = fs.open(path, "rb")
  if not file then return nil end
  local body = file:read("*a")
  file:close()
  return body
end

function M.formatSeq(seq)
  return string.format("%04d", math.floor(seq))
end

function M.formatUtcStamp(epochSeconds, dateFn)
  local fn = dateFn or os.date
  return tostring(fn("!%Y%m%d-%H%M%S", epochSeconds))
end

-- Avoid %z on Windows MSVCRT where strftime can emit non-UTF-8 localized timezone names.
function M.formatLocalTime(epochSeconds, dateFn)
  local fn = dateFn or os.date
  return tostring(fn("%Y-%m-%d %H:%M:%S", epochSeconds))
end

function M.snapshotId(seq, epochSeconds, dateFn)
  local label = "hotkey-" .. M.formatSeq(seq)
  local id = label .. "-" .. M.formatUtcStamp(epochSeconds, dateFn)
  return id, label, id .. "-"
end

function M.markerLine(fields)
  local world = type(fields.world) == "string" and fields.world ~= "" and fields.world or "<unknown>"
  return json.encode({
    seq = math.floor(fields.seq),
    localTime = tostring(fields.localTime),
    time = math.floor(fields.time),
    clock_ms = math.max(0, math.floor(fields.clock_ms)),
    world = world,
  })
end

local function countNonEmptyLines(text)
  if type(text) ~= "string" or text == "" then return 0 end
  local count = 0
  for line in (text .. "\n"):gmatch("(.-)\n") do
    if line:gsub("\r$", ""):match("%S") then count = count + 1 end
  end
  return count
end

local function parseSeqFile(body)
  local snapshotSeq = 0
  local markerSeq = 0
  if type(body) ~= "string" then return snapshotSeq, markerSeq end
  for line in (body .. "\n"):gmatch("(.-)\n") do
    local clean = line:gsub("\r$", "")
    local key, raw = clean:match("^([^=]+)=(%d+)$")
    if key == "snapshot" then
      snapshotSeq = tonumber(raw) or snapshotSeq
    elseif key == "marker" then
      markerSeq = tonumber(raw) or markerSeq
    end
  end
  return snapshotSeq, markerSeq
end

local function readWorldName(findFirstOf)
  if type(findFirstOf) ~= "function" then return "<unknown>" end
  local ok, world = pcall(findFirstOf, "World")
  if not ok or world == nil then return "<unknown>" end
  local okValid, valid = pcall(function() return world:IsValid() end)
  if not okValid or valid ~= true then return "<unknown>" end
  local okName, name = pcall(function() return world:GetFullName() end)
  if not okName or type(name) ~= "string" or name == "" then return "<unknown>" end
  return name
end

function M.create(options)
  local opts = options or {}
  local directory = opts.directory
  if type(directory) ~= "string" or directory == "" then
    error("必须提供热键输出目录 directory")
  end

  local fs = opts.fs or defaultFs()
  local log = type(opts.log) == "function" and opts.log or print
  local clock = opts.clock or os.clock
  local time = opts.time or os.time
  local dateFn = opts.date or os.date
  local isBusy = opts.isBusy or function() return false end
  local setBusy = opts.setBusy or function() end
  local onHeartbeat = opts.onHeartbeat or function() end
  local schedule = opts.schedule
  local runOnGameThread = opts.runOnGameThread or function(fn, onError)
    local ok, err = pcall(fn)
    if not ok and type(onError) == "function" then onError(err) end
  end
  local customFindFirstOf = opts.findFirstOf

  local startedClock = opts.startedClock ~= nil and opts.startedClock or clock()
  local seqPath = join(directory, "seq.txt")
  local markersPath = join(directory, "markers.jsonl")

  local loaded = false
  local snapshotSeq = 0
  local markerSeq = 0

  local function emit(text)
    pcall(log, TAG .. text .. "\n")
  end

  local function ensureLoaded()
    if loaded then return end
    loaded = true
    local seqBody = readFile(seqPath, fs)
    if seqBody then
      snapshotSeq, markerSeq = parseSeqFile(seqBody)
    end
    local markersBody = readFile(markersPath, fs)
    if markersBody then
      local existingLines = countNonEmptyLines(markersBody)
      if existingLines > markerSeq then markerSeq = existingLines end
    end
  end

  local function saveSeq()
    local content = "snapshot=" .. tostring(snapshotSeq) .. "\nmarker=" .. tostring(markerSeq) .. "\n"
    pcall(writeAtomic, seqPath, content, fs)
  end

  local function writeDone(trigger, prefix, startedAt, result)
    local ok, err = pcall(function()
      writeAtomic(join(directory, prefix .. "done.json"), protocol.completion({
        id = trigger.id,
        job = trigger.job,
        label = trigger.label,
        ok = result.ok,
        error = result.error,
        startedAt = startedAt,
        finishedAt = time(),
        files = result.files,
        stats = result.stats,
      }), fs)
    end)
    if not ok then emit("hotkey done error: " .. tostring(err)) end
    setBusy(false)
    pcall(onHeartbeat)
  end

  local controller = {
    directory = directory,
  }

  function controller.onSnapshotKey()
    if isBusy() then
      emit("hotkey F9 ignored: 正在执行其他任务，忽略本次热键快照")
      return false, "busy"
    end
    if not directoryExists(directory, fs) then
      emit("hotkey F9 error: 热键输出目录不存在：" .. directory)
      return false, "missing_dir"
    end
    ensureLoaded()
    snapshotSeq = snapshotSeq + 1
    saveSeq()
    local startedAt = time()
    local id, label, prefix = M.snapshotId(snapshotSeq, startedAt, dateFn)
    local trigger = { id = id, job = "snapshot", label = label }

    setBusy(true)
    pcall(onHeartbeat)
    emit("hotkey F9 start id=" .. id .. " label=" .. label)

    local env = {
      log = log,
      schedule = schedule,
      snapshotCreate = type(opts.snapshotCreate) == "function"
        and function() return opts.snapshotCreate(prefix, trigger) end
        or nil,
      snapshot = type(opts.snapshot) == "function"
        and function() return opts.snapshot(prefix, trigger) end
        or nil,
    }
    local ok, err = pcall(jobs.start, trigger, env, function(result)
      writeDone(trigger, prefix, startedAt, result)
    end)
    if not ok then
      emit("hotkey F9 error: " .. tostring(err))
      writeDone(trigger, prefix, startedAt, { ok = false, error = tostring(err), files = {} })
      return false, tostring(err)
    end
    return true, id
  end

  function controller.onMarkerKey()
    if not directoryExists(directory, fs) then
      emit("hotkey F10 error: 热键输出目录不存在：" .. directory)
      return false, "missing_dir"
    end
    ensureLoaded()
    markerSeq = markerSeq + 1
    saveSeq()
    local seq = markerSeq
    local now = time()
    local localTime = M.formatLocalTime(now, dateFn)
    local clockMs = math.max(0, math.floor((clock() - startedClock) * 1000))

    runOnGameThread(function()
      local finder = customFindFirstOf ~= nil and customFindFirstOf or rawget(_G, "FindFirstOf")
      local world = readWorldName(finder)
      local line = M.markerLine({
        seq = seq,
        localTime = localTime,
        time = now,
        clock_ms = clockMs,
        world = world,
      })
      appendLine(markersPath, line, fs)
      emit("hotkey F10 marker seq=" .. tostring(seq) .. " clock_ms=" .. tostring(clockMs) .. " world=" .. world)
    end, function(err)
      emit("hotkey F10 error: " .. tostring(err))
    end)
    return true, seq
  end

  return controller
end

function M.register(registerKeyBind, keyTable, controller, log)
  local emit = function(text)
    pcall(type(log) == "function" and log or print, TAG .. text .. "\n")
  end
  if type(registerKeyBind) ~= "function" then
    emit("hotkey unavailable: RegisterKeyBind 不可用")
    return false
  end
  if type(keyTable) ~= "table" or keyTable.F9 == nil or keyTable.F10 == nil then
    emit("hotkey unavailable: Key.F9 或 Key.F10 不可用")
    return false
  end
  local ok9, err9 = pcall(registerKeyBind, keyTable.F9, function()
    local ok, err = pcall(controller.onSnapshotKey)
    if not ok then emit("hotkey F9 error: " .. tostring(err)) end
  end)
  if not ok9 then
    emit("hotkey register F9 failed: " .. tostring(err9))
  end
  local ok10, err10 = pcall(registerKeyBind, keyTable.F10, function()
    local ok, err = pcall(controller.onMarkerKey)
    if not ok then emit("hotkey F10 error: " .. tostring(err)) end
  end)
  if not ok10 then
    emit("hotkey register F10 failed: " .. tostring(err10))
  end
  if ok9 and ok10 then
    emit("hotkeys registered: F9=snapshot F10=marker dir=" .. tostring(controller.directory))
  end
  return ok9 and ok10
end

return M
