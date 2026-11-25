import assert from "node:assert/strict";
import { readFile, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { lua, lauxlib, lualib, to_jsstring, to_luastring } from "fengari";

const SCRIPTS = join(import.meta.dirname, "..", "..", "mod", "FfrDump", "Scripts");

function load(state: ReturnType<typeof lauxlib.luaL_newstate>, code: string): void {
  const status = lauxlib.luaL_loadbuffer(
    state,
    to_luastring(code),
    null,
    to_luastring("ffrdump-test"),
  );
  if (status !== lua.LUA_OK) {
    const message = lua.lua_tostring(state, -1);
    throw new Error(message ? to_jsstring(message) : "Lua 编译失败");
  }
}

function runLua(body: string, encodeResult: boolean): string {
  const state = lauxlib.luaL_newstate();
  try {
    lualib.luaL_openlibs(state);
    const prelude = `package.path = ${JSON.stringify(join(SCRIPTS, "?.lua"))} .. ";" .. package.path\n`;
    const suffix = encodeResult
      ? '\nresult = require("ffrdump.json").encode((function()\n'
      : "\nresult = (function()\n";
    load(state, `${prelude}${suffix}${body}\nend)()${encodeResult ? ")" : ""}`);
    const status = lua.lua_pcall(state, 0, lua.LUA_MULTRET, 0);
    if (status !== lua.LUA_OK) {
      const message = lua.lua_tostring(state, -1);
      throw new Error(message ? to_jsstring(message) : "Lua 执行失败");
    }
    lua.lua_getglobal(state, to_luastring("result"));
    const result = lua.lua_tostring(state, -1);
    if (!result) throw new Error("Lua 没有返回字符串");
    return to_jsstring(result);
  } finally {
    lua.lua_close(state);
  }
}

// eslint-disable-next-line @typescript-eslint/no-unnecessary-type-parameters
function luaJson<T>(body: string): T {
  return JSON.parse(runLua(body, true)) as T;
}

function luaText(body: string): string {
  return runLua(body, false);
}

void test("json escapes strings, handles non-finite numbers, nesting and empty tables", () => {
  const text = luaText(`
    local json = require("ffrdump.json")
    return json.encode({
      quote = '"', slash = "\\\\", controls = "\\b\\f\\n\\r\\t\\1", chinese = "中文",
      nan = 0 / 0, infinity = math.huge, negativeInfinity = -math.huge,
      nested = json.array({ 1, true, { value = "x" } }),
      emptyArray = json.array({}), emptyObject = {},
    })
  `);
  assert.deepEqual(JSON.parse(text), {
    quote: '"',
    slash: "\\",
    controls: "\b\f\n\r\t\u0001",
    chinese: "中文",
    nan: "NaN",
    infinity: "Infinity",
    negativeInfinity: "-Infinity",
    nested: [1, true, { value: "x" }],
    emptyArray: [],
    emptyObject: {},
  });
  assert.match(text, /\\u0001/u);
  assert.equal(luaText('return require("ffrdump.json").encode({}, {emptyTable="array"})'), "[]");
  assert.equal(luaText('return require("ffrdump.json").encode({3,1,2})'), "[3,1,2]");
});

interface TriggerResult {
  trigger: { id: string; job: string; label: string } | null;
  error: string | null;
  id: string | null;
}

function parseTrigger(body: string): TriggerResult {
  return luaJson<TriggerResult>(`
    local protocol = require("ffrdump.protocol")
    local trigger, err, raw, id = protocol.parseTrigger(${JSON.stringify(body)})
    return { trigger = trigger or require("ffrdump.json").null, error = err or require("ffrdump.json").null,
      id = id or require("ffrdump.json").null }
  `);
}

const VALID_ID = "20261124-031500-field-1-snapshot";

void test("protocol accepts LF and CRLF trigger files", () => {
  for (const newline of ["\n", "\r\n"]) {
    const parsed = parseTrigger(
      `id=${VALID_ID}${newline}job=snapshot${newline}label=field-1${newline}`,
    );
    assert.deepEqual(parsed.trigger, { id: VALID_ID, job: "snapshot", label: "field-1" });
    assert.equal(parsed.error, null);
  }
});

void test("protocol rejects missing keys, traversal ids, unknown jobs and id suffix mismatches", () => {
  assert.match(parseTrigger("id=x\njob=snapshot\n").error ?? "", /缺少键/u);
  for (const id of [
    "20261124-031500-..-snapshot",
    "20261124-031500-field/1-snapshot",
    "20261124-031500-field\\1-snapshot",
  ]) {
    assert.match(parseTrigger(`id=${id}\njob=snapshot\nlabel=field-1\n`).error ?? "", /任务号/u);
  }
  assert.match(
    parseTrigger(`id=${VALID_ID}\njob=unknown\nlabel=field-1\n`).error ?? "",
    /未知任务/u,
  );
  assert.match(
    parseTrigger(`id=${VALID_ID}\njob=objects\nlabel=field-1\n`).error ?? "",
    /结尾不一致/u,
  );
});

void test("protocol truncates by UTF-8 characters without splitting a character", () => {
  const result = luaText(`
    return require("ffrdump.protocol").truncate(string.rep("汉", 513), 512)
  `);
  assert.equal(Array.from(result).length, 512);
  assert.equal(result, `${"汉".repeat(511)}…`);
  assert.equal(Buffer.from(result, "utf8").toString("utf8"), result);
  const error = luaText(`
    local values = require("ffrdump.values")
    return values.errorValue(string.rep("错", 300))
  `);
  assert.equal(Array.from(error).length, 200);
  assert.ok(error.endsWith("…"));
});

void test("protocol writes heartbeat and completion documents with array files and stats", () => {
  const heartbeat = JSON.parse(
    luaText('return require("ffrdump.protocol").heartbeat(1795000000, false)'),
  ) as unknown;
  assert.deepEqual(heartbeat, { mod: "FfrDump", version: 1, time: 1795000000, busy: false });
  const completion = JSON.parse(
    luaText(`
      return require("ffrdump.protocol").completion({
        id = "${VALID_ID}", job = "snapshot", label = "field-1", ok = false,
        error = "failed", startedAt = 10, finishedAt = 12,
      })
    `),
  ) as { files: unknown[]; stats: Record<string, unknown> };
  assert.deepEqual(completion.files, []);
  assert.deepEqual(completion.stats, { objectsScanned: 0, objectsDumped: 0, truncated: false });
});

const VALUE_READER = `
  local reader = {}
  function reader.propertyType(property) return property.kind end
  function reader.propertyName(property) return property.name end
  function reader.read(owner, property)
    if property.fail then error(property.fail) end
    return owner[property.name]
  end
  function reader.toString(value) return value.text end
  function reader.fullName(value) return value.fullName end
  function reader.objectName(value)
    if value.empty then return nil end
    return value.fullName
  end
  function reader.enum(property) return property.enum end
  function reader.hasEnum(property) return property.enum ~= nil end
  function reader.enumName(enum, value)
    if enum.noName then error("no name") end
    return enum.names[value]
  end
  function reader.struct(property) return property.struct end
  function reader.super(struct) return struct.super end
  function reader.fields(struct) return struct.fields end
  function reader.inner(property) return property.inner end
  function reader.arrayNum(array) return array.len end
  function reader.arrayItem(array, index)
    if index == 2 and array.failItem then error("bad item") end
    return array.values[index]
  end
  function reader.mapEntries(map, limit, isSet)
    if map.fail then error("unsupported") end
    local entries = {}
    for index, entry in ipairs(map.entries) do
      if index > limit then break end
      entries[index] = entry
    end
    return map.len, entries
  end
`;

void test("values encodes numbers, booleans, strings and truncation", () => {
  const result = luaJson<Record<string, unknown>>(`
    ${VALUE_READER}
    local values = require("ffrdump.values")
    local function numberProperty() return { kind = "IntProperty" } end
    return {
      integer = values.encode(reader, numberProperty(), 42),
      floating = values.encode(reader, { kind = "DoubleProperty" }, 1.5),
      nan = values.encode(reader, numberProperty(), 0 / 0),
      infinity = values.encode(reader, numberProperty(), math.huge),
      negativeInfinity = values.encode(reader, numberProperty(), -math.huge),
      bool = values.encode(reader, { kind = "BoolProperty" }, true),
      str = values.encode(reader, { kind = "StrProperty" }, "abc"),
      name = values.encode(reader, { kind = "NameProperty" }, { text = "名字" }),
      text = values.encode(reader, { kind = "TextProperty" }, { text = string.rep("汉", 513) }),
    }
  `);
  assert.equal(result.integer, 42);
  assert.equal(result.floating, 1.5);
  assert.deepEqual(
    [result.nan, result.infinity, result.negativeInfinity],
    ["NaN", "Infinity", "-Infinity"],
  );
  assert.equal(result.bool, true);
  assert.equal(result.str, "abc");
  assert.equal(result.name, "名字");
  assert.equal(Array.from(String(result.text)).length, 512);
  assert.ok(String(result.text).endsWith("…"));
});

void test("values encodes enum properties and byte enums, omitting unavailable names", () => {
  const result = luaJson<Record<string, { enum: string; value: number; name?: string }>>(`
    ${VALUE_READER}
    local values = require("ffrdump.values")
    local enum = { fullName = "Enum /Script/mucha.EThing", names = { [2] = "EThing::Two" } }
    local unnamed = { fullName = "Enum /Script/mucha.EOther", noName = true }
    return {
      enum = values.encode(reader, { kind = "EnumProperty", enum = enum }, 2),
      byte = values.encode(reader, { kind = "ByteProperty", enum = enum }, 2),
      plainByte = values.encode(reader, { kind = "ByteProperty" }, 7),
      unnamed = values.encode(reader, { kind = "EnumProperty", enum = unnamed }, 3),
    }
  `);
  assert.deepEqual(result.enum, {
    enum: "Enum /Script/mucha.EThing",
    value: 2,
    name: "EThing::Two",
  });
  assert.deepEqual(result.byte, result.enum);
  assert.deepEqual(result.plainByte, 7);
  assert.deepEqual(result.unnamed, { enum: "Enum /Script/mucha.EOther", value: 3 });
});

void test("reader treats an invalid enum wrapper as no enum", () => {
  const result = luaJson<{ enum: unknown; value: number }>(`
    local snapshot = require("ffrdump.snapshot")
    local values = require("ffrdump.values")
    local json = require("ffrdump.json")
    GetEnum = function()
      return { IsValid = function() return false end }
    end
    local reader = snapshot.makeReader()
    local property = {}
    function property:GetClass()
      return { GetFName = function()
        return { ToString = function() return "ByteProperty" end }
      end }
    end
    return { enum = reader.enum(property) or json.null, value = values.encode(reader, property, 7) }
  `);
  assert.equal(result.enum, null);
  assert.equal(result.value, 7);
});

void test("values encodes object references and nulls without expanding them", () => {
  const result = luaJson<Record<string, unknown>>(`
    ${VALUE_READER}
    local values = require("ffrdump.values")
    local object = { fullName = "MuchaThing /Game/World.Thing" }
    return {
      object = values.encode(reader, { kind = "ObjectProperty" }, object),
      weak = values.encode(reader, { kind = "WeakObjectProperty" }, object),
      soft = values.encode(reader, { kind = "SoftObjectProperty" }, object),
      class = values.encode(reader, { kind = "ClassProperty" }, object),
      null = values.encode(reader, { kind = "ObjectProperty" }, nil),
      empty = values.encode(reader, { kind = "ObjectProperty" }, { empty = true }),
    }
  `);
  for (const key of ["object", "weak", "soft", "class"]) {
    assert.equal(result[key], "MuchaThing /Game/World.Thing");
  }
  assert.equal(result.null, null);
  assert.equal(result.empty, null);
});

void test("values expands three struct levels and marks the fourth as depth", () => {
  const result = luaJson<{ a: { b: { c: string } } }>(`
    ${VALUE_READER}
    local values = require("ffrdump.values")
    local number = { kind = "IntProperty", name = "number" }
    local p4 = { kind = "StructProperty", name = "c", struct = { fields = { number } } }
    local p3 = { kind = "StructProperty", name = "b", struct = { fields = { p4 } } }
    local p2 = { kind = "StructProperty", name = "a", struct = { fields = { p3 } } }
    local p1 = { kind = "StructProperty", name = "root", struct = { fields = { p2 } } }
    return values.encode(reader, p1, { a = { b = { c = { number = 9 } } } })
  `);
  assert.deepEqual(result, { a: { b: { c: "<depth>" } } });
});

void test("values includes inherited struct fields", () => {
  const result = luaJson<Record<string, number>>(`
    ${VALUE_READER}
    local values = require("ffrdump.values")
    local parentField = { kind = "IntProperty", name = "Parent" }
    local childField = { kind = "IntProperty", name = "Child" }
    local parent = { fields = { parentField } }
    local child = { fields = { childField }, super = parent }
    local property = { kind = "StructProperty", struct = child }
    return values.encode(reader, property, { Parent = 10, Child = 20 })
  `);
  assert.deepEqual(result, { Parent: 10, Child: 20 });
});

void test("values limits arrays, maps and sets to 32 items and degrades unreadable maps", () => {
  const result = luaJson<Record<string, { len: number; items: unknown[] } | string>>(`
    ${VALUE_READER}
    local values = require("ffrdump.values")
    local numbers = {}
    local entries = {}
    for index = 1, 40 do
      numbers[index] = index
      entries[index] = { "k" .. index, index }
    end
    return {
      array = values.encode(reader,
        { kind = "ArrayProperty", inner = { kind = "IntProperty" } }, { len = 40, values = numbers }),
      map = values.encode(reader, { kind = "MapProperty" }, { len = 40, entries = entries }),
      set = values.encode(reader, { kind = "SetProperty" }, { len = 1, entries = { { "only" } } }),
      unreadable = values.encode(reader, { kind = "MapProperty" }, { fail = true }),
    }
  `);
  const array = result.array as { len: number; items: number[] };
  assert.equal(array.len, 40);
  assert.equal(array.items.length, 32);
  assert.equal(array.items[31], 32);
  const map = result.map as { len: number; items: [string, number][] };
  assert.equal(map.len, 40);
  assert.equal(map.items.length, 32);
  assert.deepEqual(map.items[0], ["k1", 1]);
  assert.deepEqual(result.set, { len: 1, items: [["only", null]] });
  assert.equal(result.unreadable, "<Map>");
});

void test("values marks delegates, interfaces, other types and read errors", () => {
  const result = luaJson<Record<string, string>>(`
    ${VALUE_READER}
    local values = require("ffrdump.values")
    return {
      delegate = values.encode(reader, { kind = "DelegateProperty" }, {}),
      interface = values.encode(reader, { kind = "InterfaceProperty" }, {}),
      other = values.encode(reader, { kind = "MulticastDelegateProperty" }, {}),
      failure = values.read(reader, {}, { kind = "IntProperty", name = "x", fail = string.rep("bad", 100) }),
    }
  `);
  assert.equal(result.delegate, "<DelegateProperty>");
  assert.equal(result.interface, "<InterfaceProperty>");
  assert.equal(result.other, "<MulticastDelegateProperty>");
  const failure = result.failure;
  assert.ok(failure);
  assert.match(failure, /^<error: /u);
  assert.equal(Array.from(failure).length, 200);
});

const SNAPSHOT_READER = `
  local snapshot = require("ffrdump.snapshot")
  local json = require("ffrdump.json")
  local stored = {}
  io.open = function(path, mode)
    if mode == "wb" then
      local handle = { parts = {} }
      function handle:write(text)
        self.parts[#self.parts + 1] = text
        return self
      end
      function handle:close()
        stored[path] = table.concat(self.parts)
        return true
      end
      return handle
    end
    local body = stored[path]
    if not body then return nil, "missing" end
    local handle = {}
    function handle:read() return body end
    function handle:close() return true end
    return handle
  end
  os.remove = function(path)
    stored[path] = nil
    return true
  end
  os.rename = function(from, to)
    if stored[from] == nil then return nil, "missing", 2 end
    stored[to] = stored[from]
    stored[from] = nil
    return true
  end
  local clock = 0
  os.clock = function()
    clock = clock + 0.26
    return clock
  end
  local reader = {}
  local superCalls = 0
  local classNameCalls = 0
  function reader.isValid(object) return object.valid ~= false end
  function reader.fullName(object)
    if object.isClass then classNameCalls = classNameCalls + 1 end
    return object.fullName
  end
  function reader.shortName(object) return object.shortName end
  function reader.class(object) return object.class end
  function reader.super(class)
    superCalls = superCalls + 1
    return class.super
  end
  function reader.outer(object) return object.outer end
  function reader.address(object) return object.address end
  function reader.member(object, name) return object.members[name] end
  function reader.read(object, property) return object.values[property.name] end
  function reader.propertyName(property) return property.name end
  function reader.propertyType(property) return property.kind end
  function reader.fields(class) return class.fields or {} end
  function reader.objectName(object) return object.fullName end
  function reader.toString(value) return value.text end
  function reader.inViewport(object) return object.inViewport end
  local nextAddress = 1
  function makeClass(name, package, super)
    local class = { fullName = name, address = nextAddress, super = super, fields = {}, isClass = true }
    nextAddress = nextAddress + 1
    class.outer = { fullName = "Package " .. package }
    return class
  end
  function makeObject(name, class)
    return { fullName = name, shortName = name:match("([^%s%.]+)$"), class = class,
      values = {}, members = {} }
  end
  function readOutput(name)
    local file = assert(io.open(directory .. "/" .. name, "rb"))
    local body = file:read("*a")
    file:close()
    return body
  end
`;

interface SnapshotResult {
  stats: {
    objectsScanned: number;
    objectsDumped: number;
    truncated: boolean;
    classesSeen: number;
    seconds: number;
  };
  classes: string;
  objects: string;
  widgets: string;
  texts: string;
  superCalls: number;
  classNameCalls: number;
}

function runSnapshot(
  directory: string,
  declarations: string,
  includeObjects = true,
): SnapshotResult {
  return luaJson<SnapshotResult>(`
    local directory = ${JSON.stringify(directory)}
    ${SNAPSHOT_READER}
    ${declarations}
    local result = snapshot.run({
      directory = directory, reader = reader,
      forEachUObject = function(callback) for _, object in ipairs(allObjects) do callback(object) end end,
      findFirstOf = function() return nil end,
      time = function() return 123 end,
    })
    return { stats = result.stats, classes = readOutput("classes.json"),
      objects = ${includeObjects ? 'readOutput("objects.jsonl")' : '""'},
      widgets = readOutput("widgets.jsonl"), texts = readOutput("texts.jsonl"),
      superCalls = superCalls, classNameCalls = classNameCalls }
  `);
}

void test("snapshot identifies game classes, skips CDOs and caches class analysis", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-snapshot-"));
  try {
    const result = runSnapshot(
      directory,
      `
      local native = makeClass("Class /Script/mucha.CPP_Thing", "/Script/mucha")
      local blueprint = makeClass("BlueprintGeneratedClass /Game/UI.BP_Thing_C", "/Game/UI")
      local engine = makeClass("Class /Script/Engine.Actor", "/Script/Engine")
      local property = { name = "Health", kind = "IntProperty" }
      native.fields = { property }
      local one = makeObject("MuchaThing /Game/World.One", native)
      one.values.Health = 10
      local two = makeObject("MuchaThing /Game/World.Two", native)
      two.values.Health = 20
      local cdo = makeObject("MuchaThing Default__CPP_Thing", native)
      local bp = makeObject("BP_Thing_C /Game/World.BP", blueprint)
      local actor = makeObject("Actor /Game/World.Actor", engine)
      allObjects = { one, two, cdo, bp, actor }
    `,
    );
    assert.equal(result.stats.objectsScanned, 5);
    assert.equal(result.stats.objectsDumped, 3);
    assert.equal(result.stats.classesSeen, 3);
    assert.equal(result.stats.seconds, 0.3);
    assert.equal(result.superCalls, 3);
    assert.equal(result.classNameCalls, 3);
    const classes = JSON.parse(result.classes) as {
      class: string;
      instances: number;
      dumped: number;
    }[];
    const native = classes.find((entry) => entry.class.includes("CPP_Thing"));
    assert.ok(native);
    assert.equal(native.instances, 2);
    assert.equal(native.dumped, 2);
    const lines = result.objects
      .trim()
      .split("\n")
      .map((line) => JSON.parse(line) as { name: string });
    assert.equal(lines.length, 3);
    assert.ok(lines.every((line) => !line.name.includes("Default__")));
    assert.ok(lines.every((line) => !line.name.includes("World.Actor")));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("snapshot collects game fields from blueprint and mucha parents only", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-snapshot-"));
  try {
    const result = runSnapshot(
      directory,
      `
      local engine = makeClass("Class /Script/Engine.Actor", "/Script/Engine")
      engine.fields = { { name = "EngineOnly", kind = "IntProperty" } }
      local native = makeClass("Class /Script/mucha.CPP_Base", "/Script/mucha", engine)
      native.fields = { { name = "NativeField", kind = "IntProperty" } }
      local blueprint = makeClass("BlueprintGeneratedClass /Game/UI.BP_Child_C", "/Game/UI", native)
      blueprint.fields = { { name = "BlueprintField", kind = "IntProperty" } }
      local object = makeObject("BP_Child_C /Game/World.Child", blueprint)
      object.values.EngineOnly = 1
      object.values.NativeField = 20
      object.values.BlueprintField = 10
      allObjects = { object }
    `,
    );
    const row = JSON.parse(result.objects.trim()) as { properties: Record<string, number> };
    assert.deepEqual(row.properties, { NativeField: 20, BlueprintField: 10 });
    assert.equal(result.stats.classesSeen, 1);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("snapshot identifies widgets and texts from the cached class chain", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-snapshot-"));
  try {
    const result = runSnapshot(
      directory,
      `
      local widgetBase = makeClass("Class /Script/UMG.UserWidget", "/Script/UMG")
      local widgetClass = makeClass("WidgetBlueprintGeneratedClass /Game/UI.W_C", "/Game/UI", widgetBase)
      local textClass = makeClass("Class /Script/UMG.RichTextBlock", "/Script/UMG")
      local widget = makeObject("W_C /Game/World.W", widgetClass)
      widget.members.Visibility = { text = "Visible" }
      widget.inViewport = true
      local text = makeObject("RichTextBlock /Game/World.T", textClass)
      text.members.Text = { text = "hello" }
      text.members.Visibility = { text = "Visible" }
      allObjects = { widget, text }
    `,
    );
    const widget = JSON.parse(result.widgets.trim()) as {
      isInViewport: boolean;
      visibility: string;
    };
    const text = JSON.parse(result.texts.trim()) as { text: string; visibility: string };
    assert.equal(widget.isInViewport, true);
    assert.equal(widget.visibility, "Visible");
    assert.equal(text.text, "hello");
    assert.equal(text.visibility, "Visible");
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("snapshot enforces the per-class limit of 50", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-snapshot-"));
  try {
    const result = runSnapshot(
      directory,
      `
      local class = makeClass("Class /Script/mucha.CPP_Thing", "/Script/mucha")
      allObjects = {}
      for index = 1, 55 do allObjects[index] = makeObject("Thing " .. index, class) end
    `,
    );
    assert.equal(result.stats.objectsDumped, 50);
    assert.equal(result.stats.truncated, true);
    assert.equal(result.objects.trim().split("\n").length, 50);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("snapshot enforces the total object limit of 20000", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-snapshot-"));
  try {
    const result = runSnapshot(
      directory,
      `
      allObjects = {}
      local index = 1
      for classIndex = 1, 401 do
        local class = makeClass("Class /Script/mucha.CPP_Thing" .. classIndex, "/Script/mucha")
        for itemIndex = 1, 50 do
          allObjects[index] = makeObject("Thing " .. index, class)
          index = index + 1
        end
      end
    `,
      false,
    );
    assert.equal(result.stats.objectsScanned, 20050);
    assert.equal(result.stats.objectsDumped, 20000);
    assert.equal(result.stats.truncated, true);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("values skips reading unsupported property types and caps depth inside arrays and maps", () => {
  const result = luaJson<{
    reads: number;
    delegate: string;
    optional: string;
    nestedArray: { items: { items: { items: string[] }[] }[] };
  }>(`
    ${VALUE_READER}
    local reads = 0
    local baseRead = reader.read
    function reader.read(owner, property)
      reads = reads + 1
      return baseRead(owner, property)
    end
    local values = require("ffrdump.values")
    local delegate = values.read(reader, {}, { kind = "MulticastInlineDelegateProperty", name = "OnClick", fail = "must not read" })
    local optional = values.read(reader, {}, { kind = "OptionalProperty", name = "Opt", fail = "must not read" })
    local leaf = { kind = "ArrayProperty", inner = { kind = "IntProperty" } }
    local mid = { kind = "ArrayProperty", inner = leaf }
    local outer = { kind = "ArrayProperty", inner = mid }
    local root = { kind = "ArrayProperty", inner = outer }
    local nestedValue = {
      len = 1,
      values = { { len = 1, values = { { len = 1, values = { { len = 1, values = { 42 } } } } } } },
    }
    return {
      reads = reads,
      delegate = delegate,
      optional = optional,
      nestedArray = values.encode(reader, root, nestedValue),
    }
  `);
  assert.equal(result.reads, 0);
  assert.equal(result.delegate, "<MulticastInlineDelegateProperty>");
  assert.equal(result.optional, "<OptionalProperty>");
  assert.equal(result.nestedArray.items[0]?.items[0]?.items[0], "<depth>");
});

void test("snapshot truncates when object scan limit or time limit is reached", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-snapshot-"));
  try {
    const byScan = luaJson<SnapshotResult>(`
      local directory = ${JSON.stringify(directory)}
      ${SNAPSHOT_READER}
      local class = makeClass("Class /Script/mucha.CPP_Thing", "/Script/mucha")
      local allObjects = {}
      for index = 1, 50 do allObjects[index] = makeObject("Thing " .. index, class) end
      local result = snapshot.run({
        directory = directory, reader = reader,
        limits = { maxScanned = 12, batchSize = 5 },
        forEachUObject = function(callback)
          for _, object in ipairs(allObjects) do
            if callback(object) == true then break end
          end
        end,
        findFirstOf = function() return nil end,
        time = function() return 100 end,
      })
      return { stats = result.stats, classes = readOutput("classes.json"),
        objects = readOutput("objects.jsonl"), widgets = readOutput("widgets.jsonl"),
        texts = readOutput("texts.jsonl"), superCalls = superCalls, classNameCalls = classNameCalls }
    `);
    assert.equal(byScan.stats.objectsScanned, 12);
    assert.equal(byScan.stats.objectsDumped, 12);
    assert.equal(byScan.stats.truncated, true);

    const byTime = luaJson<SnapshotResult>(`
      local directory = ${JSON.stringify(directory)}
      ${SNAPSHOT_READER}
      local class = makeClass("Class /Script/mucha.CPP_Thing", "/Script/mucha")
      local allObjects = {}
      for index = 1, 20 do allObjects[index] = makeObject("Thing " .. index, class) end
      local now = 100
      local timeCalls = 0
      local result = snapshot.run({
        directory = directory, reader = reader,
        limits = { maxSeconds = 5, batchSize = 4 },
        forEachUObject = function(callback)
          for _, object in ipairs(allObjects) do callback(object) end
        end,
        findFirstOf = function() return nil end,
        time = function()
          timeCalls = timeCalls + 1
          -- Advance past maxSeconds once dumping has processed a few objects.
          if timeCalls >= 6 then now = 110 end
          return now
        end,
      })
      return { stats = result.stats, classes = readOutput("classes.json"),
        objects = readOutput("objects.jsonl"), widgets = readOutput("widgets.jsonl"),
        texts = readOutput("texts.jsonl"), superCalls = superCalls, classNameCalls = classNameCalls }
    `);
    assert.equal(byTime.stats.truncated, true);
    assert.ok(byTime.stats.objectsDumped < 20);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("snapshot isolates single-object errors, invalidation across batches and CDO subobjects", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-snapshot-"));
  try {
    const result = luaJson<{
      stats: {
        objectsScanned: number;
        objectsDumped: number;
        objectErrors: number;
        truncated: boolean;
      };
      objects: string;
      widgets: string;
      steps: number;
    }>(`
      local directory = ${JSON.stringify(directory)}
      ${SNAPSHOT_READER}
      local native = makeClass("Class /Script/mucha.CPP_Thing", "/Script/mucha")
      native.fields = { { name = "Health", kind = "IntProperty" } }
      local widgetBase = makeClass("Class /Script/UMG.UserWidget", "/Script/UMG")
      local widgetClass = makeClass("WidgetBlueprintGeneratedClass /Game/UI.W_C", "/Game/UI", widgetBase)

      local good1 = makeObject("MuchaThing /Game/World.Good1", native)
      good1.values.Health = 10
      local broken = makeObject("MuchaThing /Game/World.Broken", native)
      broken.boom = true
      local expired = makeObject("MuchaThing /Game/World.Expired", native)
      expired.values.Health = 30
      local good2 = makeObject("MuchaThing /Game/World.Good2", native)
      good2.values.Health = 20
      local templateWidget = makeObject("W_C /Game/UI/W.Default__W_C:WidgetTree.SubWidget", widgetClass)
      templateWidget.inViewport = true
      local liveWidget = makeObject("W_C /Game/World.LiveWidget", widgetClass)
      liveWidget.inViewport = true
      liveWidget.members.Visibility = { text = "Visible" }

      local baseRead = reader.read
      function reader.read(object, property)
        if object.boom then error("object exploded") end
        return baseRead(object, property)
      end
      -- Also make one object fail completely inside scanObject via an unguarded metatable error.
      local badInScan = setmetatable({}, {
        __index = function() error("corrupt userdata") end,
      })

      local allObjects = { good1, badInScan, broken, expired, good2, templateWidget, liveWidget }
      local stepper = snapshot.create({
        directory = directory, reader = reader,
        limits = { batchSize = 2 },
        forEachUObject = function(callback)
          for _, object in ipairs(allObjects) do callback(object) end
        end,
        findFirstOf = function() return nil end,
        time = function() return 123 end,
      })

      local steps = 0
      local output
      while true do
        steps = steps + 1
        if steps == 2 then
          -- Simulate GC invalidating an object between scan and batch dump.
          expired.valid = false
        end
        local done, res = stepper.step()
        if done then output = res; break end
      end
      return {
        stats = output.stats,
        objects = readOutput("objects.jsonl"),
        widgets = readOutput("widgets.jsonl"),
        steps = steps,
      }
    `);
    assert.ok(result.steps >= 3, "should execute across multiple batch steps");
    const objectLines = result.objects
      .trim()
      .split("\n")
      .map((line) => JSON.parse(line) as { name: string; properties: Record<string, unknown> });
    const names = objectLines.map((row) => row.name);
    assert.ok(names.includes("MuchaThing /Game/World.Good1"));
    assert.ok(names.includes("MuchaThing /Game/World.Good2"));
    assert.ok(names.includes("MuchaThing /Game/World.Broken"));
    assert.ok(!names.includes("MuchaThing /Game/World.Expired"));
    const brokenRow = objectLines.find((row) => row.name.includes("Broken"));
    assert.equal(typeof brokenRow?.properties.Health, "string");
    assert.match(brokenRow?.properties.Health as string, /^<error: /u);

    const widgetLines = result.widgets
      .trim()
      .split("\n")
      .map((line) => JSON.parse(line) as { name: string });
    assert.equal(widgetLines.length, 1);
    assert.equal(widgetLines[0]?.name, "W_C /Game/World.LiveWidget");
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("jobs dispatches all UE4SS dump functions, logs start/done, and runs batched snapshots", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-jobs-"));
  try {
    const result = luaJson<{
      calls: string[];
      missing: string;
      snapshotFiles: string[];
      logs: string[];
      batchedTicks: number;
      batchedOk: boolean;
      batchedDumped: number;
      batchedLogs: string[];
    }>(`
      local directory = ${JSON.stringify(directory)}
      ${SNAPSHOT_READER}
      local jobs = require("ffrdump.jobs")
      local calls = {}
      local logs = {}
      local env = {
        log = function(line) logs[#logs + 1] = line end,
        snapshot = function() return { files = { "context.json" }, stats = {} } end,
      }
      for _, name in ipairs({ "DumpAllObjects", "DumpUSMAP", "DumpJMAP", "GenerateSDK",
        "GenerateUHTCompatibleHeaders" }) do
        env[name] = function() calls[#calls + 1] = name end
      end
      jobs.run({ id = "${VALID_ID}", job = "snapshot", label = "field-1" }, env)
      for _, job in ipairs({ "objects", "usmap", "jmap", "sdk", "uht" }) do
        local res = jobs.run({ id = "20261124-031500-field-1-" .. job, job = job, label = "field-1" }, env)
        assert(res.ok)
      end
      local missing = jobs.run({ id = "20261124-031500-field-1-objects", job = "objects", label = "field-1" }, {
        log = function(line) logs[#logs + 1] = line end,
      })

      local class = makeClass("Class /Script/mucha.CPP_Thing", "/Script/mucha")
      local allObjects = {}
      for index = 1, 5 do allObjects[index] = makeObject("Thing " .. index, class) end
      local batchedTicks = 0
      local batchedLogs = {}
      local batchedResult
      jobs.start(
        { id = "${VALID_ID}", job = "snapshot", label = "field-1" },
        {
          log = function(line) batchedLogs[#batchedLogs + 1] = line end,
          snapshotCreate = function()
            return snapshot.create({
              directory = directory, reader = reader, batched = true,
              limits = { batchSize = 2, batchSeconds = 10 },
              forEachUObject = function(callback)
                for _, object in ipairs(allObjects) do callback(object) end
              end,
              findFirstOf = function() return nil end,
              time = function() return 123 end,
            })
          end,
          schedule = function(stepFn)
            while true do
              batchedTicks = batchedTicks + 1
              if stepFn() then break end
            end
          end,
        },
        function(res) batchedResult = res end
      )

      return {
        calls = calls,
        missing = missing.error,
        snapshotFiles = jobs.run({ id = "${VALID_ID}", job = "snapshot", label = "field-1" }, env).files,
        logs = logs,
        batchedTicks = batchedTicks,
        batchedOk = batchedResult.ok,
        batchedDumped = batchedResult.stats.objectsDumped,
        batchedLogs = batchedLogs,
      }
    `);
    assert.deepEqual(result.calls, [
      "DumpAllObjects",
      "DumpUSMAP",
      "DumpJMAP",
      "GenerateSDK",
      "GenerateUHTCompatibleHeaders",
    ]);
    assert.match(result.missing, /DumpAllObjects/u);
    assert.deepEqual(result.snapshotFiles, ["context.json"]);
    assert.ok(
      result.logs.some((line) => line.startsWith("[FfrDump] start ") && line.endsWith("\n")),
    );
    assert.ok(
      result.logs.some((line) => line.startsWith("[FfrDump] done ") && line.endsWith("\n")),
    );
    assert.ok(
      result.batchedTicks >= 3,
      "batched snapshot should run across multiple game-thread ticks",
    );
    assert.equal(result.batchedOk, true);
    assert.equal(result.batchedDumped, 5);
    assert.equal(result.batchedLogs.length, 2);
    assert.match(result.batchedLogs[0] ?? "", /^\[FfrDump\] start .*job=snapshot.*\n$/u);
    assert.match(result.batchedLogs[1] ?? "", /^\[FfrDump\] done .*job=snapshot ok=true.*\n$/u);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

const HOTKEY_FS_STUB = `
  local stored = {}
  local existingDirs = { [directory] = true }
  io = io or {}
  io.open = function(path, mode)
    if mode == "wb" or mode == "ab" then
      local initial = (mode == "ab" and stored[path]) or ""
      local handle = { parts = { initial } }
      function handle:write(text)
        self.parts[#self.parts + 1] = text
        return self
      end
      function handle:close()
        stored[path] = table.concat(self.parts)
        return true
      end
      return handle
    end
    local body = stored[path]
    if body == nil then return nil, "missing" end
    local handle = {}
    function handle:read() return body end
    function handle:close() return true end
    return handle
  end
  os.remove = function(path)
    stored[path] = nil
    return true
  end
  os.rename = function(from, to)
    if from == to then
      if existingDirs[from] then return true end
      return nil, "missing", 2
    end
    if stored[from] == nil then return nil, "missing", 2 end
    stored[to] = stored[from]
    stored[from] = nil
    return true
  end
`;

void test("hotkey F9 captures batched prefixed snapshots, persists seq and ignores presses while busy", () => {
  const directory = "/virtual/hotkey";
  const result = luaJson<{
    registered: boolean;
    firstOk: boolean;
    firstId: string;
    midBusyOk: boolean;
    midBusyReason: string;
    secondOk: boolean;
    secondId: string;
    reloadedId: string;
    ticks: number;
    busyAfter: boolean;
    heartbeatCalls: number;
    logs: string[];
    done1: string;
    context1: string;
    objects1: string;
    classes1: string;
    widgets1: string;
    texts1: string;
    done2: string;
    seqFile: string;
  }>(`
      local directory = ${JSON.stringify(directory)}
      ${HOTKEY_FS_STUB}
      local snapshot = require("ffrdump.snapshot")
      local hotkey = require("ffrdump.hotkey")

      local reader = {}
      function reader.isValid(object) return object.valid ~= false end
      function reader.fullName(object) return object.fullName end
      function reader.shortName(object) return object.shortName end
      function reader.class(object) return object.class end
      function reader.super(class) return class.super end
      function reader.outer(object) return object.outer end
      function reader.address(object) return object.address end
      function reader.member(object, name) return object.members[name] end
      function reader.read(object, property) return object.values[property.name] end
      function reader.propertyName(property) return property.name end
      function reader.propertyType(property) return property.kind end
      function reader.fields(class) return class.fields or {} end
      function reader.objectName(object) return object.fullName end
      function reader.toString(value) return value.text end
      function reader.inViewport(object) return object.inViewport end

      local native = {
        fullName = "Class /Script/mucha.CPP_Chest",
        address = 1,
        outer = { fullName = "Package /Script/mucha" },
        fields = { { name = "Opened", kind = "BoolProperty" } },
      }
      local allObjects = {}
      for index = 1, 5 do
        allObjects[index] = {
          fullName = "CPP_Chest /Game/World.Chest_" .. index,
          shortName = "Chest_" .. index,
          class = native,
          values = { Opened = index == 1 },
          members = {},
        }
      end
      local worldObj = {
        fullName = "World /Game/Maps/Field01.Field01",
        valid = true,
      }

      local busy = false
      local heartbeatCalls = 0
      local logs = {}
      local ticks = 0
      local midBusyOk, midBusyReason
      local bound = {}
      local fakeTime = 1795458645

      local function fakeDate(fmt, ts)
        if fmt == "!%Y%m%d-%H%M%S" then
          if ts == 1795458645 then return "20261125-023045" end
          if ts == 1795458650 then return "20261125-023050" end
          return "20261125-023055"
        end
        return "2026-11-25 10:30:45"
      end

      local function makeController(stepHook)
        return hotkey.create({
          directory = directory,
          log = function(line) logs[#logs + 1] = line end,
          time = function() return fakeTime end,
          date = fakeDate,
          isBusy = function() return busy end,
          setBusy = function(value) busy = value end,
          onHeartbeat = function() heartbeatCalls = heartbeatCalls + 1 end,
          schedule = function(stepFn)
            local stepIndex = 0
            while true do
              stepIndex = stepIndex + 1
              ticks = ticks + 1
              if stepHook and stepIndex == 2 then stepHook() end
              if stepFn() then break end
            end
          end,
          snapshotCreate = function(filePrefix)
            return snapshot.create({
              directory = directory,
              filePrefix = filePrefix,
              reader = reader,
              batched = true,
              limits = { batchSize = 2, batchSeconds = 10 },
              forEachUObject = function(cb) for _, o in ipairs(allObjects) do cb(o) end end,
              findFirstOf = function(name) return name == "World" and worldObj or nil end,
              time = function() return fakeTime end,
            })
          end,
        })
      end

      local controller
      controller = makeController(function()
        -- Press F9 again while batch 2 is still in flight.
        midBusyOk, midBusyReason = controller.onSnapshotKey()
      end)

      local registered = hotkey.register(
        function(key, cb) bound[key] = cb end,
        { F9 = 0x78, F10 = 0x79 },
        controller,
        function(line) logs[#logs + 1] = line end
      )

      -- Trigger first snapshot via the registered F9 callback (also verify direct return via second).
      bound[0x78]()
      local firstId = "hotkey-0001-20261125-023045"
      fakeTime = 1795458650
      local secondOk, secondId = controller.onSnapshotKey()

      -- Simulate a mod reload / game restart: sequence should resume from seq.txt at 0003.
      fakeTime = 1795458655
      local reloaded = makeController()
      local _, reloadedId = reloaded.onSnapshotKey()

      local function readOut(name)
        local f = assert(io.open(directory .. "/" .. name, "rb"))
        local body = f:read("*a")
        f:close()
        return body
      end

      return {
        registered = registered,
        firstOk = true,
        firstId = firstId,
        midBusyOk = midBusyOk,
        midBusyReason = midBusyReason,
        secondOk = secondOk,
        secondId = secondId,
        reloadedId = reloadedId,
        ticks = ticks,
        busyAfter = busy,
        heartbeatCalls = heartbeatCalls,
        logs = logs,
        done1 = readOut(firstId .. "-done.json"),
        context1 = readOut(firstId .. "-context.json"),
        objects1 = readOut(firstId .. "-objects.jsonl"),
        classes1 = readOut(firstId .. "-classes.json"),
        widgets1 = readOut(firstId .. "-widgets.jsonl"),
        texts1 = readOut(firstId .. "-texts.jsonl"),
        done2 = readOut(secondId .. "-done.json"),
        seqFile = readOut("seq.txt"),
      }
    `);

  assert.equal(result.registered, true);
  assert.equal(result.midBusyOk, false);
  assert.equal(result.midBusyReason, "busy");
  assert.equal(result.secondOk, true);
  assert.equal(result.secondId, "hotkey-0002-20261125-023050");
  assert.equal(result.reloadedId, "hotkey-0003-20261125-023055");
  assert.ok(result.ticks >= 9, "each of the 3 snapshots should span multiple batch steps");
  assert.equal(result.busyAfter, false);
  assert.ok(result.heartbeatCalls >= 6);
  assert.ok(
    result.logs.some((line) => line.includes("hotkey F9 ignored") && line.endsWith("\n")),
    "should log ignored F9 while busy",
  );

  const done1 = JSON.parse(result.done1) as {
    id: string;
    job: string;
    label: string;
    ok: boolean;
    files: string[];
    stats: { objectsDumped: number };
  };
  assert.equal(done1.id, "hotkey-0001-20261125-023045");
  assert.equal(done1.job, "snapshot");
  assert.equal(done1.label, "hotkey-0001");
  assert.equal(done1.ok, true);
  assert.equal(done1.stats.objectsDumped, 5);
  assert.deepEqual(done1.files, [
    "hotkey-0001-20261125-023045-context.json",
    "hotkey-0001-20261125-023045-classes.json",
    "hotkey-0001-20261125-023045-objects.jsonl",
    "hotkey-0001-20261125-023045-widgets.jsonl",
    "hotkey-0001-20261125-023045-texts.jsonl",
  ]);

  const context1 = JSON.parse(result.context1) as { world: string; time: number };
  assert.equal(context1.world, "World /Game/Maps/Field01.Field01");
  assert.equal(context1.time, 1795458645);
  const objectLines = result.objects1
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line) as { name: string; properties: { Opened: boolean } });
  assert.equal(objectLines.length, 5);
  assert.equal(objectLines[0]?.properties.Opened, true);
  assert.ok(Array.isArray(JSON.parse(result.classes1)));
  assert.equal(result.widgets1, "");
  assert.equal(result.texts1, "");

  const done2 = JSON.parse(result.done2) as { id: string; label: string; ok: boolean };
  assert.equal(done2.id, "hotkey-0002-20261125-023050");
  assert.equal(done2.label, "hotkey-0002");
  assert.equal(done2.ok, true);
  assert.match(result.seqFile, /snapshot=3/u);
});

void test("hotkey F10 appends JSON marker lines with localTime, clock_ms, seq and world even while busy", () => {
  const directory = "/virtual/hotkey";
  const result = luaJson<{
    ok1: boolean;
    seq1: number;
    ok2: boolean;
    seq2: number;
    ok3: boolean;
    seq3: number;
    missingOk: boolean;
    missingReason: string;
    markers: string;
    logs: string[];
  }>(`
      local directory = ${JSON.stringify(directory)}
      ${HOTKEY_FS_STUB}
      local hotkey = require("ffrdump.hotkey")

      local logs = {}
      local now = 1795458645
      local clockNow = 10.0
      local currentWorld = {
        IsValid = function() return true end,
        GetFullName = function() return "World /Game/Maps/Dungeon01.Dungeon01" end,
      }

      local function makeController(dir)
        return hotkey.create({
          directory = dir,
          log = function(line) logs[#logs + 1] = line end,
          time = function() return now end,
          clock = function() return clockNow end,
          startedClock = 10.0,
          date = function(fmt)
            if fmt == "%Y-%m-%d %H:%M:%S" then
              return now == 1795458645 and "2026-11-25 10:30:45" or "2026-11-25 10:30:52"
            end
            return "20261125-023045"
          end,
          isBusy = function() return true end,
          findFirstOf = function(name) return name == "World" and currentWorld or nil end,
        })
      end

      local controller = makeController(directory)
      clockNow = 12.345
      local ok1, seq1 = controller.onMarkerKey()

      -- Invalid world degrades cleanly to "<unknown>" without error.
      now = 1795458652
      clockNow = 19.876
      currentWorld = {
        IsValid = function() return false end,
        GetFullName = function() error("must not call on invalid world") end,
      }
      local ok2, seq2 = controller.onMarkerKey()

      -- Reloading controller continues marker seq at 3.
      local reloaded = makeController(directory)
      clockNow = 25.0
      currentWorld = {
        IsValid = function() return true end,
        GetFullName = function() return "World /Game/Maps/Town01.Town01" end,
      }
      local ok3, seq3 = reloaded.onMarkerKey()

      -- Missing directory reports an error cleanly.
      local missing = makeController(directory .. "/does-not-exist")
      local missingOk, missingReason = missing.onMarkerKey()

      local f = assert(io.open(directory .. "/markers.jsonl", "rb"))
      local markers = f:read("*a")
      f:close()

      return {
        ok1 = ok1, seq1 = seq1,
        ok2 = ok2, seq2 = seq2,
        ok3 = ok3, seq3 = seq3,
        missingOk = missingOk, missingReason = missingReason,
        markers = markers,
        logs = logs,
      }
    `);

  assert.equal(result.ok1, true);
  assert.equal(result.seq1, 1);
  assert.equal(result.ok2, true);
  assert.equal(result.seq2, 2);
  assert.equal(result.ok3, true);
  assert.equal(result.seq3, 3);
  assert.equal(result.missingOk, false);
  assert.equal(result.missingReason, "missing_dir");

  const lines = result.markers
    .trim()
    .split("\n")
    .map(
      (line) =>
        JSON.parse(line) as {
          seq: number;
          localTime: string;
          time: number;
          clock_ms: number;
          world: string;
        },
    );
  assert.equal(lines.length, 3);
  assert.deepEqual(lines[0], {
    seq: 1,
    localTime: "2026-11-25 10:30:45",
    time: 1795458645,
    clock_ms: 2345,
    world: "World /Game/Maps/Dungeon01.Dungeon01",
  });
  assert.deepEqual(lines[1], {
    seq: 2,
    localTime: "2026-11-25 10:30:52",
    time: 1795458652,
    clock_ms: 9876,
    world: "<unknown>",
  });
  assert.deepEqual(lines[2], {
    seq: 3,
    localTime: "2026-11-25 10:30:52",
    time: 1795458652,
    clock_ms: 15000,
    world: "World /Game/Maps/Town01.Town01",
  });
});

void test("snapshot context.json reads pawnLocation (X/Y/Z) and pawnRotation (Pitch/Yaw/Roll)", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-snapshot-rot-"));
  try {
    const result = luaJson<{ context: string }>(`
      local directory = ${JSON.stringify(directory)}
      ${SNAPSHOT_READER}
      local realReader = snapshot.makeReader()
      local worldObj = {
        IsValid = function() return true end,
        GetFullName = function() return "World /Game/Map/Field01.Field01" end,
      }
      local pawnClass = {
        IsValid = function() return true end,
        GetFullName = function() return "BlueprintGeneratedClass /Game/BP_Player.BP_Player_C" end,
      }
      local pawnObj = {
        IsValid = function() return true end,
        GetFullName = function() return "BP_Player_C /Game/Map/Field01.BP_Player_C_0" end,
        GetClass = function() return pawnClass end,
        K2_GetActorLocation = function() return { X = 100.5, Y = -200.25, Z = 50.0 } end,
        K2_GetActorRotation = function() return { Pitch = 12.5, Yaw = -90.0, Roll = 0.0 } end,
      }
      local pcClass = {
        IsValid = function() return true end,
        GetFullName = function() return "Class /Script/mucha.CPP_MuchaPlayerController" end,
      }
      local pcObj = {
        IsValid = function() return true end,
        GetFullName = function() return "CPP_MuchaPlayerController /Game/Map/Field01.PC_0" end,
        GetClass = function() return pcClass end,
        Pawn = pawnObj,
      }
      snapshot.run({
        directory = directory,
        reader = realReader,
        forEachUObject = function() end,
        findFirstOf = function(name)
          if name == "World" then return worldObj end
          if name == "PlayerController" then return pcObj end
          return nil
        end,
        time = function() return 456 end,
      })
      return { context = readOutput("context.json") }
    `);
    const context = JSON.parse(result.context) as {
      world: string;
      pawnLocation: { x: number; y: number; z: number };
      pawnRotation: { pitch: number; yaw: number; roll: number };
    };
    assert.equal(context.world, "World /Game/Map/Field01.Field01");
    assert.deepEqual(context.pawnLocation, { x: 100.5, y: -200.25, z: 50 });
    assert.deepEqual(context.pawnRotation, { pitch: 12.5, yaw: -90, roll: 0 });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("main.lua and hotkey.lua compile as Lua 5.3", async () => {
  for (const relative of ["main.lua", join("ffrdump", "hotkey.lua")]) {
    const body = await readFile(join(SCRIPTS, relative), "utf8");
    const state = lauxlib.luaL_newstate();
    try {
      load(state, body);
    } finally {
      lua.lua_close(state);
    }
  }
});
