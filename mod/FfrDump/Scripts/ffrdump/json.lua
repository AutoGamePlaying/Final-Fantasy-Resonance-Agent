local M = {}

local ARRAY = {}
local OBJECT = {}
local NULL = {}

M.null = setmetatable({}, NULL)

function M.array(value)
  return setmetatable(value or {}, ARRAY)
end

function M.object(value)
  return setmetatable(value or {}, OBJECT)
end

local escapes = {
  ['"'] = '\\"',
  ["\\"] = "\\\\",
  ["\b"] = "\\b",
  ["\f"] = "\\f",
  ["\n"] = "\\n",
  ["\r"] = "\\r",
  ["\t"] = "\\t",
}

local function quote(value)
  local out = { '"' }
  for index = 1, #value do
    local byte = value:byte(index)
    local char = value:sub(index, index)
    if escapes[char] then
      out[#out + 1] = escapes[char]
    elseif byte < 32 then
      out[#out + 1] = string.format("\\u%04x", byte)
    else
      out[#out + 1] = char
    end
  end
  out[#out + 1] = '"'
  return table.concat(out)
end

local function number(value)
  if value ~= value then return '"NaN"' end
  if value == math.huge then return '"Infinity"' end
  if value == -math.huge then return '"-Infinity"' end
  return tostring(value)
end

local function isArray(value, emptyTable)
  local mt = getmetatable(value)
  if mt == ARRAY then return true end
  if mt == OBJECT then return false end
  local largest = 0
  local count = 0
  for key in pairs(value) do
    if type(key) ~= "number" or key < 1 or key % 1 ~= 0 then return false end
    if key > largest then largest = key end
    count = count + 1
  end
  if count == 0 then return emptyTable == "array" end
  if largest ~= count then return false end
  for index = 1, largest do
    if value[index] == nil then return false end
  end
  return true
end

local function encode(value, options, seen)
  if value == nil or getmetatable(value) == NULL then return "null" end
  local kind = type(value)
  if kind == "string" then return quote(value) end
  if kind == "number" then return number(value) end
  if kind == "boolean" then return tostring(value) end
  if kind ~= "table" then error("JSON 不支持类型：" .. kind) end
  if seen[value] then error("JSON 不能编码循环引用") end
  seen[value] = true
  local out
  if isArray(value, options.emptyTable) then
    out = { "[" }
    for index = 1, #value do
      if index > 1 then out[#out + 1] = "," end
      out[#out + 1] = encode(value[index], options, seen)
    end
    out[#out + 1] = "]"
  else
    local keys = {}
    for key in pairs(value) do
      if type(key) ~= "string" and type(key) ~= "number" then
        error("JSON 对象键必须是字符串或数字")
      end
      keys[#keys + 1] = { name = tostring(key), original = key }
    end
    table.sort(keys, function(left, right) return left.name < right.name end)
    out = { "{" }
    for index, key in ipairs(keys) do
      if index > 1 then out[#out + 1] = "," end
      out[#out + 1] = quote(key.name)
      out[#out + 1] = ":"
      out[#out + 1] = encode(value[key.original], options, seen)
    end
    out[#out + 1] = "}"
  end
  seen[value] = nil
  return table.concat(out)
end

function M.encode(value, options)
  local actual = options or {}
  local emptyTable = actual.emptyTable or "object"
  if emptyTable ~= "object" and emptyTable ~= "array" then
    error("emptyTable 只能是 object 或 array")
  end
  return encode(value, { emptyTable = emptyTable }, {})
end

return M
