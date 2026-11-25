local json = require("ffrdump.json")
local protocol = require("ffrdump.protocol")

local M = {}

M.LIMITS = {
  maxDepth = 3,
  maxItems = 32,
  maxString = 512,
  maxError = 200,
}

local numeric = {
  Int8Property = true,
  Int16Property = true,
  IntProperty = true,
  Int64Property = true,
  UInt16Property = true,
  UIntProperty = true,
  UInt32Property = true,
  UInt64Property = true,
  FloatProperty = true,
  DoubleProperty = true,
  ByteProperty = true,
}

local objectReferences = {
  ObjectProperty = true,
  ObjectPtrProperty = true,
  WeakObjectProperty = true,
  SoftObjectProperty = true,
  ClassProperty = true,
  SoftClassProperty = true,
  LazyObjectProperty = true,
}

local strings = {
  StrProperty = true,
  NameProperty = true,
  TextProperty = true,
}

local containers = {
  StructProperty = true,
  ArrayProperty = true,
  MapProperty = true,
  SetProperty = true,
}

local function isReadableKind(kind)
  return numeric[kind]
    or strings[kind]
    or objectReferences[kind]
    or containers[kind]
    or kind == "EnumProperty"
    or kind == "BoolProperty"
end

local function isValidObject(reader, object)
  if object == nil then return false end
  if type(reader.isValid) ~= "function" then return true end
  local ok, valid = pcall(reader.isValid, object)
  return ok and valid == true
end

local function finiteNumber(value)
  if type(value) ~= "number" then error("属性值不是数字") end
  if value ~= value then return "NaN" end
  if value == math.huge then return "Infinity" end
  if value == -math.huge then return "-Infinity" end
  return value
end

local function errorValue(err)
  return protocol.truncate("<error: " .. tostring(err) .. ">", M.LIMITS.maxError)
end

M.errorValue = errorValue

local function stringify(reader, value)
  if type(value) == "string" then return value end
  if reader.toString then return tostring(reader.toString(value)) end
  return tostring(value)
end

function M.auto(reader, value, depth)
  if value == nil then return json.null end
  local kind = type(value)
  if kind == "number" then return finiteNumber(value) end
  if kind == "boolean" or kind == "string" then
    if kind == "string" then return protocol.truncate(value, M.LIMITS.maxString) end
    return value
  end
  if reader.objectName then
    local ok, name = pcall(reader.objectName, value)
    if ok and name then return name end
  end
  if reader.toString then
    local ok, text = pcall(reader.toString, value)
    if ok then return protocol.truncate(tostring(text), M.LIMITS.maxString) end
  end
  return "<" .. kind .. ">"
end

local function encodeEnum(reader, property, value)
  local enum = reader.enum(property)
  if enum == nil then
    if reader.propertyType(property) == "ByteProperty" then return finiteNumber(value) end
    error("枚举属性没有枚举信息")
  end
  local result = {
    enum = reader.fullName(enum),
    value = finiteNumber(value),
  }
  if reader.enumName then
    local ok, name = pcall(reader.enumName, enum, value)
    if ok and name ~= nil then result.name = tostring(name) end
  end
  return result
end

local function encodeObject(reader, value)
  if value == nil then return json.null end
  local name = reader.objectName(value)
  if name == nil then return json.null end
  return name
end

local encodeValue
local readProperty

local function inheritedFields(reader, struct)
  if struct == nil or not isValidObject(reader, struct) then return {} end
  local cacheKey
  if type(reader.address) == "function" then
    local okAddr, addr = pcall(reader.address, struct)
    if okAddr and addr ~= nil then
      cacheKey = tostring(addr)
      if reader._structFields and reader._structFields[cacheKey] then
        return reader._structFields[cacheKey]
      end
    end
  end
  local chain = {}
  local seenStructs = {}
  local current = struct
  local depth = 0
  while current and depth < 64 do
    if not isValidObject(reader, current) then break end
    if seenStructs[current] then break end
    seenStructs[current] = true
    chain[#chain + 1] = current
    if type(reader.super) ~= "function" then break end
    local okSuper, nextStruct = pcall(reader.super, current)
    if not okSuper then break end
    current = nextStruct
    depth = depth + 1
  end
  local fields = {}
  local seen = {}
  for index = #chain, 1, -1 do
    local okFields, structFields = pcall(reader.fields, chain[index])
    if okFields and type(structFields) == "table" then
      for _, field in ipairs(structFields) do
        local okName, name = pcall(reader.propertyName, field)
        if okName and name and not seen[name] then
          seen[name] = true
          fields[#fields + 1] = field
        end
      end
    end
  end
  if cacheKey then
    reader._structFields = reader._structFields or {}
    reader._structFields[cacheKey] = fields
  end
  return fields
end

local function encodeStruct(reader, property, value, depth)
  if depth > M.LIMITS.maxDepth then return "<depth>" end
  if value == nil then return json.null end
  local struct = reader.struct(property)
  if struct == nil or not isValidObject(reader, struct) then error("结构定义不可用") end
  local fields = inheritedFields(reader, struct)
  local result = {}
  for _, field in ipairs(fields) do
    local okName, name = pcall(reader.propertyName, field)
    if okName and name then
      result[name] = readProperty(reader, value, field, depth + 1)
    end
  end
  return result
end

local function encodeArray(reader, property, value, depth)
  if depth > M.LIMITS.maxDepth then return "<depth>" end
  if value == nil then return { len = 0, items = json.array({}) } end
  local length = reader.arrayNum(value)
  if type(length) ~= "number" or length < 0 or length ~= length or length == math.huge then
    error("数组长度不合法")
  end
  local inner = reader.inner(property)
  if inner == nil then error("数组元素属性不可用") end
  local okInnerKind, innerKind = pcall(reader.propertyType, inner)
  local items = json.array({})
  local count = math.min(math.floor(length), M.LIMITS.maxItems)
  if okInnerKind and not isReadableKind(innerKind) then
    local placeholder = "<" .. tostring(innerKind) .. ">"
    for index = 1, count do items[index] = placeholder end
    return { len = length, items = items }
  end
  for index = 1, count do
    local ok, item = pcall(reader.arrayItem, value, index)
    if ok then
      items[index] = encodeValue(reader, inner, item, depth + 1)
    else
      items[index] = errorValue(item)
    end
  end
  return { len = length, items = items }
end

local function encodeMap(reader, property, value, depth, isSet)
  if depth > M.LIMITS.maxDepth then return "<depth>" end
  if value == nil then return isSet and "<Set>" or "<Map>" end
  if not reader.mapEntries then return "<Map>" end
  local ok, length, entries = pcall(reader.mapEntries, value, M.LIMITS.maxItems, isSet)
  if not ok or type(length) ~= "number" or type(entries) ~= "table" then return "<Map>" end
  local items = json.array({})
  local keyProperty = reader.mapKey and reader.mapKey(property) or nil
  local valueProperty = reader.mapValue and reader.mapValue(property) or nil
  local nextDepth = depth + 1
  for index, entry in ipairs(entries) do
    local key = keyProperty and encodeValue(reader, keyProperty, entry[1], nextDepth)
      or M.auto(reader, entry[1], nextDepth)
    local itemValue
    if isSet then
      itemValue = json.null
    elseif valueProperty then
      itemValue = encodeValue(reader, valueProperty, entry[2], nextDepth)
    else
      itemValue = M.auto(reader, entry[2], nextDepth)
    end
    items[index] = json.array({ key, itemValue })
  end
  return { len = length, items = items }
end

local function encodeUnsafe(reader, property, value, depth)
  local kind = reader.propertyType(property)
  if kind == "EnumProperty" then return encodeEnum(reader, property, value) end
  if kind == "ByteProperty" and reader.hasEnum and reader.hasEnum(property) then
    return encodeEnum(reader, property, value)
  end
  if numeric[kind] then return finiteNumber(value) end
  if kind == "BoolProperty" then
    if type(value) ~= "boolean" then error("布尔属性值不是 boolean") end
    return value
  end
  if strings[kind] then
    return protocol.truncate(stringify(reader, value), M.LIMITS.maxString)
  end
  if objectReferences[kind] then return encodeObject(reader, value) end
  if kind == "StructProperty" then return encodeStruct(reader, property, value, depth) end
  if kind == "ArrayProperty" then return encodeArray(reader, property, value, depth) end
  if kind == "MapProperty" then return encodeMap(reader, property, value, depth, false) end
  if kind == "SetProperty" then return encodeMap(reader, property, value, depth, true) end
  return "<" .. tostring(kind) .. ">"
end

function encodeValue(reader, property, value, depth)
  local ok, encoded = pcall(encodeUnsafe, reader, property, value, depth or 1)
  if not ok then return errorValue(encoded) end
  return encoded
end

function readProperty(reader, owner, property, depth)
  local okKind, kind = pcall(reader.propertyType, property)
  if not okKind then return errorValue(kind) end
  if not isReadableKind(kind) then return "<" .. tostring(kind) .. ">" end
  if depth > M.LIMITS.maxDepth and containers[kind] then return "<depth>" end
  local ok, value = pcall(reader.read, owner, property)
  if not ok then return errorValue(value) end
  return encodeValue(reader, property, value, depth)
end

function M.encode(reader, property, value)
  return encodeValue(reader, property, value, 1)
end

function M.read(reader, owner, property)
  return readProperty(reader, owner, property, 1)
end

return M
