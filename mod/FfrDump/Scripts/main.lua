local TAG = "[FfrDump] "
local lastErrors = {}
local busy = false

local function logError(kind, err)
  local now = os.time()
  if lastErrors[kind] and now - lastErrors[kind] < 60 then return end
  lastErrors[kind] = now
  print(TAG .. kind .. " " .. tostring(err) .. "\n")
end

local function dirname(path)
  return path:match("^(.*)[/\\][^/\\]*$")
end

local function locate()
  local info = debug.getinfo(1, "S")
  local source = info and info.source or ""
  local script = source:sub(1, 1) == "@" and source:sub(2) or ""
  local scripts = dirname(script)
  local mod = scripts and dirname(scripts) or nil
  local mods = mod and dirname(mod) or nil
  local ue4ss = mods and dirname(mods) or nil
  if ue4ss and ue4ss ~= "" then
    ue4ss = ue4ss:gsub("/", "\\")
    scripts = scripts:gsub("/", "\\")
    return ue4ss, scripts
  end
  if type(IterateGameDirectories) ~= "function" then return nil end
  local ok, directories = pcall(IterateGameDirectories)
  if not ok or not directories then return nil end
  local okPath, win64 = pcall(function()
    local directory = directories.Game.Binaries.Win64
    return directory.__absolute_path or tostring(directory)
  end)
  if not okPath or type(win64) ~= "string" or win64 == "" then return nil end
  ue4ss = win64:gsub("/", "\\") .. "\\ue4ss"
  return ue4ss, ue4ss .. "\\Mods\\FfrDump\\Scripts"
end

local ue4ss, scripts = locate()
if not ue4ss then
  print(TAG .. "error 无法确定 UE4SS 目录\n")
  return
end

package.path = scripts .. "\\?.lua;" .. package.path
local protocol = require("ffrdump.protocol")
local snapshot = require("ffrdump.snapshot")
local jobs = require("ffrdump.jobs")
local hotkey = require("ffrdump.hotkey")

local OUT = ue4ss .. "\\ffr-out"
local HOTKEY = OUT .. "\\hotkey"
local TRIGGER = OUT .. "\\trigger.txt"
local HEARTBEAT = OUT .. "\\heartbeat.json"

local function temporaryPath(path)
  local base = path:match("^(.*)%.[^%.\\/]*$")
  return (base or path) .. ".tmp"
end

local function writeAtomic(path, content)
  local temporary = temporaryPath(path)
  local file, err = io.open(temporary, "wb")
  if not file then error("无法打开输出文件：" .. tostring(err)) end
  local ok, writeError = file:write(content)
  if not ok then
    file:close()
    os.remove(temporary)
    error("写输出文件失败：" .. tostring(writeError))
  end
  local closed, closeError = file:close()
  if not closed then error("关闭输出文件失败：" .. tostring(closeError)) end
  os.remove(path)
  local renamed, renameError = os.rename(temporary, path)
  if not renamed then error("改名输出文件失败：" .. tostring(renameError)) end
end

local function directoryExists(path)
  local ok, _, code = os.rename(path, path)
  return ok == true or code == 13
end

local function heartbeat()
  local ok, err = pcall(function()
    writeAtomic(HEARTBEAT, protocol.heartbeat(os.time(), busy))
  end)
  if not ok then logError("heartbeat", err) end
end

local function jobDirectory(id)
  return OUT .. "\\jobs\\" .. id
end

local function failureDone(id, raw, message)
  if not id or not directoryExists(jobDirectory(id)) then return end
  local now = os.time()
  writeAtomic(jobDirectory(id) .. "\\done.json", protocol.completion({
    id = id,
    job = raw and raw.job or "invalid",
    label = raw and raw.label or "invalid",
    ok = false,
    error = message,
    startedAt = now,
    finishedAt = now,
    files = {},
  }))
end

local function finishTask(trigger, startedAt, result)
  local doneOk, doneError = pcall(function()
    writeAtomic(jobDirectory(trigger.id) .. "\\done.json", protocol.completion({
      id = trigger.id,
      job = trigger.job,
      label = trigger.label,
      ok = result.ok,
      error = result.error,
      startedAt = startedAt,
      finishedAt = os.time(),
      files = result.files,
      stats = result.stats,
    }))
  end)
  if not doneOk then logError("done", doneError) end
  busy = false
  heartbeat()
end

-- Drive a multi-step job across game-thread callbacks with a 20 ms yield between batches
-- so a large snapshot never blocks the UE5 main loop for seconds at a time.
local function scheduleOnGameThread(stepFn, onError)
  local finished = false
  local inFlight = false
  local function fail(err)
    if finished then return end
    finished = true
    if type(onError) == "function" then onError(err) end
  end
  local function runStep()
    if finished then return end
    inFlight = true
    local ok, err = pcall(ExecuteInGameThread, function()
      local stepOk, done = pcall(stepFn)
      inFlight = false
      if not stepOk then
        fail(done)
      elseif done then
        finished = true
      end
    end)
    if not ok then
      inFlight = false
      fail(err)
    end
  end
  runStep()
  if finished then return end
  local loopOk, loopErr = pcall(LoopAsync, 20, function()
    if finished then return true end
    if inFlight then return false end
    runStep()
    return finished
  end)
  if not loopOk then fail(loopErr) end
end

local function startTask(trigger)
  local startedAt = os.time()
  jobs.start(trigger, {
    log = print,
    schedule = scheduleOnGameThread,
    snapshotCreate = function()
      return snapshot.create({ directory = jobDirectory(trigger.id), batched = true })
    end,
    snapshot = function()
      return snapshot.run({ directory = jobDirectory(trigger.id) })
    end,
  }, function(result)
    finishTask(trigger, startedAt, result)
  end)
end

local function pollTrigger()
  local file = io.open(TRIGGER, "rb")
  if not file then return end
  local body = file:read("*a")
  file:close()
  local removed, removeError = os.remove(TRIGGER)
  if not removed then
    logError("trigger", "无法删除触发文件：" .. tostring(removeError))
    return
  end
  local trigger, parseError, raw, parsedId = protocol.parseTrigger(body)
  local validId = parsedId or raw and protocol.isValidId(raw.id) and raw.id or nil
  if not trigger then
    logError("trigger", parseError)
    local ok, err = pcall(failureDone, validId, raw, parseError)
    if not ok then logError("done", err) end
    return
  end
  if not directoryExists(jobDirectory(trigger.id)) then
    logError("trigger", "任务目录不存在：" .. trigger.id)
    return
  end
  busy = true
  heartbeat()
  local ok, err = pcall(startTask, trigger)
  if not ok then
    logError("task", err)
    finishTask(trigger, os.time(), { ok = false, error = tostring(err), files = {} })
  end
end

local function runOnGameThread(fn, onError)
  local ok, err = pcall(ExecuteInGameThread, function()
    local callOk, callErr = pcall(fn)
    if not callOk and type(onError) == "function" then onError(callErr) end
  end)
  if not ok and type(onError) == "function" then onError(err) end
end

local hotkeyController = hotkey.create({
  directory = HOTKEY,
  log = print,
  isBusy = function() return busy end,
  setBusy = function(value) busy = value end,
  onHeartbeat = heartbeat,
  schedule = scheduleOnGameThread,
  runOnGameThread = runOnGameThread,
  snapshotCreate = function(filePrefix)
    return snapshot.create({
      directory = HOTKEY,
      filePrefix = filePrefix,
      batched = true,
    })
  end,
  snapshot = function(filePrefix)
    return snapshot.run({
      directory = HOTKEY,
      filePrefix = filePrefix,
    })
  end,
})

print(TAG .. "loaded out=" .. OUT .. "\n")
hotkey.register(rawget(_G, "RegisterKeyBind"), rawget(_G, "Key"), hotkeyController, print)
heartbeat()
local tick = 0
LoopAsync(1000, function()
  tick = tick + 1
  local ok, err = pcall(function()
    if tick % 5 == 0 then heartbeat() end
    if not busy then pollTrigger() end
  end)
  if not ok then logError("tick", err) end
  return false
end)
