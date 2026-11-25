-- Minimal pure-Lua JSON encoder and RFC 8259 decoder for FfrBridge.
-- Compatible with Lua 5.3 (fengari tests) and Lua 5.4 (UE4SS).

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

function M.isNull(value)
  return value == nil or getmetatable(value) == NULL
end

function M.isArray(value)
  return type(value) == "table" and getmetatable(value) == ARRAY
end

function M.isObject(value)
  return type(value) == "table" and getmetatable(value) == OBJECT
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
  if value == 0 then return "0" end
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

-- RFC 8259 decoder

local MAX_DEPTH = 64

local unescapes = {
  ['"'] = '"',
  ["\\"] = "\\",
  ["/"] = "/",
  b = "\b",
  f = "\f",
  n = "\n",
  r = "\r",
  t = "\t",
}

local function codepointToUtf8(cp)
  if cp <= 0x7F then
    return string.char(cp)
  elseif cp <= 0x7FF then
    return string.char(
      0xC0 + math.floor(cp / 0x40),
      0x80 + (cp % 0x40)
    )
  elseif cp <= 0xFFFF then
    return string.char(
      0xE0 + math.floor(cp / 0x1000),
      0x80 + (math.floor(cp / 0x40) % 0x40),
      0x80 + (cp % 0x40)
    )
  else
    return string.char(
      0xF0 + math.floor(cp / 0x40000),
      0x80 + (math.floor(cp / 0x1000) % 0x40),
      0x80 + (math.floor(cp / 0x40) % 0x40),
      0x80 + (cp % 0x40)
    )
  end
end

local function parseError(message, pos)
  error({ message = message .. "（位置 " .. tostring(pos) .. "）" }, 0)
end

local function skipWhitespace(text, pos)
  local len = #text
  while pos <= len do
    local byte = text:byte(pos)
    if byte == 32 or byte == 9 or byte == 10 or byte == 13 then
      pos = pos + 1
    else
      break
    end
  end
  return pos
end

local function parseHex4(text, pos)
  local hex = text:sub(pos, pos + 3)
  if #hex < 4 or not hex:match("^[0-9a-fA-F]+$") then
    parseError("无效的 Unicode 转义", pos)
  end
  return tonumber(hex, 16), pos + 4
end

local function parseString(text, pos)
  local len = #text
  pos = pos + 1 -- skip opening '"'
  local out = {}
  local chunkStart = pos
  while pos <= len do
    local byte = text:byte(pos)
    if byte == 34 then -- '"'
      if pos > chunkStart then out[#out + 1] = text:sub(chunkStart, pos - 1) end
      return table.concat(out), pos + 1
    elseif byte == 92 then -- '\\'
      if pos > chunkStart then out[#out + 1] = text:sub(chunkStart, pos - 1) end
      pos = pos + 1
      if pos > len then parseError("字符串转义未结束", pos) end
      local esc = text:sub(pos, pos)
      local mapped = unescapes[esc]
      if mapped then
        out[#out + 1] = mapped
        pos = pos + 1
      elseif esc == "u" then
        local code, nextPos = parseHex4(text, pos + 1)
        pos = nextPos
        if code >= 0xD800 and code <= 0xDBFF then
          if text:sub(pos, pos + 1) ~= "\\u" then
            parseError("缺少低代理项", pos)
          end
          local low, afterLow = parseHex4(text, pos + 2)
          if low < 0xDC00 or low > 0xDFFF then
            parseError("无效的低代理项", pos + 2)
          end
          pos = afterLow
          code = 0x10000 + (code - 0xD800) * 0x400 + (low - 0xDC00)
        elseif code >= 0xDC00 and code <= 0xDFFF then
          parseError("孤立的低代理项", pos - 4)
        end
        out[#out + 1] = codepointToUtf8(code)
      else
        parseError("无效的转义字符 \\" .. esc, pos)
      end
      chunkStart = pos
    elseif byte < 32 then
      parseError("字符串包含未转义的控制字符", pos)
    else
      pos = pos + 1
    end
  end
  parseError("字符串未闭合", pos)
end

local function parseNumber(text, pos)
  local start = pos
  local len = #text
  if text:byte(pos) == 45 then pos = pos + 1 end -- '-'
  if pos > len then parseError("数字格式不完整", start) end
  local first = text:byte(pos)
  if first == 48 then -- '0'
    pos = pos + 1
  elseif first >= 49 and first <= 57 then -- '1'..'9'
    pos = pos + 1
    while pos <= len do
      local b = text:byte(pos)
      if b >= 48 and b <= 57 then pos = pos + 1 else break end
    end
  else
    parseError("无效的数字", pos)
  end
  if pos <= len and text:byte(pos) == 46 then -- '.'
    pos = pos + 1
    local fracStart = pos
    while pos <= len do
      local b = text:byte(pos)
      if b >= 48 and b <= 57 then pos = pos + 1 else break end
    end
    if pos == fracStart then parseError("小数点后缺少数字", pos) end
  end
  if pos <= len then
    local b = text:byte(pos)
    if b == 101 or b == 69 then -- 'e' or 'E'
      pos = pos + 1
      if pos <= len then
        local sign = text:byte(pos)
        if sign == 43 or sign == 45 then pos = pos + 1 end
      end
      local expStart = pos
      while pos <= len do
        local d = text:byte(pos)
        if d >= 48 and d <= 57 then pos = pos + 1 else break end
      end
      if pos == expStart then parseError("指数部分缺少数字", pos) end
    end
  end
  local slice = text:sub(start, pos - 1)
  local value = tonumber(slice)
  if value == nil then parseError("无法解析数字：" .. slice, start) end
  return value, pos
end

local parseValue

local function parseArray(text, pos, depth)
  if depth > MAX_DEPTH then parseError("JSON 嵌套层数过深", pos) end
  pos = skipWhitespace(text, pos + 1)
  local result = M.array({})
  if text:byte(pos) == 93 then return result, pos + 1 end -- ']'
  local index = 1
  while true do
    local item
    item, pos = parseValue(text, pos, depth + 1)
    result[index] = item
    index = index + 1
    pos = skipWhitespace(text, pos)
    local byte = text:byte(pos)
    if byte == 93 then return result, pos + 1 end -- ']'
    if byte ~= 44 then parseError("数组元素间缺少逗号或右括号", pos) end -- ','
    pos = skipWhitespace(text, pos + 1)
  end
end

local function parseObject(text, pos, depth)
  if depth > MAX_DEPTH then parseError("JSON 嵌套层数过深", pos) end
  pos = skipWhitespace(text, pos + 1)
  local result = M.object({})
  if text:byte(pos) == 125 then return result, pos + 1 end -- '}'
  while true do
    if text:byte(pos) ~= 34 then parseError("对象键必须是字符串", pos) end
    local key
    key, pos = parseString(text, pos)
    pos = skipWhitespace(text, pos)
    if text:byte(pos) ~= 58 then parseError("对象键值间缺少冒号", pos) end -- ':'
    pos = skipWhitespace(text, pos + 1)
    local value
    value, pos = parseValue(text, pos, depth + 1)
    result[key] = value
    pos = skipWhitespace(text, pos)
    local byte = text:byte(pos)
    if byte == 125 then return result, pos + 1 end -- '}'
    if byte ~= 44 then parseError("对象成员间缺少逗号或右括号", pos) end -- ','
    pos = skipWhitespace(text, pos + 1)
  end
end

parseValue = function(text, pos, depth)
  pos = skipWhitespace(text, pos)
  if pos > #text then parseError("意外的 JSON 结尾", pos) end
  local byte = text:byte(pos)
  if byte == 123 then return parseObject(text, pos, depth) end -- '{'
  if byte == 91 then return parseArray(text, pos, depth) end -- '['
  if byte == 34 then return parseString(text, pos) end -- '"'
  if byte == 45 or (byte >= 48 and byte <= 57) then return parseNumber(text, pos) end
  if text:sub(pos, pos + 3) == "true" then return true, pos + 4 end
  if text:sub(pos, pos + 4) == "false" then return false, pos + 5 end
  if text:sub(pos, pos + 3) == "null" then return M.null, pos + 4 end
  parseError("意外的字符：" .. text:sub(pos, pos), pos)
end

function M.decode(text)
  if type(text) ~= "string" then
    return nil, "JSON 输入不是字符串"
  end
  local ok, value, pos = pcall(function()
    local decoded, nextPos = parseValue(text, 1, 1)
    nextPos = skipWhitespace(text, nextPos)
    if nextPos <= #text then
      parseError("JSON 末尾有多余字符", nextPos)
    end
    return decoded, nextPos
  end)
  if not ok then
    if type(value) == "table" and type(value.message) == "string" then
      return nil, value.message
    end
    return nil, tostring(value)
  end
  return value, nil
end

return M
