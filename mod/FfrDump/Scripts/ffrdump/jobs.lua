local M = {}

local TAG = "[FfrDump] "

local functions = {
  objects = "DumpAllObjects",
  usmap = "DumpUSMAP",
  jmap = "DumpJMAP",
  sdk = "GenerateSDK",
  uht = "GenerateUHTCompatibleHeaders",
}

local function emitLog(env, text)
  local log = env and env.log
  if log == nil then log = print end
  if type(log) ~= "function" then return end
  pcall(log, TAG .. text .. "\n")
end

local function formatStart(trigger)
  return "start id=" .. tostring(trigger.id or "-")
    .. " job=" .. tostring(trigger.job)
    .. " label=" .. tostring(trigger.label or "-")
end

local function formatDone(trigger, result)
  local parts = {
    "done id=" .. tostring(trigger.id or "-"),
    "job=" .. tostring(trigger.job),
    "ok=" .. tostring(result.ok),
  }
  local stats = result.stats
  if type(stats) == "table" then
    if stats.objectsScanned ~= nil then parts[#parts + 1] = "scanned=" .. tostring(stats.objectsScanned) end
    if stats.objectsDumped ~= nil then parts[#parts + 1] = "dumped=" .. tostring(stats.objectsDumped) end
    if stats.truncated ~= nil then parts[#parts + 1] = "truncated=" .. tostring(stats.truncated) end
    if stats.objectErrors ~= nil and stats.objectErrors > 0 then
      parts[#parts + 1] = "objectErrors=" .. tostring(stats.objectErrors)
    end
  end
  if result.error ~= nil then parts[#parts + 1] = "error=" .. tostring(result.error) end
  return table.concat(parts, " ")
end

local function normalizeResult(ok, result)
  if not ok then return { ok = false, error = tostring(result), files = {} } end
  return {
    ok = true,
    files = result and result.files or {},
    stats = result and result.stats or nil,
    error = nil,
  }
end

function M.run(trigger, environment)
  local env = environment or {}
  emitLog(env, formatStart(trigger))
  local ok, result = pcall(function()
    if trigger.job == "snapshot" then
      local snapshot = env.snapshot
      if type(snapshot) ~= "function" then error("快照函数不可用：snapshot") end
      return snapshot(trigger)
    end
    local name = functions[trigger.job]
    if not name then error("未知任务：" .. tostring(trigger.job)) end
    local dump = env[name] or rawget(_G, name)
    if type(dump) ~= "function" then error("UE4SS 函数不可用：" .. name) end
    dump()
    return { files = {}, stats = { objectsScanned = 0, objectsDumped = 0, truncated = false } }
  end)
  local normalized = normalizeResult(ok, result)
  emitLog(env, formatDone(trigger, normalized))
  return normalized
end

function M.start(trigger, environment, onDone)
  local env = environment or {}
  local completed = false
  local function finish(result)
    if completed then return end
    completed = true
    if type(onDone) == "function" then onDone(result) end
  end

  if trigger.job == "snapshot" and type(env.snapshotCreate) == "function"
    and type(env.schedule) == "function" then
    local stepper = nil
    local createdStepper = false
    emitLog(env, formatStart(trigger))
    env.schedule(function()
      if not createdStepper then
        createdStepper = true
        local createOk, created = pcall(env.snapshotCreate, trigger)
        if not createOk then
          local failed = normalizeResult(false, created)
          emitLog(env, formatDone(trigger, failed))
          finish(failed)
          return true
        end
        stepper = created
      end
      local stepOk, done, output = pcall(stepper.step)
      if not stepOk then
        if stepper and type(stepper.abort) == "function" then pcall(stepper.abort) end
        local failed = normalizeResult(false, done)
        emitLog(env, formatDone(trigger, failed))
        finish(failed)
        return true
      end
      if done then
        local succeeded = normalizeResult(true, output)
        emitLog(env, formatDone(trigger, succeeded))
        finish(succeeded)
        return true
      end
      return false
    end, function(scheduleErr)
      if stepper and type(stepper.abort) == "function" then pcall(stepper.abort) end
      local failed = normalizeResult(false, scheduleErr)
      emitLog(env, formatDone(trigger, failed))
      finish(failed)
    end)
    return
  end

  if type(env.schedule) == "function" then
    env.schedule(function()
      finish(M.run(trigger, env))
      return true
    end, function(scheduleErr)
      local failed = normalizeResult(false, scheduleErr)
      emitLog(env, formatDone(trigger, failed))
      finish(failed)
    end)
    return
  end

  finish(M.run(trigger, env))
end

return M
