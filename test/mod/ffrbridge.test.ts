import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { lua, lauxlib, lualib, to_jsstring, to_luastring } from "fengari";

const SCRIPTS = join(import.meta.dirname, "..", "..", "mod", "FfrBridge", "Scripts");
const MOD_SYNC = join(import.meta.dirname, "..", "..", "src", "host", "cli", "mod-sync.ts");

function load(state: ReturnType<typeof lauxlib.luaL_newstate>, code: string): void {
  const status = lauxlib.luaL_loadbuffer(
    state,
    to_luastring(code),
    null,
    to_luastring("ffrbridge-test"),
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
      ? '\nresult = require("ffrbridge.json").encode((function()\n'
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

const FAKE_FS_HELPER = `
  local function makeFakeFs(dirExists)
    local files = {}
    local state = { dirExists = dirExists ~= false, files = files, ops = {} }
    local fs = {}
    function fs.open(path, mode)
      state.ops[#state.ops + 1] = { op = "open", path = path, mode = mode }
      if mode == "wb" then
        local handle = { parts = {} }
        function handle:write(text)
          state.ops[#state.ops + 1] = { op = "write", path = path }
          self.parts[#self.parts + 1] = text
          return self
        end
        function handle:close()
          state.ops[#state.ops + 1] = { op = "close", path = path }
          files[path] = table.concat(self.parts)
          return true
        end
        return handle
      end
      local body = files[path]
      if body == nil then return nil, "missing", 2 end
      local handle = {}
      function handle:read() return body end
      function handle:close() return true end
      return handle
    end
    function fs.remove(path)
      state.ops[#state.ops + 1] = { op = "remove", path = path }
      if files[path] == nil then return nil, "missing", 2 end
      files[path] = nil
      return true
    end
    function fs.rename(from, to)
      if from == to then
        if state.dirExists then return true end
        return nil, "missing", 2
      end
      state.ops[#state.ops + 1] = { op = "rename", from = from, to = to }
      if files[from] == nil then return nil, "missing", 2 end
      files[to] = files[from]
      files[from] = nil
      return true
    end
    return fs, state
  end
`;

const FAKE_UE_HELPER = `
  local function makeClass(fullName)
    return {
      IsValid = function() return true end,
      GetFullName = function() return fullName end,
    }
  end

  local function makeActor(className, extras)
    local cls = makeClass(className)
    local obj = {
      IsValid = function() return true end,
      GetClass = function() return cls end,
    }
    if extras then
      for key, value in pairs(extras) do obj[key] = value end
    end
    return obj
  end

  local function makeHealthyFindFirstOf()
    local world = {
      IsValid = function() return true end,
      GetFullName = function() return "World /Game/Maps/Field01/Field01.Field01" end,
    }
    local pawn = makeActor("BlueprintGeneratedClass /Game/BP_Hero.BP_Hero_C", {
      K2_GetActorLocation = function() return { X = 123.45, Y = -67.84, Z = -0.04 } end,
    })
    local pc = makeActor("Class /Script/mucha.CPP_FieldController", { Pawn = pawn })
    local gm = makeActor("Class /Script/mucha.CPP_MuchaGameModeBase")
    local gi = makeActor("Class /Script/mucha.CPP_MuchaGameInstance")
    local tableByClass = {
      World = world,
      PlayerController = pc,
      GameModeBase = gm,
      GameInstance = gi,
    }
    return function(name) return tableByClass[name] end
  end
`;

interface BridgeStateData {
  world: string | null;
  map: string | null;
  player_controller_class: string | null;
  pawn_class: string | null;
  game_mode_class: string | null;
  game_instance_class: string | null;
  pawn_location: { x: number; y: number; z: number } | null;
  field: Record<string, unknown> | null;
  battle: Record<string, unknown> | null;
  ui: Record<string, unknown> | null;
  party: Record<string, unknown> | null;
  errors: string[];
  bridge_version: string;
}

interface BridgeStateDoc {
  v: number;
  session: string;
  seq: number;
  clock_ms: number;
  data: BridgeStateData;
}

interface BridgeAckDoc {
  v: number;
  id: string;
  ok: boolean;
  result: Record<string, unknown> | null;
  error: string | null;
}

void test("json encodes and decodes objects, arrays, escapes, unicode and rejects malformed input", () => {
  const sample = String.raw`{"v":1,"id":"0193f2","op":"ping","text":"中文\n\t\"\\\u4e2d\uD83D\uDE00","flag":true,"none":null,"nums":[0,-1.5,1e2]}`;
  const badControl = '{"a":"\n"}';
  const badSurrogate = String.raw`{"a":"\uD800"}`;
  const result = luaJson<{
    roundtrip: Record<string, unknown>;
    emptyArr: unknown[];
    emptyObj: Record<string, unknown>;
    badTrailing: string | null;
    badLeadingZero: string | null;
    badControl: string | null;
    badSurrogate: string | null;
  }>(`
    local json = require("ffrbridge.json")
    local sample = ${JSON.stringify(sample)}
    local decoded, err = json.decode(sample)
    assert(err == nil, tostring(err))
    local emptyArr = assert(json.decode("[]"))
    local emptyObj = assert(json.decode("{}"))
    local _, badTrailing = json.decode('{"v":1} trailing')
    local _, badLeadingZero = json.decode('{"v":01}')
    local _, badControl = json.decode(${JSON.stringify(badControl)})
    local _, badSurrogate = json.decode(${JSON.stringify(badSurrogate)})
    return {
      roundtrip = decoded,
      emptyArr = emptyArr,
      emptyObj = emptyObj,
      badTrailing = badTrailing or json.null,
      badLeadingZero = badLeadingZero or json.null,
      badControl = badControl or json.null,
      badSurrogate = badSurrogate or json.null,
    }
  `);
  assert.deepEqual(result.roundtrip, {
    v: 1,
    id: "0193f2",
    op: "ping",
    text: '中文\n\t"\\中😀',
    flag: true,
    none: null,
    nums: [0, -1.5, 100],
  });
  assert.deepEqual(result.emptyArr, []);
  assert.deepEqual(result.emptyObj, {});
  assert.ok(result.badTrailing);
  assert.ok(result.badLeadingZero);
  assert.ok(result.badControl);
  assert.ok(result.badSurrogate);
});

void test("state JSON encodes v, 6-char session, incrementing seq, integer clock_ms, null and errors array", () => {
  const result = luaJson<{
    raw1: string;
    raw2: string;
    raw3: string;
    generatedSession: string;
  }>(`
    ${FAKE_FS_HELPER}
    ${FAKE_UE_HELPER}
    local protocol = require("ffrbridge.protocol")
    local fs, fsState = makeFakeFs(true)
    local now = 10.0
    local bridge = protocol.create({
      directory = "C:\\\\game\\\\ue4ss\\\\ffr-out\\\\bridge",
      fs = fs,
      topics = {},
      findFirstOf = makeHealthyFindFirstOf(),
      clock = function() return now end,
      time = function() return 1000 end,
      log = function() end,
    })
    bridge.tick()
    local raw1 = fsState.files["C:\\\\game\\\\ue4ss\\\\ffr-out\\\\bridge\\\\state.json"]
    now = 10.5
    bridge.tick()
    local raw2 = fsState.files["C:\\\\game\\\\ue4ss\\\\ffr-out\\\\bridge\\\\state.json"]
    now = 12.25
    bridge.tick()
    local raw3 = fsState.files["C:\\\\game\\\\ue4ss\\\\ffr-out\\\\bridge\\\\state.json"]
    return {
      raw1 = raw1,
      raw2 = raw2,
      raw3 = raw3,
      generatedSession = bridge.session,
    }
  `);

  assert.match(result.generatedSession, /^[a-z0-9]{6}$/u);
  // Ensure v, seq and clock_ms serialize as JSON integers without .0 suffix for Rust serde.
  assert.match(result.raw1, /"v":1\b/u);
  assert.match(result.raw1, /"seq":1\b/u);
  assert.match(result.raw1, /"clock_ms":0\b/u);
  assert.match(result.raw2, /"seq":2\b/u);
  assert.match(result.raw2, /"clock_ms":500\b/u);
  assert.match(result.raw3, /"seq":3\b/u);
  assert.match(result.raw3, /"clock_ms":2250\b/u);

  const doc1 = JSON.parse(result.raw1) as BridgeStateDoc;
  assert.deepEqual(doc1, {
    v: 1,
    session: result.generatedSession,
    seq: 1,
    clock_ms: 0,
    data: {
      world: "World /Game/Maps/Field01/Field01.Field01",
      map: "Field01",
      player_controller_class: "Class /Script/mucha.CPP_FieldController",
      pawn_class: "BlueprintGeneratedClass /Game/BP_Hero.BP_Hero_C",
      game_mode_class: "Class /Script/mucha.CPP_MuchaGameModeBase",
      game_instance_class: "Class /Script/mucha.CPP_MuchaGameInstance",
      pawn_location: { x: 123.5, y: -67.8, z: 0 },
      field: null,
      battle: null,
      ui: null,
      party: null,
      errors: [],
      bridge_version: "1",
    },
  });

  const doc3 = JSON.parse(result.raw3) as BridgeStateDoc;
  assert.equal(doc3.session, doc1.session);
  assert.equal(doc3.seq, 3);
  assert.equal(doc3.clock_ms, 2250);
});

void test("state sampler records missing Pawn and location errors into errors without breaking other fields", () => {
  const result = luaJson<{
    missingPawn: BridgeStateData;
    brokenLocation: BridgeStateData;
    staleObjects: BridgeStateData;
  }>(`
    ${FAKE_UE_HELPER}
    local state = require("ffrbridge.state")

    -- Case 1: missing Pawn (pc.Pawn is nil)
    local world = {
      IsValid = function() return true end,
      GetFullName = function() return "World /Game/Maps/Title.Title" end,
    }
    local pcNoPawn = makeActor("Class /Script/mucha.CPP_TitleController", { Pawn = nil })
    local gm = makeActor("Class /Script/mucha.CPP_MuchaGameModeBase")
    local gi = makeActor("Class /Script/mucha.CPP_MuchaGameInstance")
    local missingPawn = state.sample({
      topics = {},
      findFirstOf = function(name)
        if name == "World" then return world end
        if name == "PlayerController" then return pcNoPawn end
        if name == "GameModeBase" then return gm end
        if name == "GameInstance" then return gi end
        return nil
      end,
    })

    -- Case 2: Pawn exists, K2_GetActorLocation throws
    local badLocPawn = makeActor("BlueprintGeneratedClass /Game/BP_Hero.BP_Hero_C", {
      K2_GetActorLocation = function() error("actor destroyed") end,
    })
    local pcBadLoc = makeActor("Class /Script/mucha.CPP_FieldController", { Pawn = badLocPawn })
    local brokenLocation = state.sample({
      topics = {},
      findFirstOf = function(name)
        if name == "World" then return world end
        if name == "PlayerController" then return pcBadLoc end
        if name == "GameModeBase" then return gm end
        if name == "GameInstance" then return gi end
        return nil
      end,
    })

    -- Case 3: stale invalid World and invalid GameMode
    local stale = { IsValid = function() return false end }
    local staleObjects = state.sample({
      topics = {},
      findFirstOf = function(name)
        if name == "World" then return stale end
        if name == "PlayerController" then return pcNoPawn end
        if name == "GameModeBase" then return stale end
        if name == "GameInstance" then return gi end
        return nil
      end,
    })

    return {
      missingPawn = missingPawn,
      brokenLocation = brokenLocation,
      staleObjects = staleObjects,
    }
  `);

  assert.equal(result.missingPawn.world, "World /Game/Maps/Title.Title");
  assert.equal(result.missingPawn.map, "Title");
  assert.equal(
    result.missingPawn.player_controller_class,
    "Class /Script/mucha.CPP_TitleController",
  );
  assert.equal(result.missingPawn.game_mode_class, "Class /Script/mucha.CPP_MuchaGameModeBase");
  assert.equal(result.missingPawn.game_instance_class, "Class /Script/mucha.CPP_MuchaGameInstance");
  assert.equal(result.missingPawn.pawn_class, null);
  assert.equal(result.missingPawn.pawn_location, null);
  assert.equal(result.missingPawn.bridge_version, "1");
  assert.equal(result.missingPawn.errors.length, 1);
  assert.match(result.missingPawn.errors[0] ?? "", /pawn/u);

  assert.equal(result.brokenLocation.world, "World /Game/Maps/Title.Title");
  assert.equal(result.brokenLocation.map, "Title");
  assert.equal(result.brokenLocation.pawn_class, "BlueprintGeneratedClass /Game/BP_Hero.BP_Hero_C");
  assert.equal(result.brokenLocation.pawn_location, null);
  assert.equal(result.brokenLocation.game_mode_class, "Class /Script/mucha.CPP_MuchaGameModeBase");
  assert.equal(
    result.brokenLocation.game_instance_class,
    "Class /Script/mucha.CPP_MuchaGameInstance",
  );
  assert.equal(result.brokenLocation.errors.length, 1);
  assert.match(result.brokenLocation.errors[0] ?? "", /pawn_location.*actor destroyed/u);

  assert.equal(result.staleObjects.world, null);
  assert.equal(result.staleObjects.map, null);
  assert.equal(result.staleObjects.game_mode_class, null);
  assert.equal(
    result.staleObjects.game_instance_class,
    "Class /Script/mucha.CPP_MuchaGameInstance",
  );
  assert.equal(result.staleObjects.errors.length, 3);
});

void test("commands handle ping, duplicate id, unknown op, and bad JSON with or without id", () => {
  const result = luaJson<{
    pingAck: string | null;
    pingCommandLeft: boolean;
    dupAckWritten: boolean;
    dupCommandLeft: boolean;
    unknownAck: string | null;
    badWithIdAck: string | null;
    badNoIdAckWritten: boolean;
    badNoIdCommandLeft: boolean;
    badMissingIdFieldAckWritten: boolean;
    badVersionAck: string | null;
    logs: string[];
  }>(`
    ${FAKE_FS_HELPER}
    ${FAKE_UE_HELPER}
    local json = require("ffrbridge.json")
    local protocol = require("ffrbridge.protocol")
    local fs, fsState = makeFakeFs(true)
    local dir = "/tmp/bridge"
    local cmdPath = dir .. "/command.json"
    local ackPath = dir .. "/ack.json"
    local logs = {}
    local unixTime = 100

    local bridge = protocol.create({
      directory = dir,
      session = "abc123",
      fs = fs,
      findFirstOf = makeHealthyFindFirstOf(),
      clock = function() return 1.0 end,
      time = function() return unixTime end,
      log = function(line) logs[#logs + 1] = line end,
    })

    -- 1. ping
    fsState.files[cmdPath] = '{"v":1,"id":"id-ping-1","op":"ping","args":{}}'
    bridge.tick()
    local pingAck = fsState.files[ackPath]
    local pingCommandLeft = fsState.files[cmdPath] ~= nil

    -- 2. duplicate id is ignored (does not rewrite ack, cleans up command.json)
    fsState.files[ackPath] = nil
    fsState.files[cmdPath] = '{"v":1,"id":"id-ping-1","op":"ping","args":{}}'
    bridge.tick()
    local dupAckWritten = fsState.files[ackPath] ~= nil
    local dupCommandLeft = fsState.files[cmdPath] ~= nil

    -- 3. unknown op returns ok: false
    fsState.files[ackPath] = nil
    fsState.files[cmdPath] = '{"v":1,"id":"id-unk-2","op":"press_key","args":{"key":"A"}}'
    bridge.tick()
    local unknownAck = fsState.files[ackPath]

    -- 4. bad JSON that still contains an extractable id -> ok: false ack
    fsState.files[ackPath] = nil
    fsState.files[cmdPath] = '{"v":1,"id":"id-bad-3","op":'
    bridge.tick()
    local badWithIdAck = fsState.files[ackPath]

    -- 5. bad JSON with no id -> no ack, logs error (rate-limited within 60s)
    unixTime = 200
    fsState.files[ackPath] = nil
    fsState.files[cmdPath] = '{not valid json'
    bridge.tick()
    local badNoIdAckWritten = fsState.files[ackPath] ~= nil
    local badNoIdCommandLeft = fsState.files[cmdPath] ~= nil

    -- 6. valid JSON missing id -> no ack, logs suppressed within 60s
    unixTime = 210
    fsState.files[ackPath] = nil
    fsState.files[cmdPath] = '{"v":1,"op":"ping"}'
    bridge.tick()
    local badMissingIdFieldAckWritten = fsState.files[ackPath] ~= nil

    -- 7. unsupported protocol version with id -> ok: false ack
    fsState.files[ackPath] = nil
    fsState.files[cmdPath] = '{"v":2,"id":"id-ver-4","op":"ping"}'
    bridge.tick()
    local badVersionAck = fsState.files[ackPath]

    return {
      pingAck = pingAck or json.null,
      pingCommandLeft = pingCommandLeft,
      dupAckWritten = dupAckWritten,
      dupCommandLeft = dupCommandLeft,
      unknownAck = unknownAck or json.null,
      badWithIdAck = badWithIdAck or json.null,
      badNoIdAckWritten = badNoIdAckWritten,
      badNoIdCommandLeft = badNoIdCommandLeft,
      badMissingIdFieldAckWritten = badMissingIdFieldAckWritten,
      badVersionAck = badVersionAck or json.null,
      logs = json.array(logs),
    }
  `);

  assert.ok(result.pingAck);
  assert.deepEqual(JSON.parse(result.pingAck) as BridgeAckDoc, {
    v: 1,
    id: "id-ping-1",
    ok: true,
    result: { pong: true, seq: 1 },
    error: null,
  });
  assert.equal(result.pingCommandLeft, false);

  assert.equal(result.dupAckWritten, false);
  assert.equal(result.dupCommandLeft, false);

  assert.ok(result.unknownAck);
  assert.deepEqual(JSON.parse(result.unknownAck) as BridgeAckDoc, {
    v: 1,
    id: "id-unk-2",
    ok: false,
    result: null,
    error: "不认识的命令 press_key",
  });

  assert.ok(result.badWithIdAck);
  const badAck = JSON.parse(result.badWithIdAck) as BridgeAckDoc;
  assert.equal(badAck.v, 1);
  assert.equal(badAck.id, "id-bad-3");
  assert.equal(badAck.ok, false);
  assert.equal(badAck.result, null);
  assert.match(badAck.error ?? "", /JSON 解析失败/u);

  assert.equal(result.badNoIdAckWritten, false);
  assert.equal(result.badNoIdCommandLeft, false);
  assert.equal(result.badMissingIdFieldAckWritten, false);
  assert.ok(result.badVersionAck);
  const badVer = JSON.parse(result.badVersionAck) as BridgeAckDoc;
  assert.equal(badVer.id, "id-ver-4");
  assert.equal(badVer.ok, false);
  assert.equal(badVer.result, null);
  assert.match(badVer.error ?? "", /协议版本/u);
  // Logged once at t=100 (badWithId), once at t=200 (badNoId), suppressed at t=210 (within 60s).
  assert.equal(result.logs.length, 2);
  assert.match(result.logs[1] ?? "", /\[FfrBridge\] command/u);
});

void test("atomic write executes write tmp -> remove old -> rename in exact order", () => {
  interface FsOp {
    op: string;
    path?: string;
    mode?: string;
    from?: string;
    to?: string;
  }
  const result = luaJson<{ stateOps: FsOp[]; ackOps: FsOp[] }>(`
    ${FAKE_FS_HELPER}
    ${FAKE_UE_HELPER}
    local protocol = require("ffrbridge.protocol")
    local fs, fsState = makeFakeFs(true)
    local dir = "C:\\\\game\\\\ue4ss\\\\ffr-out\\\\bridge"
    local bridge = protocol.create({
      directory = dir,
      session = "s12345",
      fs = fs,
      findFirstOf = makeHealthyFindFirstOf(),
      clock = function() return 0 end,
      log = function() end,
    })

    bridge.writeState()
    local stateOps = {}
    for index, entry in ipairs(fsState.ops) do stateOps[index] = entry end

    fsState.ops = {}
    fsState.files[dir .. "\\\\command.json"] = '{"v":1,"id":"u1","op":"ping","args":{}}'
    bridge.handleCommand()
    local ackOps = {}
    for index, entry in ipairs(fsState.ops) do ackOps[index] = entry end

    return { stateOps = stateOps, ackOps = ackOps }
  `);

  const dir = "C:\\game\\ue4ss\\ffr-out\\bridge";
  assert.deepEqual(result.stateOps, [
    { op: "open", path: `${dir}\\state.tmp`, mode: "wb" },
    { op: "write", path: `${dir}\\state.tmp` },
    { op: "close", path: `${dir}\\state.tmp` },
    { op: "remove", path: `${dir}\\state.json` },
    { op: "rename", from: `${dir}\\state.tmp`, to: `${dir}\\state.json` },
  ]);

  assert.deepEqual(result.ackOps, [
    { op: "open", path: `${dir}\\command.json`, mode: "rb" },
    { op: "open", path: `${dir}\\ack.json`, mode: "rb" },
    { op: "open", path: `${dir}\\ack.tmp`, mode: "wb" },
    { op: "write", path: `${dir}\\ack.tmp` },
    { op: "close", path: `${dir}\\ack.tmp` },
    { op: "remove", path: `${dir}\\ack.json` },
    { op: "rename", from: `${dir}\\ack.tmp`, to: `${dir}\\ack.json` },
    { op: "open", path: `${dir}\\command.json`, mode: "rb" },
    { op: "remove", path: `${dir}\\command.json` },
  ]);
});

void test("missing bridge directory writes nothing, logs nothing, and starts at seq 1 once created", () => {
  const result = luaJson<{
    tickBefore: boolean;
    opsBefore: number;
    logsBefore: number;
    seqBefore: number;
    tickAfter: boolean;
    seqAfter: number;
  }>(`
    ${FAKE_FS_HELPER}
    ${FAKE_UE_HELPER}
    local protocol = require("ffrbridge.protocol")
    local fs, fsState = makeFakeFs(false)
    local logs = {}
    local bridge = protocol.create({
      directory = "/tmp/missing-bridge",
      session = "a0b1c2",
      fs = fs,
      findFirstOf = makeHealthyFindFirstOf(),
      clock = function() return 0 end,
      log = function(line) logs[#logs + 1] = line end,
    })

    local tickBefore = bridge.tick()
    local opsBefore = #fsState.ops
    local logsBefore = #logs
    local seqBefore = bridge.getSeq()

    fsState.dirExists = true
    local tickAfter = bridge.tick()
    local seqAfter = bridge.getSeq()

    return {
      tickBefore = tickBefore,
      opsBefore = opsBefore,
      logsBefore = logsBefore,
      seqBefore = seqBefore,
      tickAfter = tickAfter,
      seqAfter = seqAfter,
    }
  `);

  assert.equal(result.tickBefore, false);
  assert.equal(result.opsBefore, 0);
  assert.equal(result.logsBefore, 0);
  assert.equal(result.seqBefore, 0);
  assert.equal(result.tickAfter, true);
  assert.equal(result.seqAfter, 1);
});

void test("main.lua compiles as Lua 5.3", async () => {
  const body = await readFile(join(SCRIPTS, "main.lua"), "utf8");
  const state = lauxlib.luaL_newstate();
  try {
    load(state, body);
  } finally {
    lua.lua_close(state);
  }
});

void test("mod-sync supports FfrBridge in dry-run mode without connecting", async () => {
  const directory = await mkdtemp(join(tmpdir(), "ffr-bridge-sync-"));
  try {
    const machineEnv = join(directory, "machine.env");
    await writeFile(
      machineEnv,
      [
        "LAPTOP_HOST=192.0.2.10",
        "LAPTOP_USER=ffr",
        `LAPTOP_KEY=${join(directory, "id_ed25519")}`,
        "LAPTOP_ROOT=C:/ffr",
        "FFR_GAME_DIR=C:/game/Win64",
      ].join("\n") + "\n",
      "utf8",
    );
    const child = spawn(
      process.execPath,
      [MOD_SYNC, "FfrBridge", "--dry-run", "--machine-env", machineEnv],
      { stdio: ["ignore", "pipe", "pipe"] },
    );
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk: Buffer) => {
      stdout += chunk.toString("utf8");
    });
    child.stderr.on("data", (chunk: Buffer) => {
      stderr += chunk.toString("utf8");
    });
    const code = await new Promise<number>((resolve, reject) => {
      child.on("error", reject);
      child.on("close", (value) => {
        resolve(value ?? -1);
      });
    });
    assert.equal(code, 0, stderr);
    assert.match(stdout, /ue4ss\/Mods\/FfrBridge/u);
    assert.match(stdout, /启用 FfrBridge/u);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

void test("extractMap handles UE World full names and edge cases", () => {
  const result = luaJson<Record<string, string | null>>(`
    local json = require("ffrbridge.json")
    local state = require("ffrbridge.state")
    local function pick(v) return state.extractMap(v) or json.null end
    return {
      standard = pick("World /Game/Maps/Title/TitleMap.TitleMap"),
      noDot = pick("World /Game/Maps/Field01"),
      bare = pick("Field02"),
      empty = pick(""),
      trailingDot = pick("World /Game/."),
    }
  `);
  assert.equal(result.standard, "TitleMap");
  assert.equal(result.noDot, "Field01");
  assert.equal(result.bare, "Field02");
  assert.equal(result.empty, null);
  assert.equal(result.trailingDot, null);
});

void test("v1 wiring connects all four topics, deduplicates ctx.find per tick, preserves ctx.cache, and isolates topic errors", () => {
  const result = luaJson<{
    tick1: BridgeStateData;
    tick2: BridgeStateData;
    sharedFindCallsTick1: number;
    sharedFindCallsTick2: number;
    withSkeletons: BridgeStateData;
    defaultLoaded: BridgeStateData;
  }>(`
    ${FAKE_UE_HELPER}
    local state = require("ffrbridge.state")
    local baseFind = makeHealthyFindFirstOf()
    local sharedSingleton = { IsValid = function() return true end, tag = "shared" }
    local findAllItems = {
      { IsValid = function() return true end, id = 1 },
      { IsValid = function() return false end, id = 2 },
      { IsValid = function() return true end, id = 3 },
    }
    local sharedCalls = 0
    local function findFirst(name)
      if name == "SharedSingleton" then
        sharedCalls = sharedCalls + 1
        return sharedSingleton
      end
      return baseFind(name)
    end

    local topics = {
      field = {
        name = "field",
        collect = function(ctx)
          ctx.cache.hits = (ctx.cache.hits or 0) + 1
          local obj = ctx.find("SharedSingleton")
          local validList = ctx.findAll("SomeList")
          return {
            hits = ctx.cache.hits,
            tick = ctx.tick,
            clock_ms = ctx.clockMs,
            shared_tag = obj and obj.tag or nil,
            valid_count = #validList,
          }
        end,
      },
      battle = {
        name = "battle",
        collect = function(ctx)
          -- Also calls ctx.find("SharedSingleton") in the same tick; must reuse per-tick cache.
          local obj = ctx.find("SharedSingleton")
          assert(obj == sharedSingleton)
          error("battle exploded")
        end,
      },
      ui = {
        name = "ui",
        collect = function(ctx)
          ctx.cache.uiSeen = (ctx.cache.uiSeen or 0) + 1
          return { seen = ctx.cache.uiSeen }
        end,
      },
      party = {
        name = "party",
        collect = function()
          return nil
        end,
      },
    }

    local sampler = state.create({
      topics = topics,
      findFirstOf = findFirst,
      findAllOf = function() return findAllItems end,
    })

    sharedCalls = 0
    local tick1 = sampler.sample(500)
    local sharedFindCallsTick1 = sharedCalls

    sharedCalls = 0
    local tick2 = sampler.sample(1000)
    local sharedFindCallsTick2 = sharedCalls

    -- Verify empty-shell topics work cleanly alongside real field topic.
    local realField = require("ffrbridge.topics.field")
    local emptyStub = function(name)
      return { name = name, commands = {}, collect = function() return nil end }
    end
    local withSkeletons = state.sample({
      topics = {
        field = realField,
        battle = emptyStub("battle"),
        ui = emptyStub("ui"),
        party = emptyStub("party"),
      },
      findFirstOf = baseFind,
      clockMs = 100,
      tick = 1,
    })

    -- Also verify default state.loadTopics() loads all 4 modules cleanly without throwing.
    local defaultLoaded = state.sample({
      findFirstOf = baseFind,
      clockMs = 100,
      tick = 1,
    })

    return {
      tick1 = tick1,
      tick2 = tick2,
      sharedFindCallsTick1 = sharedFindCallsTick1,
      sharedFindCallsTick2 = sharedFindCallsTick2,
      withSkeletons = withSkeletons,
      defaultLoaded = defaultLoaded,
    }
  `);

  assert.equal(result.tick1.bridge_version, "1");
  assert.deepEqual(result.tick1.field, {
    hits: 1,
    tick: 1,
    clock_ms: 500,
    shared_tag: "shared",
    valid_count: 2,
  });
  assert.equal(result.tick1.battle, null);
  assert.deepEqual(result.tick1.ui, { seen: 1 });
  assert.equal(result.tick1.party, null);
  assert.equal(result.tick1.errors.length, 1);
  assert.match(result.tick1.errors[0] ?? "", /^battle:.*battle exploded/u);
  assert.equal(result.sharedFindCallsTick1, 1);

  assert.deepEqual(result.tick2.field, {
    hits: 2,
    tick: 2,
    clock_ms: 1000,
    shared_tag: "shared",
    valid_count: 2,
  });
  assert.deepEqual(result.tick2.ui, { seen: 2 });
  assert.equal(result.sharedFindCallsTick2, 1);

  assert.equal(result.withSkeletons.bridge_version, "1");
  assert.ok(result.withSkeletons.field !== null);
  assert.equal(result.withSkeletons.battle, null);
  assert.equal(result.withSkeletons.ui, null);
  assert.equal(result.withSkeletons.party, null);

  assert.equal(result.defaultLoaded.bridge_version, "1");
  assert.ok(result.defaultLoaded.field !== null);
  assert.ok(
    result.defaultLoaded.battle === null || typeof result.defaultLoaded.battle === "object",
  );
  assert.ok(result.defaultLoaded.ui === null || typeof result.defaultLoaded.ui === "object");
  assert.ok(result.defaultLoaded.party === null || typeof result.defaultLoaded.party === "object");
});

void test("protocol dispatches topic commands on tick and rejects duplicate or ping-colliding command names at startup", () => {
  const result = luaJson<{
    okAck: string | null;
    errAck: string | null;
    throwAck: string | null;
    conflictErr: string | null;
    pingConflictErr: string | null;
  }>(`
    ${FAKE_FS_HELPER}
    ${FAKE_UE_HELPER}
    local json = require("ffrbridge.json")
    local protocol = require("ffrbridge.protocol")
    local fs, fsState = makeFakeFs(true)
    local dir = "/tmp/v1-cmd-bridge"
    local cmdPath = dir .. "/command.json"
    local ackPath = dir .. "/ack.json"

    local topics = {
      field = {
        name = "field",
        collect = function(ctx)
          ctx.cache.tag = "from-field"
          return { ready = true }
        end,
        commands = {
          echo_cache = function(ctx, args)
            return { tag = ctx.cache.tag, value = args.x, tick = ctx.tick }, nil
          end,
          fail_soft = function()
            return nil, "模拟业务失败"
          end,
          fail_throw = function()
            error("boom inside command")
          end,
        },
      },
      battle = { name = "battle", collect = function() return nil end, commands = {} },
      ui = { name = "ui", collect = function() return nil end, commands = {} },
      party = { name = "party", collect = function() return nil end, commands = {} },
    }

    local bridge = protocol.create({
      directory = dir,
      session = "v1sess",
      fs = fs,
      topics = topics,
      findFirstOf = makeHealthyFindFirstOf(),
      clock = function() return 2.0 end,
      log = function() end,
    })

    fsState.files[cmdPath] = '{"v":1,"id":"cmd-1","op":"echo_cache","args":{"x":42}}'
    bridge.tick()
    local okAck = fsState.files[ackPath]

    fsState.files[ackPath] = nil
    fsState.files[cmdPath] = '{"v":1,"id":"cmd-2","op":"fail_soft","args":{}}'
    bridge.tick()
    local errAck = fsState.files[ackPath]

    fsState.files[ackPath] = nil
    fsState.files[cmdPath] = '{"v":1,"id":"cmd-3","op":"fail_throw","args":{}}'
    bridge.tick()
    local throwAck = fsState.files[ackPath]

    -- Duplicate command across two topics must fail at protocol.create
    local okConflict, conflictErr = pcall(protocol.create, {
      directory = dir,
      fs = fs,
      topics = {
        field = { name = "field", collect = function() return nil end, commands = { dup = function() return {} end } },
        party = { name = "party", collect = function() return nil end, commands = { dup = function() return {} end } },
      },
    })
    assert(not okConflict)

    -- Overriding built-in ping must fail at protocol.create
    local okPing, pingConflictErr = pcall(protocol.create, {
      directory = dir,
      fs = fs,
      topics = {
        ui = { name = "ui", collect = function() return nil end, commands = { ping = function() return {} end } },
      },
    })
    assert(not okPing)

    return {
      okAck = okAck or json.null,
      errAck = errAck or json.null,
      throwAck = throwAck or json.null,
      conflictErr = tostring(conflictErr),
      pingConflictErr = tostring(pingConflictErr),
    }
  `);

  assert.ok(result.okAck);
  assert.deepEqual(JSON.parse(result.okAck) as BridgeAckDoc, {
    v: 1,
    id: "cmd-1",
    ok: true,
    result: { tag: "from-field", value: 42, tick: 1 },
    error: null,
  });

  assert.ok(result.errAck);
  assert.deepEqual(JSON.parse(result.errAck) as BridgeAckDoc, {
    v: 1,
    id: "cmd-2",
    ok: false,
    result: null,
    error: "模拟业务失败",
  });

  assert.ok(result.throwAck);
  const thrown = JSON.parse(result.throwAck) as BridgeAckDoc;
  assert.equal(thrown.id, "cmd-3");
  assert.equal(thrown.ok, false);
  assert.equal(thrown.result, null);
  assert.match(thrown.error ?? "", /boom inside command/u);

  assert.match(result.conflictErr ?? "", /命令名冲突.*dup/u);
  assert.match(result.pingConflictErr ?? "", /命令名冲突.*ping/u);
});

void test("command cleanup keeps replaced command B for next tick, logs uncollected ack overwrite, and removes matching commands normally", () => {
  const result = luaJson<{
    ackAfterA: string | null;
    commandAfterA: string | null;
    logsAfterA: string[];
    ackAfterB: string | null;
    commandAfterB: string | null;
    logsAfterB: string[];
    ackAfterC: string | null;
    commandAfterC: string | null;
    logsAfterC: string[];
  }>(`
    ${FAKE_FS_HELPER}
    ${FAKE_UE_HELPER}
    local json = require("ffrbridge.json")
    local protocol = require("ffrbridge.protocol")
    local fs, fsState = makeFakeFs(true)
    local dir = "/tmp/bridge-race"
    local cmdPath = dir .. "/command.json"
    local ackPath = dir .. "/ack.json"
    local logs = {}

    local function snapshotLogs()
      local copy = {}
      for index, line in ipairs(logs) do copy[index] = line end
      return json.array(copy)
    end

    local topics = {
      field = {
        name = "field",
        collect = function() return { ok = true } end,
        commands = {
          slow_op = function()
            -- Simulate AGP timing out A and replacing command.json with B while A is still running.
            fsState.files[cmdPath] = '{"v":1,"id":"cmd-B","op":"ping","args":{}}'
            return { handled = "A" }, nil
          end,
        },
      },
      battle = { name = "battle", collect = function() return nil end, commands = {} },
      ui = { name = "ui", collect = function() return nil end, commands = {} },
      party = { name = "party", collect = function() return nil end, commands = {} },
    }

    local bridge = protocol.create({
      directory = dir,
      session = "race01",
      fs = fs,
      topics = topics,
      findFirstOf = makeHealthyFindFirstOf(),
      clock = function() return 1.0 end,
      time = function() return 100 end,
      log = function(line) logs[#logs + 1] = line end,
    })

    -- Tick 1: process A; command.json is swapped to B mid-execution so B must stay untouched.
    fsState.files[cmdPath] = '{"v":1,"id":"cmd-A","op":"slow_op","args":{}}'
    bridge.tick()
    local ackAfterA = fsState.files[ackPath]
    local commandAfterA = fsState.files[cmdPath]
    local logsAfterA = snapshotLogs()

    -- Tick 2: A's ack is still on disk (uncollected); B runs, logs overwrite, writes B's ack, deletes B.
    bridge.tick()
    local ackAfterB = fsState.files[ackPath]
    local commandAfterB = fsState.files[cmdPath]
    local logsAfterB = snapshotLogs()

    -- Tick 3: normal command C after AGP removes B's ack; deletes command.json with no extra log.
    fsState.files[ackPath] = nil
    fsState.files[cmdPath] = '{"v":1,"id":"cmd-C","op":"ping","args":{}}'
    bridge.tick()
    local ackAfterC = fsState.files[ackPath]
    local commandAfterC = fsState.files[cmdPath]
    local logsAfterC = snapshotLogs()

    return {
      ackAfterA = ackAfterA or json.null,
      commandAfterA = commandAfterA or json.null,
      logsAfterA = logsAfterA,
      ackAfterB = ackAfterB or json.null,
      commandAfterB = commandAfterB or json.null,
      logsAfterB = logsAfterB,
      ackAfterC = ackAfterC or json.null,
      commandAfterC = commandAfterC or json.null,
      logsAfterC = logsAfterC,
    }
  `);

  assert.ok(result.ackAfterA);
  assert.deepEqual(JSON.parse(result.ackAfterA) as BridgeAckDoc, {
    v: 1,
    id: "cmd-A",
    ok: true,
    result: { handled: "A" },
    error: null,
  });
  assert.equal(result.commandAfterA, '{"v":1,"id":"cmd-B","op":"ping","args":{}}');
  assert.equal(result.logsAfterA.length, 0);

  assert.ok(result.ackAfterB);
  assert.deepEqual(JSON.parse(result.ackAfterB) as BridgeAckDoc, {
    v: 1,
    id: "cmd-B",
    ok: true,
    result: { pong: true, seq: 2 },
    error: null,
  });
  assert.equal(result.commandAfterB, null);
  assert.equal(result.logsAfterB.length, 1);
  assert.match(result.logsAfterB[0] ?? "", /\[FfrBridge\] ack_overwrite.*cmd-A.*cmd-B/u);

  assert.ok(result.ackAfterC);
  assert.deepEqual(JSON.parse(result.ackAfterC) as BridgeAckDoc, {
    v: 1,
    id: "cmd-C",
    ok: true,
    result: { pong: true, seq: 3 },
    error: null,
  });
  assert.equal(result.commandAfterC, null);
  assert.equal(result.logsAfterC.length, 1);
});

void test("command cleanup keeps command.json and rate-limits logs when verification read or parse fails", () => {
  const result = luaJson<{
    readFail1CommandLeft: boolean;
    readFail1Ack: string | null;
    readFail1LogCount: number;
    readFail2CommandLeft: boolean;
    readFail2LogCount: number;
    readFail3CommandLeft: boolean;
    readFail3LogCount: number;
    readHandleFailCommandLeft: boolean;
    readHandleFailLogCount: number;
    parseFail1Command: string | null;
    parseFail1Ack: string | null;
    parseFail1LogCount: number;
    parseFail2Command: string | null;
    parseFail2LogCount: number;
    parseFail3LogCount: number;
    alreadyDeletedCommandLeft: boolean;
    alreadyDeletedLogCount: number;
    logs: string[];
  }>(`
    ${FAKE_FS_HELPER}
    ${FAKE_UE_HELPER}
    local json = require("ffrbridge.json")
    local protocol = require("ffrbridge.protocol")
    local baseFs, fsState = makeFakeFs(true)
    local dir = "/tmp/bridge-verify-fail"
    local cmdPath = dir .. "/command.json"
    local ackPath = dir .. "/ack.json"
    local logs = {}
    local unixTime = 100
    local verifyMode = "ok"
    local cmdOpensInTick = 0

    local fs = {
      remove = baseFs.remove,
      rename = baseFs.rename,
      open = function(path, mode)
        if path == cmdPath and mode == "rb" then
          cmdOpensInTick = cmdOpensInTick + 1
          if cmdOpensInTick == 2 then
            if verifyMode == "open_err" then
              return nil, "sharing violation", 13
            elseif verifyMode == "read_nil" then
              return {
                read = function() return nil, "disk read error" end,
                close = function() return true end,
              }
            end
          end
        end
        return baseFs.open(path, mode)
      end,
    }

    local topics = {
      field = {
        name = "field",
        collect = function() return { ok = true } end,
        commands = {
          corrupt_cmd = function()
            fsState.files[cmdPath] = "{truncated"
            return { corrupted = true }, nil
          end,
          delete_cmd = function()
            -- Simulate AGP already deleting command.json before mod verifies.
            fsState.files[cmdPath] = nil
            return { removedByAgp = true }, nil
          end,
        },
      },
      battle = { name = "battle", collect = function() return nil end, commands = {} },
      ui = { name = "ui", collect = function() return nil end, commands = {} },
      party = { name = "party", collect = function() return nil end, commands = {} },
    }

    local bridge = protocol.create({
      directory = dir,
      session = "vfail1",
      fs = fs,
      topics = topics,
      findFirstOf = makeHealthyFindFirstOf(),
      clock = function() return 1.0 end,
      time = function() return unixTime end,
      log = function(line) logs[#logs + 1] = line end,
    })

    local function runCmd(cmdJson, mode)
      verifyMode = mode or "ok"
      cmdOpensInTick = 0
      fsState.files[ackPath] = nil
      fsState.files[cmdPath] = cmdJson
      bridge.tick()
    end

    -- 1. Open fails on re-read -> keep command.json and log error.
    unixTime = 100
    runCmd('{"v":1,"id":"rf-1","op":"ping","args":{}}', "open_err")
    local readFail1CommandLeft = fsState.files[cmdPath] ~= nil
    local readFail1Ack = fsState.files[ackPath]
    local readFail1LogCount = #logs

    -- 2. Second open failure within 60s -> still keep command.json, log suppressed.
    unixTime = 130
    runCmd('{"v":1,"id":"rf-2","op":"ping","args":{}}', "open_err")
    local readFail2CommandLeft = fsState.files[cmdPath] ~= nil
    local readFail2LogCount = #logs

    -- 3. Third open failure after 60s -> still keep command.json, log emitted again.
    unixTime = 165
    runCmd('{"v":1,"id":"rf-3","op":"ping","args":{}}', "open_err")
    local readFail3CommandLeft = fsState.files[cmdPath] ~= nil
    local readFail3LogCount = #logs

    -- 4. handle:read returns nil, err -> keep command.json and log after 60s.
    unixTime = 230
    runCmd('{"v":1,"id":"rf-4","op":"ping","args":{}}', "read_nil")
    local readHandleFailCommandLeft = fsState.files[cmdPath] ~= nil
    local readHandleFailLogCount = #logs

    -- 5. Re-read returns unparseable JSON -> keep command.json and log parse error (rate-limited 60s).
    unixTime = 300
    runCmd('{"v":1,"id":"pf-1","op":"corrupt_cmd","args":{}}', "ok")
    local parseFail1Command = fsState.files[cmdPath]
    local parseFail1Ack = fsState.files[ackPath]
    local parseFail1LogCount = #logs

    unixTime = 320
    runCmd('{"v":1,"id":"pf-2","op":"corrupt_cmd","args":{}}', "ok")
    local parseFail2Command = fsState.files[cmdPath]
    local parseFail2LogCount = #logs

    unixTime = 365
    runCmd('{"v":1,"id":"pf-3","op":"corrupt_cmd","args":{}}', "ok")
    local parseFail3LogCount = #logs

    -- 6. Already removed by AGP (ENOENT code 2) -> silent no-op, no extra error log.
    unixTime = 500
    runCmd('{"v":1,"id":"agp-del-1","op":"delete_cmd","args":{}}', "ok")
    local alreadyDeletedCommandLeft = fsState.files[cmdPath] ~= nil
    local alreadyDeletedLogCount = #logs

    return {
      readFail1CommandLeft = readFail1CommandLeft,
      readFail1Ack = readFail1Ack or json.null,
      readFail1LogCount = readFail1LogCount,
      readFail2CommandLeft = readFail2CommandLeft,
      readFail2LogCount = readFail2LogCount,
      readFail3CommandLeft = readFail3CommandLeft,
      readFail3LogCount = readFail3LogCount,
      readHandleFailCommandLeft = readHandleFailCommandLeft,
      readHandleFailLogCount = readHandleFailLogCount,
      parseFail1Command = parseFail1Command or json.null,
      parseFail1Ack = parseFail1Ack or json.null,
      parseFail1LogCount = parseFail1LogCount,
      parseFail2Command = parseFail2Command or json.null,
      parseFail2LogCount = parseFail2LogCount,
      parseFail3LogCount = parseFail3LogCount,
      alreadyDeletedCommandLeft = alreadyDeletedCommandLeft,
      alreadyDeletedLogCount = alreadyDeletedLogCount,
      logs = json.array(logs),
    }
  `);

  assert.equal(result.readFail1CommandLeft, true);
  assert.ok(result.readFail1Ack);
  assert.equal((JSON.parse(result.readFail1Ack) as BridgeAckDoc).id, "rf-1");
  assert.equal(result.readFail1LogCount, 1);

  assert.equal(result.readFail2CommandLeft, true);
  assert.equal(result.readFail2LogCount, 1);

  assert.equal(result.readFail3CommandLeft, true);
  assert.equal(result.readFail3LogCount, 2);

  assert.equal(result.readHandleFailCommandLeft, true);
  assert.equal(result.readHandleFailLogCount, 3);

  assert.equal(result.parseFail1Command, "{truncated");
  assert.ok(result.parseFail1Ack);
  assert.equal((JSON.parse(result.parseFail1Ack) as BridgeAckDoc).id, "pf-1");
  assert.equal(result.parseFail1LogCount, 4);

  assert.equal(result.parseFail2Command, "{truncated");
  assert.equal(result.parseFail2LogCount, 4);

  assert.equal(result.parseFail3LogCount, 5);

  assert.equal(result.alreadyDeletedCommandLeft, false);
  assert.equal(result.alreadyDeletedLogCount, 5);

  assert.match(result.logs[0] ?? "", /\[FfrBridge\] command_remove_read.*sharing violation/u);
  assert.match(result.logs[2] ?? "", /\[FfrBridge\] command_remove_read.*disk read error/u);
  assert.match(result.logs[3] ?? "", /\[FfrBridge\] command_remove_parse/u);
});
