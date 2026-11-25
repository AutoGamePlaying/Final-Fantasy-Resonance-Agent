import assert from "node:assert/strict";
import { join } from "node:path";
import test from "node:test";
import { lua, lauxlib, lualib, to_jsstring, to_luastring } from "fengari";

const SCRIPTS = join(import.meta.dirname, "..", "..", "mod", "FfrBridge", "Scripts");

function loadLua(state: ReturnType<typeof lauxlib.luaL_newstate>, code: string): void {
  const status = lauxlib.luaL_loadbuffer(
    state,
    to_luastring(code),
    null,
    to_luastring("bridge-input-test"),
  );
  if (status !== lua.LUA_OK) {
    const message = lua.lua_tostring(state, -1);
    throw new Error(message ? to_jsstring(message) : "Lua 编译失败");
  }
}

// eslint-disable-next-line @typescript-eslint/no-unnecessary-type-parameters
function luaJson<T>(body: string): T {
  const state = lauxlib.luaL_newstate();
  try {
    lualib.luaL_openlibs(state);
    const prelude = `package.path = ${JSON.stringify(join(SCRIPTS, "?.lua"))} .. ";" .. package.path\n`;
    const code = `${prelude}result = require("ffrbridge.json").encode((function()\n${body}\nend)())`;
    loadLua(state, code);
    const status = lua.lua_pcall(state, 0, lua.LUA_MULTRET, 0);
    if (status !== lua.LUA_OK) {
      const message = lua.lua_tostring(state, -1);
      throw new Error(message ? to_jsstring(message) : "Lua 执行失败");
    }
    lua.lua_getglobal(state, to_luastring("result"));
    const raw = lua.lua_tostring(state, -1);
    if (!raw) throw new Error("Lua 没有返回 JSON 字符串");
    return JSON.parse(to_jsstring(raw)) as T;
  } finally {
    lua.lua_close(state);
  }
}

const RIG_LUA = `
  local json = require("ffrbridge.json")
  local protocol = require("ffrbridge.protocol")
  local input = require("ffrbridge.topics.input")

  local function makeFakeFs(dirExists)
    local files = {}
    local state = { dirExists = dirExists ~= false, files = files }
    local fs = {}
    function fs.open(path, mode)
      if mode == "wb" then
        local handle = { parts = {} }
        function handle:write(text)
          self.parts[#self.parts + 1] = text
          return self
        end
        function handle:close()
          files[path] = table.concat(self.parts)
          return true
        end
        return handle
      end
      local body = files[path]
      if body == nil then return nil, "missing" end
      local handle = {}
      function handle:read() return body end
      function handle:close() return true end
      return handle
    end
    function fs.remove(path)
      if files[path] == nil then return nil, "missing", 2 end
      files[path] = nil
      return true
    end
    function fs.rename(from, to)
      if from == to then
        if state.dirExists then return true end
        return nil, "missing", 2
      end
      if files[from] == nil then return nil, "missing", 2 end
      files[to] = files[from]
      files[from] = nil
      return true
    end
    return fs, state
  end

  local function makeClass(fullName)
    return {
      IsValid = function() return true end,
      GetFullName = function() return fullName end,
    }
  end

  local function makeObj(classFull, extras)
    local cls = makeClass(classFull)
    local obj = {
      IsValid = function() return true end,
      GetClass = function() return cls end,
    }
    if extras then
      for k, v in pairs(extras) do obj[k] = v end
    end
    return obj
  end

  local function makeInputRig(opts)
    local cfg = opts or {}
    local fs, fsState = makeFakeFs(true)
    local dir = "/tmp/ffr-input-bridge"
    if cfg.enabled then
      fsState.files[dir .. "/input.enabled"] = "enabled\\n"
    end

    local nowSec = cfg.startSec or 10.0
    local logs = {}
    local calls = {}
    local timers = {}
    local uiState = {
      screen_kind = cfg.screenKind or "system_menu",
      cursor = cfg.cursor ~= nil and cfg.cursor or 0,
      blocked = cfg.blocked or {},
    }

    local actionObjects = {}
    local function getOrCreateAction(path)
      if not actionObjects[path] then
        actionObjects[path] = makeObj("Class /Script/EnhancedInput.InputAction", {
          assetPath = path,
          GetFullName = function() return "InputAction " .. path end,
        })
      end
      return actionObjects[path]
    end

    local inputMgr = makeObj("Class /Script/mucha.CPP_InputManager", {
      GetInputAction = function(_, mappingType, actionName)
        if cfg.disableGetInputAction then return nil end
        for _, spec in pairs(input.ACTIONS) do
          if spec.mappingType == mappingType and spec.actionName == actionName then
            return getOrCreateAction(spec.assetPath)
          end
          if spec.fieldOverride
            and spec.fieldOverride.mappingType == mappingType
            and spec.fieldOverride.actionName == actionName
          then
            return getOrCreateAction(spec.fieldOverride.assetPath)
          end
        end
        return nil
      end,
    })

    local subsys = makeObj("Class /Script/EnhancedInput.EnhancedInputLocalPlayerSubsystem", {})
    if not cfg.noActionInject then
      subsys.InjectInputForAction = function(_, actionObj, rawValue, modifiers, triggers)
        calls[#calls + 1] = {
          method = "InjectInputForAction",
          assetPath = actionObj and actionObj.assetPath or nil,
          rawValue = rawValue,
          modifierCount = modifiers and #modifiers or 0,
          triggerCount = triggers and #triggers or 0,
        }
        if cfg.onInject then cfg.onInject(uiState, calls[#calls]) end
      end
    end
    if not cfg.noVectorInject then
      subsys.InjectInputVectorForAction = function(_, actionObj, vec, modifiers, triggers)
        calls[#calls + 1] = {
          method = "InjectInputVectorForAction",
          assetPath = actionObj and actionObj.assetPath or nil,
          vector = vec,
          modifierCount = modifiers and #modifiers or 0,
          triggerCount = triggers and #triggers or 0,
        }
        if cfg.onInject then cfg.onInject(uiState, calls[#calls]) end
      end
    end
    subsys.InjectInputForPlayerMapping = function(_, mappingName, rawValue, modifiers, triggers)
      calls[#calls + 1] = {
        method = "InjectInputForPlayerMapping",
        mappingName = mappingName,
        rawValue = rawValue,
        modifierCount = modifiers and #modifiers or 0,
        triggerCount = triggers and #triggers or 0,
      }
      if cfg.onInject then cfg.onInject(uiState, calls[#calls]) end
    end
    subsys.InjectInputVectorForPlayerMapping = function(_, mappingName, vec, modifiers, triggers)
      calls[#calls + 1] = {
        method = "InjectInputVectorForPlayerMapping",
        mappingName = mappingName,
        vector = vec,
        modifierCount = modifiers and #modifiers or 0,
        triggerCount = triggers and #triggers or 0,
      }
      if cfg.onInject then cfg.onInject(uiState, calls[#calls]) end
    end
    if not cfg.noContinuous then
      subsys.StartContinuousInputInjectionForAction = function(_, actionObj, rawValue, modifiers, triggers)
        calls[#calls + 1] = {
          method = "StartContinuousInputInjectionForAction",
          assetPath = actionObj and actionObj.assetPath or nil,
          rawValue = rawValue,
          modifierCount = modifiers and #modifiers or 0,
          triggerCount = triggers and #triggers or 0,
        }
        if cfg.onInject then cfg.onInject(uiState, calls[#calls]) end
      end
      subsys.StopContinuousInputInjectionForAction = function(_, actionObj)
        calls[#calls + 1] = {
          method = "StopContinuousInputInjectionForAction",
          assetPath = actionObj and actionObj.assetPath or nil,
        }
      end
    end

    local eiLib = makeObj("Class /Script/EnhancedInput.EnhancedInputLibrary", {
      MakeInputActionValueOfType = function(_, x, y, z, valueType)
        return { x = x, y = y, z = z, valueType = valueType }
      end,
    })

    local triggerCls = makeObj("Class /Script/CoreUObject.Class", {})
    local statics = makeObj("Class /Script/Engine.GameplayStatics", {
      SpawnObject = function(_, cls, outer)
        if cls == triggerCls and outer == subsys then
          return makeObj("Class /Script/EnhancedInput.InputTriggerPressed", {})
        end
        return nil
      end,
    })

    local world = {
      IsValid = function() return true end,
      GetFullName = function() return "World /Game/Maps/Dng_01Gra_40_PL.Dng_01Gra_40_PL" end,
    }
    local pawn = makeObj("BlueprintGeneratedClass /Game/BP_MapUnit_Player.BP_MapUnit_Player_C", {
      K2_GetActorLocation = function() return { X = 100.0, Y = 200.0, Z = 0.0 } end,
    })
    local pc = makeObj("Class /Script/mucha.CPP_MuchaPlayerController", { Pawn = pawn })
    local gm = makeObj("Class /Script/mucha.CPP_MuchaGameModeBase")
    local gi = makeObj("Class /Script/mucha.CPP_MuchaGameInstance")

    local vmCmd = makeObj("Class /Script/mucha.VM_BtlCmd_C", {
      FocusCmdData = { EntryIndex = cfg.battleCursor or 0 },
      UnitSelectVM = makeObj("Class /Script/mucha.VM_BtlCmdUnitSelect_C", {
        FocusBaseUniqueId = cfg.battleTargetUid or 2,
      }),
    })
    local uiBattle = makeObj("Class /Script/mucha.CPP_UI_BattleSubsystem", {
      BtlCmdManager = makeObj("Class /Script/mucha.CPP_UI_BtlCmdManager", {
        CmdState = cfg.battleCmdState or 0,
      }),
    })

    local byClass = {
      World = world,
      PlayerController = pc,
      GameModeBase = gm,
      GameInstance = gi,
      CPP_InputManager = inputMgr,
      VM_BtlCmd_C = vmCmd,
      CPP_UI_BattleSubsystem = uiBattle,
    }
    if not cfg.noSubsys then
      byClass.EnhancedInputLocalPlayerSubsystem = subsys
    end

    local uiTopic = {
      name = "ui",
      collect = function()
        local out = {
          screen_kind = uiState.screen_kind,
          input_blocked = json.array(uiState.blocked),
          menu = json.null,
          choice = json.null,
          shop = json.null,
          save_ui = json.null,
          tutorial = json.null,
          battle_result = json.null,
        }
        if uiState.screen_kind == "system_menu" or uiState.screen_kind == "main_menu" then
          out.menu = { kind = uiState.screen_kind, focus_index = uiState.cursor }
        elseif uiState.screen_kind == "dialog" or uiState.screen_kind == "choice" then
          out.choice = { source = "dialog", focus_index = uiState.cursor }
        elseif uiState.screen_kind == "shop" then
          out.shop = { scene_stack = { "Command", "Shop" }, item_focus_index = uiState.cursor }
        end
        return out
      end,
      commands = {},
    }

    local scheduleFn = nil
    if cfg.useScheduler then
      scheduleFn = function(delayMs, fn)
        timers[#timers + 1] = { delayMs = delayMs, fn = fn }
        return true
      end
    end

    local bridge = protocol.create({
      directory = dir,
      session = "inpt01",
      fs = fs,
      topics = { ui = uiTopic },
      findFirstOf = function(name) return byClass[name] end,
      staticFindObject = function(path)
        if path == "/Script/EnhancedInput.Default__EnhancedInputLibrary" then
          if cfg.noEiLib then return nil end
          return eiLib
        end
        if path == "/Script/EnhancedInput.InputTriggerPressed" then
          return triggerCls
        end
        if path == "/Script/Engine.Default__GameplayStatics" then
          return statics
        end
        if cfg.disableStaticFindAction then return nil end
        if type(path) == "string" and path:find("/Game/Input/", 1, true) then
          return getOrCreateAction(path)
        end
        return nil
      end,
      clock = function() return nowSec end,
      time = function() return 1000 end,
      log = function(line) logs[#logs + 1] = line end,
      scheduleDelayed = scheduleFn,
    })

    local cmdSeq = 0
    local function sendPress(args)
      cmdSeq = cmdSeq + 1
      local id = "cmd-" .. tostring(cmdSeq)
      fsState.files[dir .. "/ack.json"] = nil
      fsState.files[dir .. "/command.json"] = json.encode({
        v = 1,
        id = id,
        op = "press",
        args = args,
      })
      bridge.tick()
      local ackRaw = fsState.files[dir .. "/ack.json"]
      return ackRaw and json.decode(ackRaw) or nil
    end

    local function clearHistory()
      for i = #timers, 1, -1 do timers[i] = nil end
      for i = #calls, 1, -1 do calls[i] = nil end
    end

    return {
      dir = dir,
      fsState = fsState,
      bridge = bridge,
      logs = logs,
      calls = calls,
      timers = timers,
      uiState = uiState,
      vmCmd = vmCmd,
      uiBattle = uiBattle,
      setClock = function(sec) nowSec = sec end,
      clearHistory = clearHistory,
      sendPress = sendPress,
    }
  end
`;

interface PressAck {
  v: number;
  id: string;
  ok: boolean;
  result: {
    action: string;
    times: number;
    hold_ms: number;
    x?: number;
    y?: number;
    ms?: number;
    injected: {
      method: string;
      input_action: string;
      asset_path: string;
      action_name: string;
      mapping_type: string;
      mapping_name: string;
      resolved_via: string;
      vector: { x: number; y: number; z: number };
      continuous_stop_scheduled: boolean;
    };
    before: {
      screen_kind: string | null;
      cursor: number | null;
      cursor_source: string | null;
      input_blocked: string[];
    };
    after: {
      screen_kind: string | null;
      cursor: number | null;
      cursor_source: string | null;
      input_blocked: string[];
    };
    window_deactivated: boolean;
  } | null;
  error: string | null;
}

void test("mapping table covers all required semantic actions with complete verified metadata", () => {
  interface MappingCheck {
    whitelist: string[];
    entries: Record<
      string,
      {
        kind: string;
        mappingType: number;
        mappingTypeName: string;
        actionName: string;
        assetPath: string;
        mappingName: string;
        valueType: number;
        hasVector: boolean;
        hasFieldOverride: boolean;
        evidence: string;
      }
    >;
    registeredCommands: string[];
    stateHasInputKey: boolean;
  }

  const out = luaJson<MappingCheck>(`
    ${RIG_LUA}
    local entries = {}
    for name, spec in pairs(input.ACTIONS) do
      entries[name] = {
        kind = spec.kind,
        mappingType = spec.mappingType,
        mappingTypeName = spec.mappingTypeName,
        actionName = spec.actionName,
        assetPath = spec.assetPath,
        mappingName = spec.mappingName,
        valueType = spec.valueType,
        hasVector = type(spec.vector) == "table",
        hasFieldOverride = type(spec.fieldOverride) == "table",
        evidence = spec.evidence,
      }
    end
    local rig = makeInputRig({ enabled = false })
    rig.bridge.tick()
    local stateDoc = json.decode(rig.fsState.files[rig.dir .. "/state.json"])
    return {
      whitelist = json.array(input.WHITELIST),
      entries = entries,
      registeredCommands = json.array(rig.bridge.commands),
      stateHasInputKey = stateDoc.data.input ~= nil,
    }
  `);

  const required = [
    "confirm",
    "cancel",
    "up",
    "down",
    "left",
    "right",
    "page_prev",
    "page_next",
    "menu",
    "system_menu",
    "map",
    "quick_heal",
    "interact",
    "move",
  ];
  for (const name of required) {
    assert.ok(out.whitelist.includes(name), `whitelist missing ${name}`);
    const entry = out.entries[name];
    assert.ok(entry, `ACTIONS missing ${name}`);
    assert.equal(typeof entry.mappingType, "number");
    assert.ok(entry.mappingType > 0, `${name} mappingType`);
    assert.ok(entry.mappingTypeName.length > 0, `${name} mappingTypeName`);
    assert.ok(entry.actionName.length > 0, `${name} actionName`);
    assert.match(entry.assetPath, /^\/Game\/Input\/.*IA_[A-Za-z0-9_]+\.IA_[A-Za-z0-9_]+$/u);
    assert.ok(entry.mappingName.length > 0, `${name} mappingName`);
    assert.ok(entry.evidence.length > 0, `${name} evidence`);
    if (name === "move") {
      assert.equal(entry.kind, "move");
      assert.equal(entry.hasVector, false);
    } else {
      assert.equal(entry.kind, "discrete");
      assert.equal(entry.hasVector, true);
    }
  }
  assert.equal(out.entries.confirm?.hasFieldOverride, true);
  assert.ok(out.registeredCommands.includes("press"));
  assert.equal(out.stateHasInputKey, false, "state.json must not add a data.input topic");
});

void test("input.enabled switch file defaults off, unlocks when created, and locks again when removed", () => {
  const out = luaJson<{
    offAck: PressAck;
    offCalls: number;
    onAck: PressAck;
    onCalls: number;
    offAgainAck: PressAck;
    offAgainCalls: number;
    logs: string[];
  }>(`
    ${RIG_LUA}
    local rig = makeInputRig({ enabled = false, screenKind = "system_menu", cursor = 0 })

    -- 1. Switch absent -> rejected with "输入未开启", zero injections
    local offAck = rig.sendPress({ action = "confirm" })
    local offCalls = #rig.calls

    -- 2. Create input.enabled -> allowed
    rig.fsState.files[rig.dir .. "/input.enabled"] = "enabled\\n"
    rig.setClock(10.1)
    local onAck = rig.sendPress({ action = "confirm" })
    local onCalls = #rig.calls

    -- 3. Remove input.enabled -> rejected again
    rig.fsState.files[rig.dir .. "/input.enabled"] = nil
    rig.setClock(10.5)
    local offAgainAck = rig.sendPress({ action = "confirm" })
    local offAgainCalls = #rig.calls

    return {
      offAck = offAck,
      offCalls = offCalls,
      onAck = onAck,
      onCalls = onCalls,
      offAgainAck = offAgainAck,
      offAgainCalls = offAgainCalls,
      logs = json.array(rig.logs),
    }
  `);

  assert.equal(out.offAck.ok, false);
  assert.equal(out.offAck.result, null);
  assert.match(out.offAck.error ?? "", /输入未开启/u);
  assert.equal(out.offCalls, 0);

  assert.equal(out.onAck.ok, true);
  assert.equal(out.onCalls, 1);

  assert.equal(out.offAgainAck.ok, false);
  assert.match(out.offAgainAck.error ?? "", /输入未开启/u);
  assert.equal(out.offAgainCalls, 1);

  assert.equal(out.logs.length, 3);
  assert.match(out.logs[0] ?? "", /input rejected reason=disabled/u);
  assert.match(out.logs[1] ?? "", /input ok action=confirm/u);
});

void test("whitelist rejects unknown actions and invalid times/hold_ms/move parameters", () => {
  const out = luaJson<{
    unknownAction: PressAck;
    emptyAction: PressAck;
    badTimesZero: PressAck;
    badTimesTooLarge: PressAck;
    badHoldNeg: PressAck;
    badMoveZero: PressAck;
    badMoveRange: PressAck;
    badMoveMs: PressAck;
    totalInjectCalls: number;
  }>(`
    ${RIG_LUA}
    local rig = makeInputRig({ enabled = true })
    return {
      unknownAction = rig.sendPress({ action = "teleport_boss" }),
      emptyAction = rig.sendPress({ action = "" }),
      badTimesZero = rig.sendPress({ action = "down", times = 0 }),
      badTimesTooLarge = rig.sendPress({ action = "down", times = 99 }),
      badHoldNeg = rig.sendPress({ action = "down", hold_ms = -5 }),
      badMoveZero = rig.sendPress({ action = "move", x = 0, y = 0, ms = 200 }),
      badMoveRange = rig.sendPress({ action = "move", x = 1.5, y = 0, ms = 200 }),
      badMoveMs = rig.sendPress({ action = "move", x = 0, y = 1, ms = 9999 }),
      totalInjectCalls = #rig.calls,
    }
  `);

  assert.equal(out.unknownAction.ok, false);
  assert.match(out.unknownAction.error ?? "", /白名单/u);
  assert.equal(out.emptyAction.ok, false);
  assert.equal(out.badTimesZero.ok, false);
  assert.equal(out.badTimesTooLarge.ok, false);
  assert.equal(out.badHoldNeg.ok, false);
  assert.equal(out.badMoveZero.ok, false);
  assert.equal(out.badMoveRange.ok, false);
  assert.equal(out.badMoveMs.ok, false);
  assert.equal(out.totalInjectCalls, 0);
});

void test("minimum 50ms interval blocks rapid back-to-back presses and allows after cooldown", () => {
  const out = luaJson<{
    first: PressAck;
    tooFast0: PressAck;
    tooFast49: PressAck;
    after50: PressAck;
    totalInjectCalls: number;
  }>(`
    ${RIG_LUA}
    local rig = makeInputRig({ enabled = true, startSec = 20.0 })
    local first = rig.sendPress({ action = "down" })
    -- Same tick (0 ms elapsed)
    local tooFast0 = rig.sendPress({ action = "down" })
    -- 49 ms later (still < 50 ms)
    rig.setClock(20.049)
    local tooFast49 = rig.sendPress({ action = "down" })
    -- 50 ms later (>= 50 ms) -> succeeds
    rig.setClock(20.050)
    local after50 = rig.sendPress({ action = "down" })
    return {
      first = first,
      tooFast0 = tooFast0,
      tooFast49 = tooFast49,
      after50 = after50,
      totalInjectCalls = #rig.calls,
    }
  `);

  assert.equal(out.first.ok, true);
  assert.equal(out.tooFast0.ok, false);
  assert.match(out.tooFast0.error ?? "", /过于频繁/u);
  assert.equal(out.tooFast49.ok, false);
  assert.match(out.tooFast49.error ?? "", /最小间隔 50 毫秒/u);
  assert.equal(out.after50.ok, true);
  assert.equal(out.totalInjectCalls, 2);
});

void test("press records before/after screen_kind & cursor, field confirm switches to UnitInteract, and logs every press", () => {
  interface InjectCallRecord {
    method: string;
    assetPath?: string;
    rawValue?: { x: number; y: number; z: number; valueType: number };
    triggerCount?: number;
  }
  const out = luaJson<{
    menuDown: PressAck;
    fieldConfirm: PressAck;
    battleTarget: PressAck;
    calls: InjectCallRecord[];
    logs: string[];
  }>(`
    ${RIG_LUA}
    local rig = makeInputRig({
      enabled = true,
      screenKind = "system_menu",
      cursor = 0,
      blocked = { "WindowDeactivate" },
      onInject = function(uiState, call)
        if call.assetPath and call.assetPath:find("IA_UI_Common_Direction", 1, true) then
          if uiState.cursor ~= nil then
            uiState.cursor = uiState.cursor + 1
          end
        elseif call.assetPath and call.assetPath:find("IA_Map_UnitInteract", 1, true) then
          uiState.screen_kind = "talk"
          uiState.cursor = nil
        end
      end,
    })

    -- 1. Menu down moves cursor 0 -> 1 and flags WindowDeactivate
    local menuDown = rig.sendPress({ action = "down" })

    -- 2. In field, confirm switches to IA_Map_UnitInteract
    rig.uiState.screen_kind = "field"
    rig.uiState.cursor = nil
    rig.uiState.blocked = {}
    rig.setClock(11.0)
    local fieldConfirm = rig.sendPress({ action = "confirm" })

    -- 3. In battle target mode, cursor reads battle.cmd.target.focus_uid
    rig.uiState.screen_kind = "battle"
    rig.uiBattle.BtlCmdManager.CmdState = 1
    rig.vmCmd.UnitSelectVM.FocusBaseUniqueId = 2
    rig.setClock(12.0)
    local battleTarget = rig.sendPress({ action = "right" })

    return {
      menuDown = menuDown,
      fieldConfirm = fieldConfirm,
      battleTarget = battleTarget,
      calls = json.array(rig.calls),
      logs = json.array(rig.logs),
    }
  `);

  assert.equal(out.menuDown.ok, true);
  assert.ok(out.menuDown.result);
  assert.equal(out.menuDown.result.injected.method, "InjectInputForAction");
  assert.equal(out.menuDown.result.injected.input_action, "IA_UI_Common_Direction");
  assert.deepEqual(out.menuDown.result.before, {
    screen_kind: "system_menu",
    cursor: 0,
    cursor_source: "ui.menu.focus_index",
    input_blocked: ["WindowDeactivate"],
  });
  assert.deepEqual(out.menuDown.result.after, {
    screen_kind: "system_menu",
    cursor: 1,
    cursor_source: "ui.menu.focus_index",
    input_blocked: ["WindowDeactivate"],
  });
  assert.equal(out.menuDown.result.window_deactivated, true);
  assert.ok(out.calls[0]);
  assert.equal(out.calls[0].triggerCount, 1);
  assert.deepEqual(out.calls[0].rawValue, { x: 0, y: -1, z: 0, valueType: 2 });

  assert.equal(out.fieldConfirm.ok, true);
  assert.ok(out.fieldConfirm.result);
  assert.equal(out.fieldConfirm.result.injected.input_action, "IA_Map_UnitInteract");
  assert.equal(out.fieldConfirm.result.injected.mapping_type, "MapUnit");
  assert.equal(out.fieldConfirm.result.before.screen_kind, "field");
  assert.equal(out.fieldConfirm.result.after.screen_kind, "talk");
  assert.equal(out.fieldConfirm.result.window_deactivated, false);

  assert.equal(out.battleTarget.ok, true);
  assert.ok(out.battleTarget.result);
  assert.equal(out.battleTarget.result.before.screen_kind, "battle");
  assert.equal(out.battleTarget.result.before.cursor, 2);
  assert.equal(out.battleTarget.result.before.cursor_source, "battle.cmd.target.focus_uid");

  assert.equal(out.logs.length, 3);
  assert.match(out.logs[0] ?? "", /blocked=WindowDeactivate/u);
  assert.match(out.logs[1] ?? "", /before=field:null after=talk:null/u);
});

void test("continuous move starts injection, schedules delayed stop, and cancels stale continuous input on new action", () => {
  interface CallEntry {
    method: string;
    assetPath?: string;
    rawValue?: { x: number; y: number; z: number; valueType: number };
  }
  const out = luaJson<{
    moveAck: PressAck;
    timerCount: number;
    timerDelay: number;
    callsBeforeTimer: CallEntry[];
    callsAfterTimer: CallEntry[];
    interruptMethods: string[];
    repeatAck: PressAck;
    repeatTimerDelays: number[];
    callsAfterRepeat: number;
  }>(`
    ${RIG_LUA}
    local rig = makeInputRig({
      enabled = true,
      screenKind = "field",
      useScheduler = true,
    })

    local moveAck = rig.sendPress({ action = "move", x = 0, y = 1, ms = 350 })
    local timerCount = #rig.timers
    local timerDelay = rig.timers[1] and rig.timers[1].delayMs or -1
    local callsBeforeTimer = {}
    for i = 1, #rig.calls do callsBeforeTimer[i] = rig.calls[i] end

    -- Fire delayed stop callback
    rig.timers[1].fn()
    local callsAfterTimer = {}
    for i = 1, #rig.calls do callsAfterTimer[i] = rig.calls[i] end

    -- Start a second move and interrupt it with a discrete action before timer fires:
    -- old continuous input must be stopped first, and stale timer becomes a no-op.
    rig.clearHistory()
    rig.setClock(10.5)
    rig.sendPress({ action = "move", x = 1, y = 0, ms = 400 })
    local staleTimer = rig.timers[1]
    rig.setClock(10.6)
    rig.sendPress({ action = "cancel" })
    staleTimer.fn()
    local interruptMethods = {}
    for i = 1, #rig.calls do interruptMethods[i] = rig.calls[i].method end

    -- Multi-press times=3 schedules 2 extra presses spaced by 50ms
    rig.clearHistory()
    rig.setClock(11.0)
    local repeatAck = rig.sendPress({ action = "down", times = 3 })
    local repeatTimerDelays = {}
    for i = 1, #rig.timers do
      repeatTimerDelays[i] = rig.timers[i].delayMs
      rig.timers[i].fn()
    end

    return {
      moveAck = moveAck,
      timerCount = timerCount,
      timerDelay = timerDelay,
      callsBeforeTimer = json.array(callsBeforeTimer),
      callsAfterTimer = json.array(callsAfterTimer),
      interruptMethods = json.array(interruptMethods),
      repeatAck = repeatAck,
      repeatTimerDelays = json.array(repeatTimerDelays),
      callsAfterRepeat = #rig.calls,
    }
  `);

  assert.equal(out.moveAck.ok, true);
  assert.ok(out.moveAck.result);
  assert.equal(out.moveAck.result.action, "move");
  assert.equal(out.moveAck.result.ms, 350);
  assert.equal(out.moveAck.result.injected.method, "StartContinuousInputInjectionForAction");
  assert.equal(out.moveAck.result.injected.input_action, "IA_Map_UnitMove");
  assert.equal(out.moveAck.result.injected.continuous_stop_scheduled, true);
  assert.equal(out.timerCount, 1);
  assert.equal(out.timerDelay, 350);
  assert.equal(out.callsBeforeTimer.length, 1);
  assert.ok(out.callsBeforeTimer[0]);
  assert.equal(out.callsBeforeTimer[0].method, "StartContinuousInputInjectionForAction");
  assert.deepEqual(out.callsBeforeTimer[0].rawValue, { x: 0, y: 1, z: 0, valueType: 2 });
  assert.equal(out.callsAfterTimer.length, 2);
  assert.equal(out.callsAfterTimer[1]?.method, "StopContinuousInputInjectionForAction");
  assert.deepEqual(out.interruptMethods, [
    "StartContinuousInputInjectionForAction",
    "StopContinuousInputInjectionForAction",
    "InjectInputForAction",
  ]);

  assert.equal(out.repeatAck.ok, true);
  assert.ok(out.repeatAck.result);
  assert.equal(out.repeatAck.result.times, 3);
  assert.deepEqual(out.repeatTimerDelays, [50, 100]);
  assert.equal(out.callsAfterRepeat, 3);
});

void test("falls back to InjectInputVectorForAction and InjectInputForPlayerMapping when primary helpers are absent", () => {
  const out = luaJson<{
    vectorFallback: PressAck;
    mappingFallback: PressAck;
    noSubsys: PressAck;
  }>(`
    ${RIG_LUA}
    -- 1. EnhancedInputLibrary unavailable -> falls back to InjectInputVectorForAction
    local rig1 = makeInputRig({ enabled = true, noEiLib = true })
    local vectorFallback = rig1.sendPress({ action = "page_next" })

    -- 2. InputAction object unresolvable -> falls back to InjectInputForPlayerMapping
    local rig2 = makeInputRig({
      enabled = true,
      disableGetInputAction = true,
      disableStaticFindAction = true,
    })
    local mappingFallback = rig2.sendPress({ action = "map" })

    -- 3. Missing subsystem -> clean Chinese error
    local rig3 = makeInputRig({ enabled = true, noSubsys = true })
    local noSubsys = rig3.sendPress({ action = "confirm" })

    return {
      vectorFallback = vectorFallback,
      mappingFallback = mappingFallback,
      noSubsys = noSubsys,
    }
  `);

  assert.equal(out.vectorFallback.ok, true);
  assert.ok(out.vectorFallback.result);
  assert.equal(out.vectorFallback.result.injected.method, "InjectInputVectorForAction");
  assert.deepEqual(out.vectorFallback.result.injected.vector, { x: 1, y: 0, z: 0 });

  assert.equal(out.mappingFallback.ok, true);
  assert.ok(out.mappingFallback.result);
  assert.equal(out.mappingFallback.result.injected.method, "InjectInputForPlayerMapping");
  assert.equal(out.mappingFallback.result.injected.mapping_name, "Map_OpenMainMenu_Map");
  assert.equal(out.mappingFallback.result.injected.resolved_via, "PlayerMapping");

  assert.equal(out.noSubsys.ok, false);
  assert.match(out.noSubsys.error ?? "", /EnhancedInputLocalPlayerSubsystem/u);
});
