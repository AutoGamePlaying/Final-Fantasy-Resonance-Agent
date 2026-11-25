import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import test from "node:test";
import { lua, lauxlib, lualib, to_jsstring, to_luastring } from "fengari";

const SCRIPTS = join(import.meta.dirname, "..", "..", "mod", "FfrBridge", "Scripts");

function load(state: ReturnType<typeof lauxlib.luaL_newstate>, code: string): void {
  const status = lauxlib.luaL_loadbuffer(
    state,
    to_luastring(code),
    null,
    to_luastring("bridge-party-test"),
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

const FAKE_PARTY_HELPER = `
  local function makeTArray(items)
    local arr = {}
    for i = 1, #items do arr[i] = items[i] end
    function arr:GetArrayNum() return #self end
    return arr
  end

  local function makeTMap(entries)
    local map = { _entries = entries }
    function map:ForEach(cb)
      for i = 1, #self._entries do
        local pair = self._entries[i]
        local k = { get = function() return pair[1] end }
        local v = { get = function() return pair[2] end }
        if cb(k, v) == true then break end
      end
    end
    return map
  end

  local function makeUObject(props)
    local obj = { _valid = true }
    function obj:IsValid() return self._valid end
    if props then
      for k, v in pairs(props) do obj[k] = v end
    end
    return obj
  end

  local function makePoint(cur, maxVal)
    return makeUObject({ CurrentPoint = cur, MaxPoint = maxVal })
  end

  local function makeUnitVm(spec)
    local mapEntries = {}
    if spec.stats then
      for k, v in pairs(spec.stats) do
        mapEntries[#mapEntries + 1] = { k, makePoint(v[1], v[2] or v[1]) }
      end
    end
    local stageVm = spec.stageLevel ~= nil and makePoint(spec.stageLevel, 99) or nil
    local levelProgVm = makeUObject({
      StageVM = stageVm,
      GetLevel = spec.progGetLevel and function() return spec.progGetLevel end or nil,
    })
    return makeUObject({
      HitPointVM = spec.hp and makePoint(spec.hp[1], spec.hp[2]) or nil,
      MagicPointVM = spec.mp and makePoint(spec.mp[1], spec.mp[2]) or nil,
      LevelProgressionVM = levelProgVm,
      GetLevel = spec.vmGetLevel and function() return spec.vmGetLevel end or nil,
      ParamPointVMMap = makeTMap(mapEntries),
    })
  end

  local function makeFlags(trueIds, length)
    local total = length or 2048
    local flags = {}
    for i = 1, total do flags[i] = false end
    for i = 1, #trueIds do
      flags[trueIds[i] + 1] = true
    end
    return makeTArray(flags)
  end

  local function makeHealthyFixture()
    local progressArr = {}
    for i = 1, 200 do progressArr[i] = 0 end
    progressArr[1] = 1040

    local saveData = makeUObject({
      Player = {
        currentMoney = 2080,
        PlayTime = 3525.04,
      },
      Party = {
        partyList = makeTArray({ 1001, 1002 }),
        guestUnitId = -1,
      },
      unitParameterList = makeTArray({
        {
          UnitId = 1001,
          Experience = 473,
          HitPoint = 382, -- stale in-battle save value
          MagicPoint = 43,
          EquipWeaponId = 10100,
          EquipArmorId = 11000,
          EquipAccessoryIdList = makeTArray({ 12000 }),
          equipVisionId = -1,
          JoinStatus = 0,
        },
        {
          UnitId = 1002,
          Experience = 473,
          HitPoint = 336,
          MagicPoint = 50,
          EquipWeaponId = 10200,
          EquipArmorId = 11000,
          EquipAccessoryIdList = makeTArray({ 12000, 12001 }),
          equipVisionId = 5001,
          JoinStatus = 0,
        },
        {
          UnitId = 1006,
          Experience = 24,
          HitPoint = 272,
          MagicPoint = 40,
          EquipWeaponId = 10600,
          EquipArmorId = -1,
          EquipAccessoryIdList = makeTArray({}),
          equipVisionId = -1,
          JoinStatus = 2,
        },
      }),
      inventory = makeTArray({
        { ItemId = 1, Num = 6 },
        { ItemId = 3, Num = 3 },
        { ItemId = 4, Num = 4 },
      }),
      Progress = {
        Progress = makeTArray(progressArr),
        eventFlag = makeFlags({ 14, 15, 22, 25, 105, 1160, 2047 }),
      },
    })

    local optData = makeUObject({ LastSavedDataIndex = 0 })

    local summaryEntries = {}
    for slot = 0, 10 do
      local exists = slot == 0 or slot == 1
      summaryEntries[#summaryEntries + 1] = {
        slot,
        {
          bExists = exists,
          LoadStatus = exists and 0 or 7,
          MainProgress = slot == 0 and 1040 or (slot == 1 and 1030 or -1),
          CurrentMapId = exists and 3001 or -1,
          PlayLevel = exists and 3 or -1,
          PlayTimeSec = slot == 0 and 3525.0 or (slot == 1 and 1829.26 or 0.0),
          DateTime = { Ticks = slot == 0 and "639311658396860000" or (slot == 1 and "639311640000000000" or 0) },
          difficulty = 1,
          bIsCleared = false,
          PartyUnitIds = makeTArray(exists and { 1001, 1002 } or {}),
        },
      }
    end

    local saveMgr = makeUObject({
      m_pGameSaveData = saveData,
      m_pOptionSaveData = optData,
      GameSaveDataSummaryMap = makeTMap(summaryEntries),
    })

    local vm1001 = makeUnitVm({
      hp = { 287, 382 }, -- real-time battle HP differs from SaveData 382
      mp = { 33, 43 },
      stageLevel = 3,
      stats = {
        [5] = { 46 }, [6] = { 30 }, [7] = { 20 }, [8] = { 32 }, [9] = { 9 },
        [11] = { 10, 100 },
        [12] = { 67 }, [13] = { 35 }, [14] = { 29 }, [15] = { 35 }, [16] = { 32 },
      },
    })
    local vm1002 = makeUnitVm({
      hp = { 231, 336 },
      mp = { 45, 50 },
      stageLevel = nil, -- exercise fallback to vm:GetLevel()
      vmGetLevel = 3,
      stats = {
        [5] = { 41 }, [6] = { 26 }, [7] = { 24 }, [8] = { 29 }, [9] = { 11 },
        [11] = { 0, 100 },
        [12] = { 60 }, [13] = { 31 }, [14] = { 33 }, [15] = { 32 }, [16] = { 29 },
      },
    })
    local vmById = { [1001] = vm1001, [1002] = vm1002 }
    local uvmMgr = makeUObject({
      GetUnitParamVM = function(_, uid) return vmById[uid] end,
      UnitParamVMMap = makeTMap({ { 1001, vm1001 }, { 1002, vm1002 } }),
    })
    local manualVmMgr = makeUObject({ UnitVMManager = uvmMgr })
    local scenSub = makeUObject({ m_CurrentDestinationMapIdList = makeTArray({ 3001 }) })
    local questSub = makeUObject({ MainQuestId = 1001 })

    local singletons = {
      CPP_MuchaSaveDataManager = saveMgr,
      CPP_ManualViewModelManager = manualVmMgr,
      CPP_UnitVMManager = uvmMgr,
      CPP_ScenarioSubsystem = scenSub,
      CPP_QuestSubsystem = questSub,
    }
    local findCalls = {}
    local errors = {}

    local function makeCtx(cache, tick, world)
      return {
        find = function(cls)
          findCalls[cls] = (findCalls[cls] or 0) + 1
          return singletons[cls]
        end,
        findAll = function() return {} end,
        err = function(field, msg) errors[#errors + 1] = field .. ": " .. tostring(msg) end,
        cache = cache,
        clockMs = tick * 500,
        tick = tick,
        world = world or "World /Game/Maps/Dng_01.Dng_01",
      }
    end

    return {
      saveMgr = saveMgr,
      saveData = saveData,
      optData = optData,
      manualVmMgr = manualVmMgr,
      uvmMgr = uvmMgr,
      vm1001 = vm1001,
      vm1002 = vm1002,
      scenSub = scenSub,
      questSub = questSub,
      singletons = singletons,
      findCalls = findCalls,
      errors = errors,
      makeCtx = makeCtx,
    }
  end
`;

interface PartyMember {
  unit_id: number;
  hp: number | null;
  max_hp: number | null;
  mp: number | null;
  max_mp: number | null;
  level: number | null;
  exp: number | null;
  weapon_id: number | null;
  armor_id: number | null;
  accessory_ids: number[] | null;
  vision_id: number | null;
  join_status: number | null;
}

interface UnitStats {
  attack: number | null;
  defence: number | null;
  intelligence: number | null;
  mind: number | null;
  agility: number | null;
  equip_cost: number | null;
  max_equip_cost: number | null;
  physical_attack: number | null;
  physical_defense: number | null;
  magical_attack: number | null;
  magical_defense: number | null;
  divine: number | null;
}

interface InventoryDelta {
  item_id: number;
  before: number;
  after: number;
  delta: number;
}

interface InventorySummary {
  count: number;
  total_qty: number;
  hash: string;
  changed: InventoryDelta[];
}

interface EventFlagsSummary {
  total: number;
  true_count: number;
  hash: string;
  added: number[];
  removed: number[];
}

interface PartyData {
  save_ready: boolean;
  money: number | null;
  play_time: number | null;
  party_ids: number[] | null;
  guest_unit_id: number | null;
  party: PartyMember[] | null;
  party_stats: Record<string, UnitStats | null> | null;
  inventory: InventorySummary | null;
  progress: number | null;
  chapter: number | null;
  destination_map_ids: number[] | null;
  event_flags: EventFlagsSummary | null;
  main_quest_id: number | null;
  last_saved_slot: number | null;
}

interface SaveSlotItem {
  slot_index: number;
  is_auto_save: boolean;
  exists: boolean;
  load_status: number | null;
  main_progress: number | null;
  chapter: number | null;
  current_map_id: number | null;
  play_level: number | null;
  play_time: number | null;
  date_time_ticks: string | null;
  difficulty: number | null;
  is_cleared: boolean | null;
  party_unit_ids: number[];
}

interface SaveSlotsResult {
  slot_count: number;
  last_saved_slot: number | null;
  slots: SaveSlotItem[];
}

interface FullInventoryResult {
  count: number;
  total_qty: number;
  hash: string;
  items: { item_id: number; num: number }[];
}

interface FullEventFlagsResult {
  total: number;
  true_count: number;
  hash: string;
  true_ids: number[];
}

void test("collect outputs all 14 party fields with real-time VM HP/MP, level fallback, and summaries", () => {
  const result = luaJson<{
    data: PartyData;
    errors: string[];
    findCalls: Record<string, number>;
    rawJson: string;
  }>(`
    ${FAKE_PARTY_HELPER}
    local json = require("ffrbridge.json")
    local party = require("ffrbridge.topics.party")
    local fx = makeHealthyFixture()
    local cache = {}
    local data = party.collect(fx.makeCtx(cache, 1))
    return {
      data = data,
      errors = json.array(fx.errors),
      findCalls = fx.findCalls,
      rawJson = json.encode(data),
    }
  `);

  assert.deepEqual(result.errors, []);
  // Cold tick should do exactly 4 singleton lookups and zero redundant fallback finds.
  assert.deepEqual(result.findCalls, {
    CPP_MuchaSaveDataManager: 1,
    CPP_ManualViewModelManager: 1,
    CPP_ScenarioSubsystem: 1,
    CPP_QuestSubsystem: 1,
  });

  // Integers must serialize without .0 suffix.
  assert.match(result.rawJson, /"money":2080\b/u);
  assert.match(result.rawJson, /"progress":1040\b/u);
  assert.match(result.rawJson, /"chapter":1\b/u);
  assert.match(result.rawJson, /"hp":287\b/u);

  const d = result.data;
  assert.equal(d.save_ready, true);
  assert.equal(d.money, 2080);
  assert.equal(d.play_time, 3525);
  assert.deepEqual(d.party_ids, [1001, 1002]);
  assert.equal(d.guest_unit_id, null);
  assert.equal(d.progress, 1040);
  assert.equal(d.chapter, 1);
  assert.deepEqual(d.destination_map_ids, [3001]);
  assert.equal(d.main_quest_id, 1001);
  assert.equal(d.last_saved_slot, 0);

  assert.deepEqual(d.party, [
    {
      unit_id: 1001,
      hp: 287,
      max_hp: 382,
      mp: 33,
      max_mp: 43,
      level: 3,
      exp: 473,
      weapon_id: 10100,
      armor_id: 11000,
      accessory_ids: [12000],
      vision_id: null,
      join_status: 0,
    },
    {
      unit_id: 1002,
      hp: 231,
      max_hp: 336,
      mp: 45,
      max_mp: 50,
      level: 3,
      exp: 473,
      weapon_id: 10200,
      armor_id: 11000,
      accessory_ids: [12000, 12001],
      vision_id: 5001,
      join_status: 0,
    },
  ]);

  assert.deepEqual(d.party_stats, {
    "1001": {
      attack: 46,
      defence: 30,
      intelligence: 20,
      mind: 32,
      agility: 9,
      equip_cost: 10,
      max_equip_cost: 100,
      physical_attack: 67,
      physical_defense: 35,
      magical_attack: 29,
      magical_defense: 35,
      divine: 32,
    },
    "1002": {
      attack: 41,
      defence: 26,
      intelligence: 24,
      mind: 29,
      agility: 11,
      equip_cost: 0,
      max_equip_cost: 100,
      physical_attack: 60,
      physical_defense: 31,
      magical_attack: 33,
      magical_defense: 32,
      divine: 29,
    },
  });

  assert.ok(d.inventory);
  assert.equal(d.inventory.count, 3);
  assert.equal(d.inventory.total_qty, 13);
  assert.match(d.inventory.hash, /^[0-9a-f]{8}$/u);
  assert.deepEqual(d.inventory.changed, []);

  assert.ok(d.event_flags);
  assert.equal(d.event_flags.total, 2048);
  assert.equal(d.event_flags.true_count, 7);
  assert.match(d.event_flags.hash, /^[0-9a-f]{8}$/u);
  assert.deepEqual(d.event_flags.added, []);
  assert.deepEqual(d.event_flags.removed, []);
});

void test("cache reuses singletons across ticks with 0 lookups and re-finds after invalidation or map change", () => {
  const result = luaJson<{
    afterTick1: Record<string, number>;
    afterTick2: Record<string, number>;
    afterInvalidation: Record<string, number>;
    afterMapChange: Record<string, number>;
    tick4QuestId: number | null;
    errors: string[];
  }>(`
    ${FAKE_PARTY_HELPER}
    local json = require("ffrbridge.json")
    local party = require("ffrbridge.topics.party")
    local fx = makeHealthyFixture()
    local cache = {}

    local function snapshotCounts()
      local out = {}
      for k, v in pairs(fx.findCalls) do out[k] = v end
      return out
    end

    party.collect(fx.makeCtx(cache, 1, "World /Game/Maps/Dng_01.Dng_01"))
    local afterTick1 = snapshotCounts()

    -- Tick 2: everything cached & valid -> 0 ctx.find calls
    party.collect(fx.makeCtx(cache, 2, "World /Game/Maps/Dng_01.Dng_01"))
    local afterTick2 = snapshotCounts()

    -- Tick 3: invalidate QuestSubsystem only -> only CPP_QuestSubsystem re-found
    fx.questSub._valid = false
    local newQuestSub = makeUObject({ MainQuestId = 1002 })
    fx.singletons.CPP_QuestSubsystem = newQuestSub
    party.collect(fx.makeCtx(cache, 3, "World /Game/Maps/Dng_01.Dng_01"))
    local afterInvalidation = snapshotCounts()

    -- Tick 4: world changes -> all 4 cached singletons re-queried
    local d4 = party.collect(fx.makeCtx(cache, 4, "World /Game/Maps/Town_01.Town_01"))
    local afterMapChange = snapshotCounts()

    return {
      afterTick1 = afterTick1,
      afterTick2 = afterTick2,
      afterInvalidation = afterInvalidation,
      afterMapChange = afterMapChange,
      tick4QuestId = d4.main_quest_id,
      errors = json.array(fx.errors),
    }
  `);

  assert.deepEqual(result.errors, []);
  assert.deepEqual(result.afterTick1, {
    CPP_MuchaSaveDataManager: 1,
    CPP_ManualViewModelManager: 1,
    CPP_ScenarioSubsystem: 1,
    CPP_QuestSubsystem: 1,
  });
  assert.deepEqual(result.afterTick2, result.afterTick1);
  assert.deepEqual(result.afterInvalidation, {
    CPP_MuchaSaveDataManager: 1,
    CPP_ManualViewModelManager: 1,
    CPP_ScenarioSubsystem: 1,
    CPP_QuestSubsystem: 2,
  });
  assert.deepEqual(result.afterMapChange, {
    CPP_MuchaSaveDataManager: 2,
    CPP_ManualViewModelManager: 2,
    CPP_ScenarioSubsystem: 2,
    CPP_QuestSubsystem: 3,
  });
  assert.equal(result.tick4QuestId, 1002);
});

void test("missing or invalid singletons degrade gracefully to null and record errors without breaking others", () => {
  const result = luaJson<{
    noSaveMgr: PartyData;
    noSaveMgrErrors: string[];
    noVmFallback: PartyData;
    noVmErrors: string[];
  }>(`
    ${FAKE_PARTY_HELPER}
    local json = require("ffrbridge.json")
    local party = require("ffrbridge.topics.party")

    -- Case 1: CPP_MuchaSaveDataManager missing, Scenario & Quest still healthy
    local fx1 = makeHealthyFixture()
    fx1.singletons.CPP_MuchaSaveDataManager = nil
    local noSaveMgr = party.collect(fx1.makeCtx({}, 1))

    -- Case 2: Both ManualViewModelManager and UnitVMManager invalid -> HP/MP fall back to SaveData
    local fx2 = makeHealthyFixture()
    fx2.manualVmMgr._valid = false
    fx2.uvmMgr._valid = false
    local noVmFallback = party.collect(fx2.makeCtx({}, 1))

    return {
      noSaveMgr = noSaveMgr,
      noSaveMgrErrors = json.array(fx1.errors),
      noVmFallback = noVmFallback,
      noVmErrors = json.array(fx2.errors),
    }
  `);

  assert.equal(result.noSaveMgr.save_ready, false);
  assert.equal(result.noSaveMgr.money, null);
  assert.equal(result.noSaveMgr.party, null);
  assert.equal(result.noSaveMgr.inventory, null);
  assert.equal(result.noSaveMgr.event_flags, null);
  assert.equal(result.noSaveMgr.last_saved_slot, null);
  // Independent subsystems still succeed.
  assert.deepEqual(result.noSaveMgr.destination_map_ids, [3001]);
  assert.equal(result.noSaveMgr.main_quest_id, 1001);
  assert.equal(result.noSaveMgrErrors.length, 1);
  assert.match(result.noSaveMgrErrors[0] ?? "", /save_manager/u);

  // Without VM, party still outputs SaveData HP/MP and equipment, while max_hp/max_mp/level/party_stats are null.
  assert.ok(result.noVmFallback.party);
  const firstFallback = result.noVmFallback.party[0];
  assert.ok(firstFallback);
  assert.equal(firstFallback.hp, 382);
  assert.equal(firstFallback.max_hp, null);
  assert.equal(firstFallback.mp, 43);
  assert.equal(firstFallback.max_mp, null);
  assert.equal(firstFallback.level, null);
  assert.equal(firstFallback.weapon_id, 10100);
  assert.equal(result.noVmFallback.party_stats, null);
  assert.equal(result.noVmErrors.length, 1);
  assert.match(result.noVmErrors[0] ?? "", /unit_vm_manager/u);
});

void test("property read exceptions are caught per-field and reported to ctx.err without throwing", () => {
  const result = luaJson<{
    data: PartyData;
    errors: string[];
  }>(`
    ${FAKE_PARTY_HELPER}
    local json = require("ffrbridge.json")
    local party = require("ffrbridge.topics.party")
    local fx = makeHealthyFixture()

    -- Make Player.currentMoney throw while PlayTime succeeds
    fx.saveData.Player = setmetatable({ PlayTime = 42.16 }, {
      __index = function(_, key)
        if key == "currentMoney" then error("money read fault") end
        return nil
      end,
    })

    -- Make questSub.MainQuestId throw
    fx.singletons.CPP_QuestSubsystem = setmetatable({
      IsValid = function() return true end,
    }, {
      __index = function(_, key)
        if key == "MainQuestId" then error("quest memory fault") end
        return nil
      end,
    })

    -- Make vm1001.HitPointVM.MaxPoint throw while CurrentPoint succeeds
    fx.vm1001.HitPointVM = setmetatable({
      IsValid = function() return true end,
      CurrentPoint = 150,
    }, {
      __index = function(_, key)
        if key == "MaxPoint" then error("max hp fault") end
        return nil
      end,
    })

    local data = party.collect(fx.makeCtx({}, 1))
    return {
      data = data,
      errors = json.array(fx.errors),
    }
  `);

  assert.equal(result.data.money, null);
  assert.equal(result.data.play_time, 42.2);
  assert.equal(result.data.main_quest_id, null);
  assert.ok(result.data.party);
  const member0 = result.data.party[0];
  const member1 = result.data.party[1];
  assert.ok(member0);
  assert.ok(member1);
  assert.equal(member0.hp, 150);
  assert.equal(member0.max_hp, null);
  assert.equal(member1.hp, 231);
  assert.equal(result.data.progress, 1040);

  assert.equal(result.errors.length, 3);
  assert.ok(result.errors.some((e) => e.includes("party.money") && e.includes("money read fault")));
  assert.ok(
    result.errors.some(
      (e) => e.includes("party.main_quest_id") && e.includes("quest memory fault"),
    ),
  );
  assert.ok(
    result.errors.some((e) => e.includes("party.unit.1001.hp.max") && e.includes("max hp fault")),
  );
});

void test("save_ready, chapter transitions, inventory diffs, and event flag diffs work across ticks", () => {
  const result = luaJson<{
    t1Title: PartyData;
    t2Prologue: PartyData;
    t3Chest: PartyData;
    t6FlagFlip: PartyData;
    t10Steady: PartyData;
    errors: string[];
  }>(`
    ${FAKE_PARTY_HELPER}
    local json = require("ffrbridge.json")
    local party = require("ffrbridge.topics.party")
    local fx = makeHealthyFixture()
    local cache = {}

    -- Tick 1: Title Setup placeholder (Progress[0] = 0, LastSavedDataIndex = -1)
    fx.saveData.Progress.Progress[1] = 0
    fx.optData.LastSavedDataIndex = -1
    local t1Title = party.collect(fx.makeCtx(cache, 1))

    -- Tick 2: Enter Prologue (Progress[0] = 1005, guest unit 1013 joins) -> save_ready false->true resets diff baselines
    fx.saveData.Progress.Progress[1] = 1005
    fx.saveData.Party.guestUnitId = 1013
    fx.saveData.inventory = makeTArray({
      { ItemId = 1, Num = 6 },
      { ItemId = 3, Num = 3 },
    })
    fx.saveData.Progress.eventFlag = makeFlags({ 1, 8, 20 })
    local t2Prologue = party.collect(fx.makeCtx(cache, 2))

    -- Tick 3: Open chest (inventory len 2 -> 3) & use 1 Potion (6 -> 5) -> immediate dirty refresh on len change
    fx.saveData.inventory = makeTArray({
      { ItemId = 1, Num = 5 },
      { ItemId = 3, Num = 3 },
      { ItemId = 10201, Num = 1 },
    })
    local t3Chest = party.collect(fx.makeCtx(cache, 3))

    -- Tick 6: Advance to Chapter 1 (Progress[0] = 1020) and flip event flags (8 off, 14 & 2047 on)
    fx.saveData.Progress.Progress[1] = 1020
    fx.saveData.Progress.eventFlag = makeFlags({ 1, 14, 20, 2047 })
    local t6FlagFlip = party.collect(fx.makeCtx(cache, 6))

    -- Tick 10: 4 ticks later with no new changes -> diffs clear to []
    local t10Steady = party.collect(fx.makeCtx(cache, 10))

    return {
      t1Title = t1Title,
      t2Prologue = t2Prologue,
      t3Chest = t3Chest,
      t6FlagFlip = t6FlagFlip,
      t10Steady = t10Steady,
      errors = json.array(fx.errors),
    }
  `);

  assert.deepEqual(result.errors, []);

  assert.equal(result.t1Title.save_ready, false);
  assert.equal(result.t1Title.progress, 0);
  assert.equal(result.t1Title.chapter, null);
  assert.equal(result.t1Title.last_saved_slot, null);

  assert.equal(result.t2Prologue.save_ready, true);
  assert.equal(result.t2Prologue.progress, 1005);
  assert.equal(result.t2Prologue.chapter, 0);
  assert.equal(result.t2Prologue.guest_unit_id, 1013);
  // Entering gameplay from title resets baseline -> empty initial diffs.
  assert.ok(result.t2Prologue.inventory);
  assert.ok(result.t2Prologue.event_flags);
  assert.deepEqual(result.t2Prologue.inventory.changed, []);
  assert.deepEqual(result.t2Prologue.event_flags.added, []);
  assert.deepEqual(result.t2Prologue.event_flags.removed, []);

  assert.ok(result.t3Chest.inventory);
  assert.equal(result.t3Chest.inventory.count, 3);
  assert.equal(result.t3Chest.inventory.total_qty, 9);
  assert.notEqual(result.t3Chest.inventory.hash, result.t2Prologue.inventory.hash);
  assert.deepEqual(result.t3Chest.inventory.changed, [
    { item_id: 1, before: 6, after: 5, delta: -1 },
    { item_id: 10201, before: 0, after: 1, delta: 1 },
  ]);

  assert.equal(result.t6FlagFlip.chapter, 1);
  assert.ok(result.t6FlagFlip.event_flags);
  assert.equal(result.t6FlagFlip.event_flags.true_count, 4);
  assert.notEqual(result.t6FlagFlip.event_flags.hash, result.t2Prologue.event_flags.hash);
  assert.deepEqual(result.t6FlagFlip.event_flags.added, [14, 2047]);
  assert.deepEqual(result.t6FlagFlip.event_flags.removed, [8]);

  assert.ok(result.t10Steady.inventory);
  assert.ok(result.t10Steady.event_flags);
  assert.deepEqual(result.t10Steady.inventory.changed, []);
  assert.deepEqual(result.t10Steady.event_flags.added, []);
  assert.deepEqual(result.t10Steady.event_flags.removed, []);
});

void test("low-frequency caches skip ParamPointVMMap and eventFlag scans on warm steady ticks", () => {
  const result = luaJson<{
    statScansAfterT1: number;
    statScansAfterT2: number;
    statScansAfterT5: number;
    flagScansAfterT1: number;
    flagScansAfterT2: number;
    flagScansAfterT5: number;
  }>(`
    ${FAKE_PARTY_HELPER}
    local party = require("ffrbridge.topics.party")
    local fx = makeHealthyFixture()

    local statScans = 0
    local origForEach = fx.vm1001.ParamPointVMMap.ForEach
    fx.vm1001.ParamPointVMMap.ForEach = function(self, cb)
      statScans = statScans + 1
      return origForEach(self, cb)
    end

    local flagScans = 0
    local origFlagLen = fx.saveData.Progress.eventFlag.GetArrayNum
    fx.saveData.Progress.eventFlag.GetArrayNum = function(self)
      flagScans = flagScans + 1
      return origFlagLen(self)
    end

    local cache = {}
    party.collect(fx.makeCtx(cache, 1))
    local statScansAfterT1, flagScansAfterT1 = statScans, flagScans

    -- Ticks 2, 3, 4: unchanged state -> neither stats nor flags are re-scanned
    party.collect(fx.makeCtx(cache, 2))
    party.collect(fx.makeCtx(cache, 3))
    party.collect(fx.makeCtx(cache, 4))
    local statScansAfterT2, flagScansAfterT2 = statScans, flagScans

    -- Tick 5 (1 + 4): low-frequency refresh fires once
    party.collect(fx.makeCtx(cache, 5))
    local statScansAfterT5, flagScansAfterT5 = statScans, flagScans

    return {
      statScansAfterT1 = statScansAfterT1,
      statScansAfterT2 = statScansAfterT2,
      statScansAfterT5 = statScansAfterT5,
      flagScansAfterT1 = flagScansAfterT1,
      flagScansAfterT2 = flagScansAfterT2,
      flagScansAfterT5 = flagScansAfterT5,
    }
  `);

  assert.equal(result.statScansAfterT1, 1);
  assert.equal(result.statScansAfterT2, 1);
  assert.equal(result.statScansAfterT5, 2);
  assert.equal(result.flagScansAfterT1, 1);
  assert.equal(result.flagScansAfterT2, 1);
  assert.equal(result.flagScansAfterT5, 2);
});

void test("commands save_slots, inventory (>32 items untruncated), and event_flags return full read-only data", () => {
  const result = luaJson<{
    slotsMap: SaveSlotsResult;
    slotsFallback: SaveSlotsResult;
    invFull: FullInventoryResult;
    flagsFull: FullEventFlagsResult;
    summaryHashInv: string;
    summaryHashFlags: string;
    missingMgrErr: string | null;
  }>(`
    ${FAKE_PARTY_HELPER}
    local json = require("ffrbridge.json")
    local party = require("ffrbridge.topics.party")
    local fx = makeHealthyFixture()

    -- Populate 40 inventory items to verify no 32-item truncation
    local bigInv = {}
    for i = 1, 40 do
      bigInv[i] = { ItemId = 100 + i, Num = i }
    end
    fx.saveData.inventory = makeTArray(bigInv)

    local cache = {}
    local collected = party.collect(fx.makeCtx(cache, 1))
    local slotsMap, err1 = party.commands.save_slots(fx.makeCtx(cache, 1), {})
    assert(err1 == nil, tostring(err1))

    local invFull, err2 = party.commands.inventory(fx.makeCtx(cache, 1), {})
    assert(err2 == nil, tostring(err2))

    local flagsFull, err3 = party.commands.event_flags(fx.makeCtx(cache, 1), {})
    assert(err3 == nil, tostring(err3))

    -- Fallback path for save_slots via FindGameSaveDataSummary when GameSaveDataSummaryMap is nil
    local fxFb = makeHealthyFixture()
    fxFb.saveMgr.GameSaveDataSummaryMap = nil
    fxFb.saveMgr.GetSaveSlotNum = function() return 3 end
    fxFb.saveMgr.FindGameSaveDataSummary = function(_, idx, out)
      out.OutSummary = {
        bExists = idx == 2,
        LoadStatus = idx == 2 and 0 or 7,
        MainProgress = idx == 2 and 2050 or -1,
        CurrentMapId = idx == 2 and 4001 or -1,
        PlayLevel = idx == 2 and 12 or -1,
        PlayTimeSec = idx == 2 and 7200.5 or 0.0,
        DateTime = { Ticks = idx == 2 and "639312000000000000" or 0 },
        difficulty = 2,
        bIsCleared = idx == 2,
        PartyUnitIds = makeTArray(idx == 2 and { 1001, 1002, 1003 } or {}),
      }
      return true
    end
    local slotsFallback, err4 = party.commands.save_slots(fxFb.makeCtx({}, 1), {})
    assert(err4 == nil, tostring(err4))

    -- Missing saveMgr returns nil, errorMessage
    local fxMiss = makeHealthyFixture()
    fxMiss.singletons.CPP_MuchaSaveDataManager = nil
    local resMiss, errMiss = party.commands.save_slots(fxMiss.makeCtx({}, 1), {})
    assert(resMiss == nil)

    return {
      slotsMap = slotsMap,
      slotsFallback = slotsFallback,
      invFull = invFull,
      flagsFull = flagsFull,
      summaryHashInv = collected.inventory.hash,
      summaryHashFlags = collected.event_flags.hash,
      missingMgrErr = errMiss or json.null,
    }
  `);

  assert.equal(result.slotsMap.slot_count, 11);
  assert.equal(result.slotsMap.last_saved_slot, 0);
  const slot0 = result.slotsMap.slots[0];
  const slot1 = result.slotsMap.slots[1];
  const slot10 = result.slotsMap.slots[10];
  assert.ok(slot0);
  assert.ok(slot1);
  assert.ok(slot10);
  assert.equal(slot0.slot_index, 0);
  assert.equal(slot0.is_auto_save, true);
  assert.equal(slot0.exists, true);
  assert.equal(slot0.chapter, 1);
  assert.equal(slot0.date_time_ticks, "639311658396860000");
  assert.equal(slot1.slot_index, 1);
  assert.equal(slot1.is_auto_save, false);
  assert.equal(slot1.play_time, 1829.3);
  assert.equal(slot10.slot_index, 10);
  assert.equal(slot10.exists, false);
  assert.equal(slot10.date_time_ticks, null);
  assert.equal(slot10.play_time, null);
  assert.deepEqual(slot10.party_unit_ids, []);

  assert.equal(result.slotsFallback.slot_count, 3);
  const fbSlot2 = result.slotsFallback.slots[2];
  assert.ok(fbSlot2);
  assert.equal(fbSlot2.exists, true);
  assert.equal(fbSlot2.chapter, 2);
  assert.equal(fbSlot2.is_cleared, true);
  assert.deepEqual(fbSlot2.party_unit_ids, [1001, 1002, 1003]);

  assert.equal(result.invFull.count, 40);
  assert.equal(result.invFull.total_qty, (40 * 41) / 2);
  assert.equal(result.invFull.items.length, 40);
  const lastItem = result.invFull.items[39];
  assert.ok(lastItem);
  assert.equal(lastItem.item_id, 140);
  assert.equal(lastItem.num, 40);
  assert.equal(result.invFull.hash, result.summaryHashInv);

  assert.equal(result.flagsFull.total, 2048);
  assert.equal(result.flagsFull.true_count, 7);
  assert.deepEqual(result.flagsFull.true_ids, [14, 15, 22, 25, 105, 1160, 2047]);
  assert.equal(result.flagsFull.hash, result.summaryHashFlags);

  assert.match(result.missingMgrErr ?? "", /CPP_MuchaSaveDataManager/u);
});

void test("party.lua compiles cleanly as Lua 5.3", async () => {
  const body = await readFile(join(SCRIPTS, "ffrbridge", "topics", "party.lua"), "utf8");
  const state = lauxlib.luaL_newstate();
  try {
    load(state, body);
  } finally {
    lua.lua_close(state);
  }
});
