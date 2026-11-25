local json = require("ffrdump.json")

local M = {}

M.STRING_LIMIT = 512
M.ERROR_LIMIT = 200
M.JOBS = {
  snapshot = true,
  objects = true,
  usmap = true,
  jmap = true,
  sdk = true,
  uht = true,
}

local function utf8Step(value, index)
  local byte = value:byte(index)
  if byte < 0x80 then return 1 end
  if byte < 0xE0 then return 2 end
  if byte < 0xF0 then return 3 end
  return 4
end

function M.truncate(value, limit)
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

local function validLabel(label)
  return type(label) == "string" and #label >= 1 and #label <= 32
    and label:match("^[a-z0-9%-]+$") ~= nil
end

local function splitId(id)
  if type(id) ~= "string" then return nil end
  local date, time, label, job = id:match("^(%d+)%-(%d+)%-([a-z0-9%-]+)%-([a-z]+)$")
  if not date or #date ~= 8 or #time ~= 6 or not validLabel(label) or not M.JOBS[job] then
    return nil
  end
  if id ~= date .. "-" .. time .. "-" .. label .. "-" .. job then return nil end
  return { id = id, job = job, label = label }
end

function M.isValidId(id)
  return splitId(id) ~= nil
end

function M.parseTrigger(body)
  local values = {}
  for line in (tostring(body) .. "\n"):gmatch("(.-)\n") do
    line = line:gsub("\r$", "")
    if line ~= "" then
      local key, value = line:match("^([^=]+)=(.*)$")
      if not key then return nil, "触发文件有一行不是 键=值：" .. line, values end
      if values[key] ~= nil then return nil, "触发文件有重复键：" .. key, values end
      if key ~= "id" and key ~= "job" and key ~= "label" then
        return nil, "触发文件有未知键：" .. key, values
      end
      values[key] = value
    end
  end
  for _, key in ipairs({ "id", "job", "label" }) do
    if values[key] == nil then return nil, "触发文件缺少键：" .. key, values end
  end
  local parsed = splitId(values.id)
  local validId = parsed and parsed.id or nil
  if not M.JOBS[values.job] then return nil, "未知任务：" .. values.job, values, validId end
  if not validLabel(values.label) then return nil, "场面名格式不合法", values, validId end
  if not parsed then return nil, "任务号格式不合法", values, validId end
  if parsed.job ~= values.job then
    return nil, "任务和任务号结尾不一致", values, validId
  end
  if parsed.label ~= values.label then
    return nil, "场面名和任务号不一致", values, validId
  end
  return parsed
end

function M.heartbeat(time, busy)
  return json.encode({ mod = "FfrDump", version = 1, time = time, busy = busy })
end

function M.completion(fields)
  local done = {
    id = fields.id,
    job = fields.job,
    label = fields.label,
    ok = fields.ok,
    error = fields.error == nil and json.null
      or M.truncate(tostring(fields.error), M.ERROR_LIMIT),
    startedAt = fields.startedAt,
    finishedAt = fields.finishedAt,
    files = json.array(fields.files or {}),
    stats = fields.stats or { objectsScanned = 0, objectsDumped = 0, truncated = false },
  }
  return json.encode(done)
end

return M
