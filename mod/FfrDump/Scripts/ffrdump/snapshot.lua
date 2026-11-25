local json = require("ffrdump.json")
local values = require("ffrdump.values")

local M = {}

M.LIMITS = {
  perClass = 50,
  objects = 20000,
  widgets = 2000,
  texts = 2000,
  maxScanned = 100000,
  maxSeconds = 30,
  batchSize = 200,
  batchSeconds = 0.05,
}

-- EObjectFlags for CDOs, archetypes/templates, uninitialized or destroying UObjects.
-- Deliberately omits RF_DefaultSubObject so live Actor components are still captured.
local SKIP_OBJECT_FLAGS = 0x00038630

local function resolveLimits(override)
  local limits = {}
  for key, value in pairs(M.LIMITS) do limits[key] = value end
  if type(override) == "table" then
    for key, value in pairs(override) do
      if value ~= nil then limits[key] = value end
    end
  end
  return limits
end

local function pathOf(fullName)
  if type(fullName) ~= "string" then return "" end
  return fullName:match("^%S+%s+(.+)$") or fullName
end

local function temporaryPath(path)
  local base = path:match("^(.*)%.[^%.\\/]*$")
  return (base or path) .. ".tmp"
end

local function join(directory, name)
  local separator = directory:find("\\", 1, true) and not directory:find("/", 1, true)
    and "\\" or "/"
  return directory .. separator .. name
end

local function openWriter(directory, name)
  local path = join(directory, name)
  local file, err = io.open(temporaryPath(path), "wb")
  if not file then error("无法写快照临时文件：" .. tostring(err)) end
  local closed = false
  local writer = { path = path, file = file, first = true }
  function writer.write(text)
    local ok, writeError = file:write(text)
    if not ok then error("写快照失败：" .. tostring(writeError)) end
  end
  function writer.line(value)
    writer.write(json.encode(value) .. "\n")
  end
  function writer.close()
    if closed then return end
    closed = true
    local ok, closeError = file:close()
    if not ok then error("关闭快照失败：" .. tostring(closeError)) end
  end
  function writer.abort()
    if closed then return end
    closed = true
    pcall(function() file:close() end)
    pcall(os.remove, temporaryPath(path))
  end
  function writer.finish()
    writer.close()
    os.remove(path)
    local ok, renameError = os.rename(temporaryPath(path), path)
    if not ok then error("改名快照失败：" .. tostring(renameError)) end
  end
  return writer
end

local function safe(reader, method, ...)
  local fn = reader[method]
  if not fn then return nil end
  local ok, value = pcall(fn, ...)
  if ok then return value end
  return nil
end

local function isValid(reader, object)
  if object == nil then return false end
  if type(reader.isValid) ~= "function" then return true end
  return safe(reader, "isValid", object) == true
end

local function fullName(reader, object)
  if object == nil then return nil end
  local name = safe(reader, "fullName", object)
  if type(name) == "string" and name ~= "" then return name end
  return nil
end

local function classOf(reader, object)
  if object == nil then return nil end
  local cls = safe(reader, "class", object)
  if not isValid(reader, cls) then return nil end
  return cls
end

local function className(reader, object)
  return fullName(reader, classOf(reader, object)) or "<unknown>"
end

local function isCdo(reader, object)
  local name = safe(reader, "shortName", object)
  if type(name) ~= "string" or name == "" then
    name = pathOf(fullName(reader, object)):match("[^.]+$")
  end
  return type(name) == "string" and name:sub(1, 9) == "Default__"
end

local function isTemplateOrDestroyed(reader, object)
  return safe(reader, "isTemplateOrDestroyed", object) == true
end

local function packageName(reader, class)
  local outer = safe(reader, "outer", class)
  local last
  local seen = {}
  local depth = 0
  while outer and depth < 64 do
    if not isValid(reader, outer) then break end
    if seen[outer] then break end
    seen[outer] = true
    local name = fullName(reader, outer)
    if name then last = pathOf(name) end
    outer = safe(reader, "outer", outer)
    depth = depth + 1
  end
  return last or ""
end

local function inspectClassNode(reader, class, superMeta, knownKey, knownName)
  local key = knownKey
  if not key then
    local address = safe(reader, "address", class)
    key = address ~= nil and tostring(address) or nil
  end
  if key and superMeta[key] then return superMeta[key] end
  local name = knownName or fullName(reader, class)
  if not name then return nil end
  key = key or name
  if superMeta[key] then return superMeta[key] end
  local pkg = packageName(reader, class)
  local game = pkg == "/Script/mucha" or pathOf(name):sub(1, 6) == "/Game/"
  local fields = {}
  if game then
    local rawFields = safe(reader, "fields", class)
    if type(rawFields) == "table" then
      for _, property in ipairs(rawFields) do
        local propertyName = safe(reader, "propertyName", property)
        if type(propertyName) == "string" and propertyName ~= "" then
          fields[#fields + 1] = { name = propertyName, property = property }
        end
      end
    end
  end
  local superClass = safe(reader, "super", class)
  if not isValid(reader, superClass) then superClass = nil end
  local node = {
    name = name,
    packageName = pkg,
    game = game,
    isWidget = name == "Class /Script/UMG.UserWidget",
    isText = name == "Class /Script/UMG.TextBlock" or name == "Class /Script/UMG.RichTextBlock",
    fields = fields,
    super = superClass,
  }
  superMeta[key] = node
  return node
end

local function makeClassInfo(reader, class, cache, order, superMeta)
  local address = safe(reader, "address", class)
  local key = address ~= nil and tostring(address) or nil
  if key and cache[key] then return cache[key] end
  local name = fullName(reader, class)
  if not name then return nil end
  key = key or name
  if cache[key] then return cache[key] end
  local info = {
    className = name,
    packageName = "",
    superChain = json.array({}),
    fields = {},
    game = false,
    isWidget = false,
    isText = false,
    alive = 0,
    queued = 0,
    dumped = 0,
  }
  local chain = {}
  local seenChain = {}
  local current = class
  local depth = 0
  while current and depth < 64 do
    if not isValid(reader, current) then break end
    if seenChain[current] then break end
    seenChain[current] = true
    local node = inspectClassNode(
      reader,
      current,
      superMeta,
      current == class and key or nil,
      current == class and name or nil
    )
    if not node then break end
    if depth == 0 then info.packageName = node.packageName end
    info.superChain[#info.superChain + 1] = node.name
    info.game = info.game or node.game
    info.isWidget = info.isWidget or node.isWidget
    info.isText = info.isText or node.isText
    chain[#chain + 1] = node
    current = node.super
    depth = depth + 1
  end
  local seen = {}
  for index = #chain, 1, -1 do
    local entry = chain[index]
    if entry.game then
      for _, field in ipairs(entry.fields) do
        if not seen[field.name] then
          seen[field.name] = true
          info.fields[#info.fields + 1] = field
        end
      end
    end
  end
  cache[key] = info
  order[#order + 1] = info
  return info
end

local function member(reader, object, name)
  local ok, value = pcall(reader.member, object, name)
  if not ok then return values.errorValue(value) end
  return values.auto(reader, value)
end

local function objectDescriptor(reader, object)
  if not isValid(reader, object) then return json.null end
  return { name = fullName(reader, object) or "<unknown>", class = className(reader, object) }
end

local function vector(reader, value)
  if value == nil then return json.null end
  return { x = member(reader, value, "X"), y = member(reader, value, "Y"), z = member(reader, value, "Z") }
end

local function rotator(reader, value)
  if value == nil then return json.null end
  return {
    pitch = member(reader, value, "Pitch"),
    yaw = member(reader, value, "Yaw"),
    roll = member(reader, value, "Roll"),
  }
end

local function writeContext(options)
  local reader = options.reader
  local filePrefix = type(options.filePrefix) == "string" and options.filePrefix or ""
  local function find(name)
    local ok, object = pcall(options.findFirstOf, name)
    if ok and isValid(reader, object) then return object end
    return nil
  end
  local world = find("World")
  local controller = find("PlayerController")
  local pawn = controller and safe(reader, "member", controller, "Pawn") or nil
  if not isValid(reader, pawn) then pawn = nil end
  local location = pawn and safe(reader, "location", pawn) or nil
  local rotation = pawn and safe(reader, "rotation", pawn) or nil
  local context = {
    world = fullName(reader, world) or "<unknown>",
    playerController = objectDescriptor(reader, controller),
    pawn = objectDescriptor(reader, pawn),
    pawnLocation = vector(reader, location),
    pawnRotation = rotator(reader, rotation),
    gameInstance = className(reader, find("GameInstance")),
    gameMode = className(reader, find("GameModeBase")),
    gameState = className(reader, find("GameStateBase")),
    time = options.time(),
  }
  local writer = openWriter(options.directory, filePrefix .. "context.json")
  local ok, err = pcall(function()
    writer.write(json.encode(context) .. "\n")
    writer.finish()
  end)
  if not ok then
    writer.abort()
    error(err)
  end
end

local function makeReader()
  local reader = {}
  local function validWrapper(object)
    if object == nil then return false end
    local ok, valid = pcall(function() return object:IsValid() end)
    return ok and valid == true
  end
  function reader.isValid(object) return validWrapper(object) end
  function reader.isTemplateOrDestroyed(object)
    if not validWrapper(object) then return true end
    local ok, flagged = pcall(function()
      if type(object.HasAnyFlags) ~= "function" then return false end
      return object:HasAnyFlags(SKIP_OBJECT_FLAGS) == true
    end)
    return ok and flagged == true
  end
  function reader.fullName(object)
    if not validWrapper(object) then return nil end
    return object:GetFullName()
  end
  function reader.shortName(object)
    if not validWrapper(object) then return nil end
    return object:GetFName():ToString()
  end
  function reader.class(object)
    if not validWrapper(object) then return nil end
    local cls = object:GetClass()
    if not validWrapper(cls) then return nil end
    return cls
  end
  function reader.super(class)
    if not validWrapper(class) then return nil end
    local sup = class:GetSuperStruct()
    if not validWrapper(sup) then return nil end
    return sup
  end
  function reader.outer(object)
    if not validWrapper(object) then return nil end
    local out = object:GetOuter()
    if not validWrapper(out) then return nil end
    return out
  end
  function reader.address(object)
    if not validWrapper(object) then return nil end
    return object:GetAddress()
  end
  function reader.member(object, name)
    if object == nil then return nil end
    return object[name]
  end
  function reader.read(object, property)
    if object == nil then return nil end
    return object[reader.propertyName(property)]
  end
  function reader.propertyName(property) return property:GetFName():ToString() end
  function reader.propertyType(property) return property:GetClass():GetFName():ToString() end
  function reader.fields(struct)
    local fields = {}
    if not validWrapper(struct) then return fields end
    struct:ForEachProperty(function(property) fields[#fields + 1] = property end)
    return fields
  end
  function reader.objectName(object)
    if not validWrapper(object) then return nil end
    return object:GetFullName()
  end
  function reader.toString(value)
    if type(value) == "string" then return value end
    return value:ToString()
  end
  function reader.enum(property)
    local enum
    if type(GetEnum) == "function" then
      enum = GetEnum(property)
    else
      enum = property:GetEnum()
    end
    if not validWrapper(enum) then return nil end
    return enum
  end
  function reader.hasEnum(property)
    local ok, enum = pcall(reader.enum, property)
    return ok and enum ~= nil
  end
  function reader.enumName(enum, value) return enum:GetNameByValue(value):ToString() end
  function reader.struct(property)
    local struct
    if type(GetStruct) == "function" then
      struct = GetStruct(property)
    else
      struct = property:GetStruct()
    end
    if not validWrapper(struct) then return nil end
    return struct
  end
  function reader.inner(property)
    local inner = property:GetInner()
    if not validWrapper(inner) then return nil end
    return inner
  end
  function reader.arrayNum(array) return array:GetArrayNum() end
  function reader.arrayItem(array, index) return array[index] end
  local function unwrap(value)
    if value ~= nil and type(value.get) == "function" then return value:get() end
    return value
  end
  function reader.mapEntries(map, limit, isSet)
    if map == nil or type(map.ForEach) ~= "function" then error("不支持遍历 Map") end
    local length = #map
    local entries = {}
    map:ForEach(function(key, value)
      if #entries >= limit then return true end
      if isSet then
        entries[#entries + 1] = { unwrap(key), json.null }
      else
        entries[#entries + 1] = { unwrap(key), unwrap(value) }
      end
    end)
    return length, entries
  end
  function reader.location(pawn)
    if not validWrapper(pawn) then return nil end
    return pawn:K2_GetActorLocation()
  end
  function reader.rotation(pawn)
    if not validWrapper(pawn) then return nil end
    return pawn:K2_GetActorRotation()
  end
  function reader.inViewport(widget)
    if not validWrapper(widget) then return nil end
    return widget:IsInViewport()
  end
  return reader
end

M.makeReader = makeReader

function M.create(options)
  local limits = resolveLimits(options.limits)
  local clock = options.clock or os.clock
  local time = options.time or os.time
  local started = clock()
  local startedTime = time()
  local reader = options.reader or makeReader()
  local iterator = options.forEachUObject or ForEachUObject
  local findFirstOf = options.findFirstOf or FindFirstOf
  local filePrefix = type(options.filePrefix) == "string" and options.filePrefix or ""
  local function fileName(base)
    return filePrefix .. base
  end
  local settings = {
    reader = reader,
    directory = options.directory,
    filePrefix = filePrefix,
    findFirstOf = findFirstOf,
    time = time,
  }
  local batched = options.batched == true
  local batchSize = math.max(1, math.floor(limits.batchSize or M.LIMITS.batchSize))
  local batchSeconds = batched and limits.batchSeconds or nil
  local maxScanned = limits.maxScanned
  local maxSeconds = limits.maxSeconds

  local objectsWriter
  local widgetsWriter
  local textsWriter
  local classesWriter
  local cache = {}
  local superMeta = {}
  local order = {}
  local pending = {}
  local pendingCount = 0
  local cursor = 1
  local queuedObjects = 0
  local queuedWidgets = 0
  local queuedTexts = 0
  local stats = {
    objectsScanned = 0,
    objectsDumped = 0,
    truncated = false,
    objectErrors = 0,
  }
  local state = "scan"
  local finalResult = nil

  local function isTimeExceeded()
    return maxSeconds ~= nil and maxSeconds > 0 and (time() - startedTime) >= maxSeconds
  end

  local function abortWriters()
    if objectsWriter then objectsWriter.abort(); objectsWriter = nil end
    if widgetsWriter then widgetsWriter.abort(); widgetsWriter = nil end
    if textsWriter then textsWriter.abort(); textsWriter = nil end
    if classesWriter then classesWriter.abort(); classesWriter = nil end
  end

  local function scanObject(object)
    if not isValid(reader, object) or isTemplateOrDestroyed(reader, object) or isCdo(reader, object) then
      return
    end
    local class = classOf(reader, object)
    if not class then return end
    local info = makeClassInfo(reader, class, cache, order, superMeta)
    if not info then return end
    info.alive = info.alive + 1
    if not info.game and not info.isWidget and not info.isText then return end

    local wantGame = false
    local wantWidget = false
    local wantText = false

    if info.game then
      if info.queued >= limits.perClass or queuedObjects >= limits.objects then
        stats.truncated = true
      else
        wantGame = true
        info.queued = info.queued + 1
        queuedObjects = queuedObjects + 1
      end
    end

    if info.isWidget then
      if queuedWidgets >= limits.widgets then
        stats.truncated = true
      else
        wantWidget = true
        queuedWidgets = queuedWidgets + 1
      end
    end

    if info.isText then
      if queuedTexts >= limits.texts then
        stats.truncated = true
      else
        wantText = true
        queuedTexts = queuedTexts + 1
      end
    end

    if wantGame or wantWidget or wantText then
      pending[#pending + 1] = {
        object = object,
        info = info,
        game = wantGame,
        widget = wantWidget,
        text = wantText,
      }
    end
  end

  local function dumpEntry(entry)
    local object = entry.object
    local info = entry.info
    -- Re-verify validity on each batch tick in case GC or map travel invalidated the object.
    if not isValid(reader, object) or isTemplateOrDestroyed(reader, object) then return end
    local objectFullName = fullName(reader, object)
    if objectFullName and objectFullName:find("Default__", 1, true) then return end
    local displayName = objectFullName or "<unknown>"

    if entry.game then
      local properties = {}
      for _, field in ipairs(info.fields) do
        properties[field.name] = values.read(reader, object, field.property)
      end
      objectsWriter.line({
        name = displayName,
        class = info.className,
        properties = properties,
      })
      info.dumped = info.dumped + 1
      stats.objectsDumped = stats.objectsDumped + 1
    end

    if entry.widget then
      local visible = safe(reader, "inViewport", object)
      if type(visible) ~= "boolean" then visible = "<error: IsInViewport>" end
      widgetsWriter.line({
        name = displayName,
        class = info.className,
        isInViewport = visible,
        visibility = member(reader, object, "Visibility"),
      })
    end

    if entry.text then
      textsWriter.line({
        name = displayName,
        class = info.className,
        text = member(reader, object, "Text"),
        visibility = member(reader, object, "Visibility"),
      })
    end
  end

  local function finishSnapshot()
    objectsWriter.finish(); objectsWriter = nil
    widgetsWriter.finish(); widgetsWriter = nil
    textsWriter.finish(); textsWriter = nil
    classesWriter = openWriter(options.directory, fileName("classes.json"))
    classesWriter.write("[")
    for index, info in ipairs(order) do
      if index > 1 then classesWriter.write(",") end
      classesWriter.write(json.encode({
        class = info.className,
        package = info.packageName,
        superChain = info.superChain,
        instances = info.alive,
        dumped = info.dumped,
      }))
    end
    classesWriter.write("]\n")
    classesWriter.finish(); classesWriter = nil
    stats.classesSeen = #order
    stats.seconds = math.floor((clock() - started) * 10 + 0.5) / 10
    state = "done"
    finalResult = {
      files = json.array({
        fileName("context.json"),
        fileName("classes.json"),
        fileName("objects.jsonl"),
        fileName("widgets.jsonl"),
        fileName("texts.jsonl"),
      }),
      stats = stats,
    }
    return true, finalResult
  end

  local stepper = {}

  function stepper.abort()
    abortWriters()
    state = "done"
  end

  function stepper.step()
    if state == "done" then return true, finalResult end

    if state == "scan" then
      writeContext(settings)
      objectsWriter = openWriter(options.directory, fileName("objects.jsonl"))
      widgetsWriter = openWriter(options.directory, fileName("widgets.jsonl"))
      textsWriter = openWriter(options.directory, fileName("texts.jsonl"))
      local stopped = false
      iterator(function(object)
        if stopped then return true end
        if maxScanned ~= nil and maxScanned > 0 and stats.objectsScanned >= maxScanned then
          stats.truncated = true
          stopped = true
          return true
        end
        stats.objectsScanned = stats.objectsScanned + 1
        if stats.objectsScanned % 256 == 0 and isTimeExceeded() then
          stats.truncated = true
          stopped = true
          return true
        end
        local ok = pcall(scanObject, object)
        if not ok then stats.objectErrors = stats.objectErrors + 1 end
      end)
      pendingCount = #pending
      if pendingCount > 0 and isTimeExceeded() then
        stats.truncated = true
        state = "finish"
      elseif pendingCount > 0 then
        state = "dump"
        return false, nil
      else
        state = "finish"
      end
    end

    if state == "dump" then
      local batchStarted = batchSeconds and clock() or nil
      local processed = 0
      while cursor <= pendingCount do
        if isTimeExceeded() then
          stats.truncated = true
          state = "finish"
          break
        end
        if processed >= batchSize then return false, nil end
        if batchStarted and processed > 0 and (clock() - batchStarted) >= batchSeconds then
          return false, nil
        end
        local entry = pending[cursor]
        pending[cursor] = nil
        cursor = cursor + 1
        processed = processed + 1
        local ok = pcall(dumpEntry, entry)
        if not ok then stats.objectErrors = stats.objectErrors + 1 end
      end
      if cursor > pendingCount then state = "finish" end
    end

    if state == "finish" then return finishSnapshot() end
    return false, nil
  end

  return stepper
end

function M.run(options)
  local stepper = M.create(options)
  local ok, result = pcall(function()
    while true do
      local done, output = stepper.step()
      if done then return output end
    end
  end)
  if not ok then
    stepper.abort()
    error(result)
  end
  return result
end

return M
