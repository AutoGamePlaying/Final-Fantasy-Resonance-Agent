-- Shared pure-Lua helpers for FfrBridge v1 (UE4SS Lua 5.4 and fengari Lua 5.3).
-- Read-only helpers for safe UObject checks, TArray iteration, vectors, strings, and enums.

local json = require("ffrbridge.json")

local M = {}

M.TEXT_LIMIT = 200

local function utf8Step(value, index)
  local byte = value:byte(index)
  if byte < 0x80 then return 1 end
  if byte < 0xE0 then return 2 end
  if byte < 0xF0 then return 3 end
  return 4
end

function M.truncate(value, limit)
  local text = tostring(value)
  local maxChars = limit or M.TEXT_LIMIT
  if maxChars < 1 then return "" end
  local count = 0
  local index = 1
  local previousEnd = 0
  while index <= #text do
    count = count + 1
    local nextIndex = index + utf8Step(text, index)
    if count == maxChars then
      if nextIndex > #text then return text end
      return text:sub(1, previousEnd) .. "…"
    end
    previousEnd = nextIndex - 1
    index = nextIndex
  end
  return text
end

function M.isFiniteNumber(value)
  return type(value) == "number" and value == value and value ~= math.huge and value ~= -math.huge
end

function M.round1(value)
  local scaled = math.floor(value * 10 + 0.5)
  if scaled == 0 then return 0 end
  return scaled / 10
end

function M.toInt(value)
  if not M.isFiniteNumber(value) then return nil end
  return math.floor(value)
end

function M.boolOrNull(value)
  if type(value) == "boolean" then return value end
  return json.null
end

function M.isValidObject(obj)
  if obj == nil then return false, nil end
  local ok, valid = pcall(function() return obj:IsValid() end)
  if not ok then return false, tostring(valid) end
  return valid == true, nil
end

function M.fullNameOf(obj)
  local valid, validErr = M.isValidObject(obj)
  if validErr then return nil, validErr end
  if not valid then return nil, nil end
  local ok, name = pcall(function() return obj:GetFullName() end)
  if not ok then return nil, tostring(name) end
  if type(name) ~= "string" or name == "" then return nil, "GetFullName 返回空值" end
  return name, nil
end

function M.classFullNameOf(obj)
  local ok, cls = pcall(function() return obj:GetClass() end)
  if not ok then return nil, tostring(cls) end
  local name, nameErr = M.fullNameOf(cls)
  if nameErr then return nil, nameErr end
  if not name then return nil, "GetClass 返回无效类" end
  return name, nil
end

function M.toStr(value)
  if value == nil or json.isNull(value) then return nil, nil end
  if type(value) == "string" then return value, nil end
  local ok, text = pcall(function() return value:ToString() end)
  if ok and type(text) == "string" then return text, nil end
  local okStr, fallback = pcall(tostring, value)
  if not okStr then return nil, tostring(fallback) end
  return fallback, nil
end

function M.readProp(obj, key)
  if obj == nil then return nil, "对象为空" end
  local ok, value = pcall(function() return obj[key] end)
  if not ok then return nil, tostring(value) end
  return value, nil
end

function M.callMethod(obj, methodName, ...)
  if obj == nil then return nil, "对象为空" end
  local args = { ... }
  local argCount = select("#", ...)
  local ok, result = pcall(function()
    local fn = obj[methodName]
    if type(fn) ~= "function" then
      error("缺少方法 " .. tostring(methodName))
    end
    return fn(obj, table.unpack(args, 1, argCount))
  end)
  if not ok then return nil, tostring(result) end
  return result, nil
end

function M.arrayLen(arr)
  if arr == nil then return 0, nil end
  local okNum, num = pcall(function()
    if type(arr.GetArrayNum) == "function" then
      return arr:GetArrayNum()
    end
    return nil
  end)
  if okNum and M.isFiniteNumber(num) and num >= 0 then
    return math.floor(num), nil
  end
  local okLen, len = pcall(function() return #arr end)
  if okLen and M.isFiniteNumber(len) and len >= 0 then
    return math.floor(len), nil
  end
  return nil, tostring(not okNum and num or len)
end

function M.arrayAt(arr, index)
  if arr == nil then return nil, "数组为空" end
  local ok, value = pcall(function() return arr[index] end)
  if not ok then return nil, tostring(value) end
  return value, nil
end

function M.readVector(vec)
  if vec == nil then return nil, "向量为空" end
  local ok, x, y, z = pcall(function() return vec.X, vec.Y, vec.Z end)
  if not ok then return nil, tostring(x) end
  if not M.isFiniteNumber(x) or not M.isFiniteNumber(y) or not M.isFiniteNumber(z) then
    return nil, "向量坐标不是有限数字"
  end
  return {
    x = M.round1(x),
    y = M.round1(y),
    z = M.round1(z),
  }, nil
end

function M.readActorLocation(actor)
  if actor == nil then return nil, "Actor 为空" end
  local okLoc, loc = pcall(function() return actor:K2_GetActorLocation() end)
  if not okLoc then return nil, tostring(loc) end
  if loc == nil then return nil, "K2_GetActorLocation 返回空值" end
  return M.readVector(loc)
end

function M.decodeEnum(value, tableByInt)
  if value == nil or json.isNull(value) then return nil end
  if type(value) == "number" then
    if not M.isFiniteNumber(value) then return nil end
    local mapped = tableByInt[math.floor(value)]
    if mapped then return mapped end
    return "Unknown_" .. tostring(math.floor(value))
  end
  if type(value) == "string" then
    local stripped = value:match("::([^:]+)$") or value
    if stripped == "" then return nil end
    return stripped
  end
  local str = M.toStr(value)
  if type(str) == "string" and str ~= "" then
    return str:match("::([^:]+)$") or str
  end
  return nil
end

return M
