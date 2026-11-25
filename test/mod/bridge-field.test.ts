import assert from "node:assert/strict";
import { join } from "node:path";
import test from "node:test";
import { lua, lauxlib, lualib, to_jsstring, to_luastring } from "fengari";

const SCRIPTS = join(import.meta.dirname, "..", "..", "mod", "FfrBridge", "Scripts");

function load(state: ReturnType<typeof lauxlib.luaL_newstate>, code: string): void {
  const status = lauxlib.luaL_loadbuffer(
    state,
    to_luastring(code),
    null,
    to_luastring("bridge-field-test"),
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
    load(
      state,
      `${prelude}\nresult = require("ffrbridge.json").encode((function()\n${body}\nend)())`,
    );
    const status = lua.lua_pcall(state, 0, lua.LUA_MULTRET, 0);
    if (status !== lua.LUA_OK) {
      const message = lua.lua_tostring(state, -1);
      throw new Error(message ? to_jsstring(message) : "Lua 执行失败");
    }
    lua.lua_getglobal(state, to_luastring("result"));
    const result = lua.lua_tostring(state, -1);
    if (!result) throw new Error("Lua 没有返回字符串");
    return JSON.parse(to_jsstring(result)) as T;
  } finally {
    lua.lua_close(state);
  }
}

interface Vec3 {
  x: number;
  y: number;
  z: number;
}

interface FieldState {
  map_id: number | null;
  area_id: number | null;
  is_loading: boolean | null;
  in_battle: boolean | null;
  in_event: boolean | null;
  in_dialog: boolean | null;
  in_menu: boolean | null;
  can_control: boolean;
  camera_deg: number | null;
  camera_internal_rotation: boolean | null;
  player: {
    location: Vec3 | null;
    visible: boolean | null;
    unit_id: number | null;
    direction: string | null;
    direction_deg: number | null;
    movement_method: string | null;
    speed_2d: number | null;
  };
  interact_target: {
    kind: string;
    unique_id: number | null;
    event_id: string | null;
    interact_type: string | null;
    location: Vec3 | null;
    shop_id?: number;
    inn_id?: number;
    is_open?: boolean | null;
    can_teleport?: boolean | null;
  } | null;
  quest_targets: {
    main_quest_id: number | null;
    preferred_quest_id: number | null;
    destination_map_ids: number[];
  };
  unlocked_landmarks: number[];
  chests: {
    unique_id: number | null;
    chest_id: number | null;
    is_open: boolean | null;
    item_id: number | null;
    item_num: number | null;
    money: number | null;
    location: Vec3 | null;
  }[];
  save_points: {
    unique_id: number | null;
    type: string | null;
    is_teleport: boolean | null;
    can_teleport: boolean | null;
    target_map_id: number | null;
    target_point_id: number | null;
    location: Vec3 | null;
  }[];
  transitions_and_npcs: {
    triggers: {
      unique_id: number | null;
      map_id: number | null;
      point_id: number | null;
      is_same_map: boolean | null;
      is_auto: boolean | null;
      location: Vec3 | null;
    }[];
    npcs: {
      unique_id: number | null;
      unit_id: number | null;
      interact_type: string | null;
      shop_id: number | null;
      inn_id: number | null;
      location: Vec3 | null;
    }[];
  };
}

const FIELD_FIXTURE_HELPER = `
  local function makeTArray(items)
    local arr = { _items = items }
    function arr:GetArrayNum() return #self._items end
    return setmetatable(arr, {
      __index = function(tbl, key)
        if type(key) == "number" then return tbl._items[key] end
        return rawget(tbl, key)
      end,
      __len = function(tbl) return #tbl._items end,
    })
  end

  local function makeFName(text)
    return { ToString = function() return text end }
  end

  local function makeObj(clsFullName, props)
    local cls = {
      IsValid = function() return true end,
      GetFullName = function() return clsFullName end,
    }
    local obj = {
      _valid = true,
      IsValid = function(self) return self._valid end,
      GetClass = function() return cls end,
    }
    if props then
      for key, value in pairs(props) do obj[key] = value end
    end
    return obj
  end

  local function makeHealthyWorldFixture()
    local world = {
      _valid = true,
      _fullName = "World /Game/Map/Dng/01Gra/01Gra_40/Dng_01Gra_40_PL.Dng_01Gra_40_PL",
      IsValid = function(self) return self._valid end,
      GetFullName = function(self) return self._fullName end,
    }

    local stackGame = makeObj("Class /Script/EnhancedUI.CPP_EU_ActivatableWidgetStack", {
      GetFName = function() return makeFName("Stack_Game") end,
      GetFullName = function() return "CPP_EU_ActivatableWidgetStack /Engine/Transient.Stack_Game" end,
    })
    local stackSystem = makeObj("Class /Script/EnhancedUI.CPP_EU_ActivatableWidgetStack", {
      GetFName = function() return makeFName("Stack_System") end,
      GetFullName = function() return "CPP_EU_ActivatableWidgetStack /Engine/Transient.Stack_System" end,
    })
    local activeLayer = stackGame

    local gameLayout = makeObj("WidgetBlueprintGeneratedClass /Game/UI/WBP_OverAllLayout.WBP_OverAllLayout_C", {
      Stack_Game = stackGame,
      GetInputTargetLayer = function() return activeLayer end,
    })
    local loadMgr = makeObj("Class /Script/mucha.CPP_LoadingScreenManager", {
      IsShowLoadingScreen = false,
    })
    local dialogMgr = makeObj("Class /Script/mucha.CPP_DialogManager", {
      DialogWidget = nil,
    })
    local widgetMgr = makeObj("Class /Script/mucha.CPP_GameWidgetManagerSubsystem", {
      m_LoadScrManager = loadMgr,
      m_DialogManager = dialogMgr,
      m_GameLayout = gameLayout,
    })

    local areaMgr = makeObj("Class /Script/mucha.CPP_MiniMapAreaManager", {
      CurrentAreaId = 2,
    })
    local miniMapSub = makeObj("Class /Script/mucha.CPP_MiniMapSubsystem", {
      PropertyKey = { mapId = 3000, MovementMethod = 1 },
      AreaManager = areaMgr,
    })

    local btlSub = makeObj("Class /Script/mucha.CPP_UI_BattleSubsystem", {
      bDuringBattle = false,
    })
    local talkSub = makeObj("Class /Script/mucha.CPP_TalkEventSubsystem", {
      m_pEventManager = nil,
    })
    local mainMenuOpen = false
    local mainMenuSub = makeObj("Class /Script/mucha.CPP_MainMenuSubsystem", {
      IsOpenMenu = function() return mainMenuOpen end,
    })
    local miniMapVm = makeObj("BlueprintGeneratedClass /Game/UI/VM_MiniMap.VM_MiniMap_C", {
      PlayerDirection = 2,
      PlayerDirectionDegree = 180.0,
      CameraDirectionDeg = 15.25,
    })

    local chest = makeObj("BlueprintGeneratedClass /Game/BP/Map/Gimmick/BP_MapGimmick_TreasureBox.BP_MapGimmick_TreasureBox_C", {
      m_UniqueId = 4,
      m_Id = 1,
      m_IsOpen = false,
      m_ItemId = 12009, -- legacy wrong field that must be ignored
      m_MapInteractType = 11,
      interactType = 11,
      m_TreasureBoxData = { ID = 1, mapId = 3000, ItemId = 4, ItemNum = 1, Money = 0 },
      K2_GetActorLocation = function() return { X = 5268.14, Y = 1409.06, Z = -888.44 } end,
    })
    local savePoint = makeObj("BlueprintGeneratedClass /Game/BP/Map/Gimmick/BP_MapGimmick_SavePoint.BP_MapGimmick_SavePoint_C", {
      m_UniqueId = 16,
      m_Type = 1,
      m_IsTeleport = true,
      m_TranditionMapId = 3001,
      m_TranditionPoint = 10000,
      m_MapInteractType = 33,
      interactType = 33,
      CanTeleprot = function() return true end,
      K2_GetActorLocation = function() return { X = -100.0, Y = 200.5, Z = -50.0 } end,
    })
    local trigger = makeObj("BlueprintGeneratedClass /Game/BP/Map/Transition/BP_MapTransitionTrigger.BP_MapTransitionTrigger_C", {
      m_UniqueId = 1,
      m_MapId = 3001,
      m_PointID = 2,
      m_IsSameMapTrasition = false,
      m_IsAutoTransition = true,
      K2_GetActorLocation = function() return { X = 0.0, Y = -500.0, Z = 10.0 } end,
    })
    local shopNpc = makeObj("BlueprintGeneratedClass /Game/BP/Map/Unit/BP_MapUnit_NPC.BP_MapUnit_NPC_C", {
      m_UniqueId = 3,
      m_MapUnitId = 1060,
      m_MapObjectType = 1,
      m_IsVisible = true,
      m_MapInteractType = 17,
      ShopEntry = { ShopID = 2 },
      InnEntry = { InnID = -1 },
      K2_GetActorLocation = function() return { X = 300.0, Y = 400.0, Z = 0.0 } end,
    })
    local hiddenNpc = makeObj("BlueprintGeneratedClass /Game/BP/Map/Unit/BP_MapUnit_NPC.BP_MapUnit_NPC_C", {
      m_UniqueId = 9,
      m_MapUnitId = 1061,
      m_MapObjectType = 1,
      m_IsVisible = false,
      m_MapInteractType = 1,
      ShopEntry = { ShopID = -1 },
      InnEntry = { InnID = -1 },
      K2_GetActorLocation = function() return { X = 999.0, Y = 999.0, Z = 0.0 } end,
    })

    local moveComp = makeObj("Class /Script/Engine.CharacterMovementComponent", {
      Velocity = { X = 600.0, Y = 450.0, Z = 0.0 }, -- 2D norm = 750.0
    })
    local pawn = makeObj("BlueprintGeneratedClass /Game/BP/Map/Unit/BP_MapUnit_Player.BP_MapUnit_Player_C", {
      m_IsVisible = true,
      m_MapUnitId = 10,
      m_MapMovementMethod = 1,
      m_Direction = 1, -- always North in-game; must NOT be used
      m_IsEnableUnitInput = true,
      m_IsEnableMovement = true,
      m_MapCameraInternalRotation = false,
      m_pPawnMovementComponent = moveComp,
      m_pInteractMapGimmickList = makeTArray({ chest }),
      m_InteractMapUnitList = makeTArray({}),
      m_InteractEventId = { EventId = makeFName("treasureBox") },
      K2_GetActorLocation = function() return { X = 5200.04, Y = 1400.06, Z = -888.41 } end,
    })
    local pc = makeObj("Class /Script/mucha.CPP_MuchaPlayerController", { Pawn = pawn })

    local mapMgr = makeObj("Class /Script/mucha.CPP_MapManager", {
      GetMapId = function() return 3000 end,
      GetMapAreaId = function() return 2 end,
      m_MapGimmickManager = makeObj("Class /Script/mucha.CPP_MapGimmickManager", {
        m_MapGimmickList = makeTArray({ chest, savePoint }),
      }),
      m_pMapTransitionTriggerStorage = makeObj("Class /Script/mucha.CPP_MapTransitionTriggerStorage", {
        m_TransitionTriggerList = makeTArray({ trigger }),
      }),
      m_MapUnitStorage = makeObj("Class /Script/mucha.CPP_MapUnitStorage", {
        m_MapUnits = makeTArray({ pawn, shopNpc, hiddenNpc }),
      }),
    })

    local questSub = makeObj("Class /Script/mucha.CPP_QuestSubsystem", {
      MainQuestId = 1001,
      PreferredQuestId = -1,
    })
    local scenSub = makeObj("Class /Script/mucha.CPP_ScenarioSubsystem", {
      m_CurrentDestinationMapIdList = makeTArray({ 3001 }),
    })
    local landmarkVm = makeObj("BlueprintGeneratedClass /Game/UI/VM_WldMapLandmarkList.VM_WldMapLandmarkList_C", {
      AllLandmarkIdList = makeTArray({ 3001, 2014 }),
    })

    local objects = {
      World = world,
      PlayerController = pc,
      CPP_GameWidgetManagerSubsystem = widgetMgr,
      CPP_MiniMapSubsystem = miniMapSub,
      CPP_UI_BattleSubsystem = btlSub,
      CPP_TalkEventSubsystem = talkSub,
      CPP_MainMenuSubsystem = mainMenuSub,
      VM_MiniMap_C = miniMapVm,
      CPP_MapManager = mapMgr,
      CPP_QuestSubsystem = questSub,
      CPP_ScenarioSubsystem = scenSub,
      VM_WldMapLandmarkList_C = landmarkVm,
    }

    return {
      objects = objects,
      world = world,
      pawn = pawn,
      pc = pc,
      loadMgr = loadMgr,
      dialogMgr = dialogMgr,
      btlSub = btlSub,
      talkSub = talkSub,
      miniMapSub = miniMapSub,
      areaMgr = areaMgr,
      chest = chest,
      savePoint = savePoint,
      shopNpc = shopNpc,
      stackGame = stackGame,
      stackSystem = stackSystem,
      setActiveLayer = function(layer) activeLayer = layer end,
      setMainMenuOpen = function(open) mainMenuOpen = open end,
    }
  end
`;

void test("field topic collects all 21 fields accurately in healthy field state", () => {
  const result = luaJson<{ field: FieldState; errors: string[] }>(`
    ${FIELD_FIXTURE_HELPER}
    local json = require("ffrbridge.json")
    local field = require("ffrbridge.topics.field")
    local fix = makeHealthyWorldFixture()
    local errors = {}
    local ctx = {
      find = function(name) return fix.objects[name] end,
      findAll = function() return {} end,
      err = function(f, m) errors[#errors + 1] = tostring(f) .. ": " .. tostring(m) end,
      cache = {},
      clockMs = 1000,
      tick = 1,
      world = fix.world._fullName,
    }
    return { field = field.collect(ctx), errors = json.array(errors) }
  `);

  assert.deepEqual(result.errors, []);
  const f = result.field;
  assert.equal(f.map_id, 3000);
  assert.equal(f.area_id, 2);
  assert.equal(f.is_loading, false);
  assert.equal(f.in_battle, false);
  assert.equal(f.in_event, false);
  assert.equal(f.in_dialog, false);
  assert.equal(f.in_menu, false);
  assert.equal(f.can_control, true);
  assert.equal(f.camera_deg, 15.3);
  assert.equal(f.camera_internal_rotation, false);

  assert.deepEqual(f.player, {
    location: { x: 5200, y: 1400.1, z: -888.4 },
    visible: true,
    unit_id: 10,
    direction: "South",
    direction_deg: 180,
    movement_method: "Walking",
    speed_2d: 750,
  });

  assert.deepEqual(f.interact_target, {
    kind: "treasure_box",
    unique_id: 4,
    event_id: "treasureBox",
    interact_type: "Treasure",
    location: { x: 5268.1, y: 1409.1, z: -888.4 },
    is_open: false,
  });

  assert.deepEqual(f.quest_targets, {
    main_quest_id: 1001,
    preferred_quest_id: -1,
    destination_map_ids: [3001],
  });
  assert.deepEqual(f.unlocked_landmarks, [3001, 2014]);

  assert.deepEqual(f.chests, [
    {
      unique_id: 4,
      chest_id: 1,
      is_open: false,
      item_id: 4, // must come from m_TreasureBoxData, not legacy m_ItemId=12009
      item_num: 1,
      money: 0,
      location: { x: 5268.1, y: 1409.1, z: -888.4 },
    },
  ]);

  assert.deepEqual(f.save_points, [
    {
      unique_id: 16,
      type: "Boss",
      is_teleport: true,
      can_teleport: true,
      target_map_id: 3001,
      target_point_id: 10000,
      location: { x: -100, y: 200.5, z: -50 },
    },
  ]);

  assert.deepEqual(f.transitions_and_npcs, {
    triggers: [
      {
        unique_id: 1,
        map_id: 3001,
        point_id: 2,
        is_same_map: false,
        is_auto: true,
        location: { x: 0, y: -500, z: 10 },
      },
    ],
    npcs: [
      {
        unique_id: 3,
        unit_id: 1060,
        interact_type: "Shop",
        shop_id: 2,
        inn_id: -1,
        location: { x: 300, y: 400, z: 0 },
      },
    ],
  });
});

void test("field topic enforces can_control gates, interact_target variants, and speed_2d fallback", () => {
  const result = luaJson<{
    stagingControl: boolean;
    hiddenControl: boolean;
    systemLayerControl: boolean;
    menuControl: boolean;
    savePointTarget: FieldState["interact_target"];
    npcTarget: FieldState["interact_target"];
    noTarget: FieldState["interact_target"];
    fallbackSpeed1: number | null;
    fallbackSpeed2: number | null;
  }>(`
    ${FIELD_FIXTURE_HELPER}
    local json = require("ffrbridge.json")
    local field = require("ffrbridge.topics.field")
    local fix = makeHealthyWorldFixture()

    local function sampleAt(cache, clockMs, tick)
      return field.collect({
        find = function(name) return fix.objects[name] end,
        findAll = function() return {} end,
        err = function() end,
        cache = cache,
        clockMs = clockMs,
        tick = tick,
        world = fix.world._fullName,
      })
    end

    -- 1. Cutscene staging coord blocks control
    local origLoc = fix.pawn.K2_GetActorLocation
    fix.pawn.K2_GetActorLocation = function() return { X = 25408.0, Y = 384.0, Z = -500.0 } end
    local stagingControl = sampleAt({}, 1000, 1).can_control
    fix.pawn.K2_GetActorLocation = origLoc

    -- 2. Invisible player blocks control
    fix.pawn.m_IsVisible = false
    local hiddenControl = sampleAt({}, 1000, 1).can_control
    fix.pawn.m_IsVisible = true

    -- 3. System menu layer (Stack_System) blocks control even when MainMenu is closed
    fix.setActiveLayer(fix.stackSystem)
    local systemLayerControl = sampleAt({}, 1000, 1).can_control
    fix.setActiveLayer(fix.stackGame)

    -- 4. Main menu open blocks control
    fix.setMainMenuOpen(true)
    local menuControl = sampleAt({}, 1000, 1).can_control
    fix.setMainMenuOpen(false)

    -- 5. interact_target: save_point with can_teleport = false (verify false is NOT turned into null)
    fix.savePoint.CanTeleprot = function() return false end
    fix.pawn.m_pInteractMapGimmickList = makeTArray({ fix.savePoint })
    fix.pawn.m_InteractEventId = { EventId = makeFName("savePoint") }
    local savePointTarget = sampleAt({}, 1000, 1).interact_target

    -- 6. interact_target: shop NPC
    fix.pawn.m_pInteractMapGimmickList = makeTArray({})
    fix.pawn.m_InteractMapUnitList = makeTArray({ fix.shopNpc })
    fix.pawn.m_InteractEventId = { EventId = makeFName("None") }
    local npcTarget = sampleAt({}, 1000, 1).interact_target

    -- 7. interact_target: none
    fix.pawn.m_InteractMapUnitList = makeTArray({})
    local noTarget = sampleAt({}, 1000, 1).interact_target

    -- 8. speed_2d fallback using delta position / delta clock_ms when movement component is absent
    fix.pawn.m_pPawnMovementComponent = nil
    local speedCache = {}
    fix.pawn.K2_GetActorLocation = function() return { X = 0.0, Y = 0.0, Z = 0.0 } end
    local s1 = sampleAt(speedCache, 1000, 1)
    fix.pawn.K2_GetActorLocation = function() return { X = 300.0, Y = 400.0, Z = 0.0 } end
    local s2 = sampleAt(speedCache, 1500, 2) -- 500 u in 0.5 s = 1000.0 u/s

    return {
      stagingControl = stagingControl,
      hiddenControl = hiddenControl,
      systemLayerControl = systemLayerControl,
      menuControl = menuControl,
      savePointTarget = savePointTarget,
      npcTarget = npcTarget,
      noTarget = noTarget or json.null,
      fallbackSpeed1 = s1.player.speed_2d,
      fallbackSpeed2 = s2.player.speed_2d,
    }
  `);

  assert.equal(result.stagingControl, false);
  assert.equal(result.hiddenControl, false);
  assert.equal(result.systemLayerControl, false);
  assert.equal(result.menuControl, false);
  assert.deepEqual(result.savePointTarget, {
    kind: "save_point",
    unique_id: 16,
    event_id: "savePoint",
    interact_type: "SavePoint",
    location: { x: -100, y: 200.5, z: -50 },
    can_teleport: false,
  });
  assert.deepEqual(result.npcTarget, {
    kind: "npc",
    unique_id: 3,
    event_id: null,
    interact_type: "Shop",
    location: { x: 300, y: 400, z: 0 },
    shop_id: 2,
  });
  assert.equal(result.noTarget, null);
  assert.equal(result.fallbackSpeed1, null);
  assert.equal(result.fallbackSpeed2, 1000);
});

void test("field topic handles missing/invalid objects, fallbacks, and property read exceptions", () => {
  const result = luaJson<{
    missingAll: FieldState;
    missingErrors: string[];
    fallbackMenu: FieldState;
    fallbackErrors: string[];
    throwingField: FieldState;
    throwingErrors: string[];
  }>(`
    ${FIELD_FIXTURE_HELPER}
    local json = require("ffrbridge.json")
    local field = require("ffrbridge.topics.field")

    -- Case 1: every singleton is missing or invalid
    local invalidObj = { IsValid = function() return false end }
    local missingErrors = {}
    local missingAll = field.collect({
      find = function(name)
        if name == "CPP_UI_BattleSubsystem" then return invalidObj end
        return nil
      end,
      findAll = function() return {} end,
      err = function(f, m) missingErrors[#missingErrors + 1] = tostring(f) .. ": " .. tostring(m) end,
      cache = {},
      clockMs = 0,
      tick = 1,
    })

    -- Case 2: fallbacks (CPP_MapManager for map_id/area_id, CPP_VM_MainMenu for in_menu, no landmark VM)
    local fix = makeHealthyWorldFixture()
    fix.objects.CPP_MiniMapSubsystem = nil
    fix.objects.CPP_MainMenuSubsystem = nil
    fix.objects.VM_WldMapLandmarkList_C = nil
    fix.objects.CPP_VM_MainMenu = makeObj("Class /Script/mucha.CPP_VM_MainMenu", {
      SceneDataStack = makeTArray({ "UI.MainMenu.Scene.Item" }),
    })
    local fallbackErrors = {}
    local fallbackMenu = field.collect({
      find = function(name) return fix.objects[name] end,
      findAll = function() return {} end,
      err = function(f, m) fallbackErrors[#fallbackErrors + 1] = tostring(f) .. ": " .. tostring(m) end,
      cache = {},
      clockMs = 500,
      tick = 1,
      world = fix.world._fullName,
    })

    -- Case 3: throwing property getters on live objects
    local fixThrow = makeHealthyWorldFixture()
    fixThrow.objects.CPP_UI_BattleSubsystem = setmetatable({
      IsValid = function() return true end,
    }, {
      __index = function(_, key)
        if key == "bDuringBattle" then error("read bDuringBattle crashed") end
        return nil
      end,
    })
    fixThrow.pawn.K2_GetActorLocation = function() error("location boom") end
    fixThrow.savePoint.CanTeleprot = function() error("teleport check failed") end
    local throwingErrors = {}
    local throwingField = field.collect({
      find = function(name) return fixThrow.objects[name] end,
      findAll = function() return {} end,
      err = function(f, m) throwingErrors[#throwingErrors + 1] = tostring(f) .. ": " .. tostring(m) end,
      cache = {},
      clockMs = 1000,
      tick = 1,
      world = fixThrow.world._fullName,
    })

    return {
      missingAll = missingAll,
      missingErrors = json.array(missingErrors),
      fallbackMenu = fallbackMenu,
      fallbackErrors = json.array(fallbackErrors),
      throwingField = throwingField,
      throwingErrors = json.array(throwingErrors),
    }
  `);

  assert.equal(result.missingAll.map_id, null);
  assert.equal(result.missingAll.area_id, null);
  assert.equal(result.missingAll.is_loading, null);
  assert.equal(result.missingAll.in_battle, null);
  assert.equal(result.missingAll.in_event, null);
  assert.equal(result.missingAll.in_dialog, null);
  assert.equal(result.missingAll.in_menu, null);
  assert.equal(result.missingAll.can_control, false);
  assert.equal(result.missingAll.player.location, null);
  assert.deepEqual(result.missingAll.chests, []);
  assert.deepEqual(result.missingAll.unlocked_landmarks, []);
  assert.ok(result.missingErrors.length >= 5);

  assert.deepEqual(result.fallbackErrors, []);
  assert.equal(result.fallbackMenu.map_id, 3000);
  assert.equal(result.fallbackMenu.area_id, 2);
  assert.equal(result.fallbackMenu.in_menu, true);
  assert.equal(result.fallbackMenu.can_control, false);
  assert.deepEqual(result.fallbackMenu.unlocked_landmarks, []);

  assert.equal(result.throwingField.in_battle, null);
  assert.equal(result.throwingField.player.location, null);
  assert.equal(result.throwingField.can_control, false);
  assert.equal(result.throwingField.save_points[0]?.can_teleport, null);
  assert.equal(result.throwingField.map_id, 3000); // unrelated fields still work
  assert.ok(result.throwingErrors.some((e) => e.includes("bDuringBattle crashed")));
  assert.ok(result.throwingErrors.some((e) => e.includes("location boom")));
  assert.ok(result.throwingErrors.some((e) => e.includes("teleport check failed")));
});

void test("field topic reuses cached singletons on tick 2, rescans after events, and drops cache on map travel", () => {
  const result = luaJson<{
    tick1Finds: number;
    tick2Finds: number;
    tick2ChestOpen: boolean | null;
    afterEventChestOpen: boolean | null;
    duringLoadChestsLen: number;
    duringLoadControl: boolean;
    afterTravelFinds: number;
    afterTravelMapId: number | null;
  }>(`
    ${FIELD_FIXTURE_HELPER}
    local field = require("ffrbridge.topics.field")
    local fix = makeHealthyWorldFixture()
    local cache = {}
    local findCount = 0

    local function runTick(clockMs, tick)
      findCount = 0
      local out = field.collect({
        find = function(name)
          findCount = findCount + 1
          return fix.objects[name]
        end,
        findAll = function() return {} end,
        err = function() end,
        cache = cache,
        clockMs = clockMs,
        tick = tick,
        world = fix.world._fullName,
      })
      return out, findCount
    end

    -- Tick 1: cold map load populates GI + World caches and scans low-freq lists
    local out1, f1 = runTick(1000, 1)
    assert(out1.chests[1].is_open == false)

    -- Tick 2 (+500 ms, same map): 0 ctx.find calls and cached low-freq list reused
    fix.chest.m_IsOpen = true -- mutated without event/5s; cached list should stay stale until trigger
    local out2, f2 = runTick(1500, 2)

    -- Tick 3: event starts, then Tick 4: event ends -> triggers chest rescan
    fix.talkSub.m_pEventManager = makeObj("Class /Script/mucha.CPP_TalkEventManager")
    runTick(2000, 3)
    fix.talkSub.m_pEventManager = nil
    local out4 = runTick(2500, 4)

    -- Tick 5: loading starts -> discards world cache and returns empty chests
    fix.loadMgr.IsShowLoadingScreen = true
    local out5 = runTick(3000, 5)

    -- Tick 6: new map finished loading -> re-finds World objects cleanly
    fix.loadMgr.IsShowLoadingScreen = false
    fix.world._fullName = "World /Game/Map/Town/01Gra/Town_01Gra_03_PL.Town_01Gra_03_PL"
    fix.miniMapSub.PropertyKey.mapId = 2013
    local out6, f6 = runTick(3500, 6)

    return {
      tick1Finds = f1,
      tick2Finds = f2,
      tick2ChestOpen = out2.chests[1].is_open,
      afterEventChestOpen = out4.chests[1].is_open,
      duringLoadChestsLen = #out5.chests,
      duringLoadControl = out5.can_control,
      afterTravelFinds = f6,
      afterTravelMapId = out6.map_id,
    }
  `);

  assert.ok(result.tick1Finds > 0 && result.tick1Finds <= 12);
  assert.equal(result.tick2Finds, 0);
  assert.equal(result.tick2ChestOpen, false);
  assert.equal(result.afterEventChestOpen, true);
  assert.equal(result.duringLoadChestsLen, 0);
  assert.equal(result.duringLoadControl, false);
  assert.ok(result.afterTravelFinds > 0);
  assert.equal(result.afterTravelMapId, 2013);
});

void test("query_nav_path returns path points, supports StaticFindObject fallback, and validates inputs", () => {
  const result = luaJson<{
    okPath: { valid: boolean; partial: boolean; length: number; points: Vec3[] } | null;
    okErr: string | null;
    fallbackPath: { valid: boolean; partial: boolean; length: number; points: Vec3[] } | null;
    noPath: { valid: boolean; partial: boolean; length: number; points: Vec3[] } | null;
    badArgsErr: string | null;
    missingNavErr: string | null;
    throwErr: string | null;
  }>(`
    ${FIELD_FIXTURE_HELPER}
    local json = require("ffrbridge.json")
    local field = require("ffrbridge.topics.field")
    local fix = makeHealthyWorldFixture()

    local navPathObj = {
      IsValid = function() return true end,
      IsPartial = function() return false end,
      GetPathLength = function() return 512.34 end,
      PathPoints = makeTArray({
        { X = 0.0, Y = 0.0, Z = 10.0 },
        { X = 250.45, Y = 100.0, Z = 10.0 },
        { X = 500.0, Y = 120.04, Z = 10.0 },
      }),
    }
    local navSys = {
      IsValid = function() return true end,
      FindPathToLocationSynchronously = function(_, w, startVec, goalVec)
        assert(w == fix.world)
        assert(startVec.X == 0 and goalVec.X == 500)
        return navPathObj
      end,
    }
    fix.objects.NavigationSystemV1 = navSys

    local okPath, okErr = field.commands.query_nav_path({
      find = function(name) return fix.objects[name] end,
      cache = {},
    }, { start = { x = 0, y = 0, z = 10 }, goal = { x = 500, y = 120, z = 10 } })

    -- StaticFindObject fallback when FindFirstOf("NavigationSystemV1") returns nil
    fix.objects.NavigationSystemV1 = nil
    local fallbackPath = field.commands.query_nav_path({
      find = function(name) return fix.objects[name] end,
      staticFindObject = function(path)
        if path == "/Script/NavigationSystem.Default__NavigationSystemV1" then return navSys end
        return nil
      end,
      cache = {},
    }, { start = { x = 0, y = 0, z = 10 }, goal = { x = 500, y = 120, z = 10 } })

    -- Unreachable path (nil return)
    local noPath = field.commands.query_nav_path({
      find = function(name)
        if name == "World" then return fix.world end
        if name == "NavigationSystemV1" then
          return {
            IsValid = function() return true end,
            FindPathToLocationSynchronously = function() return nil end,
          }
        end
        return nil
      end,
      cache = {},
    }, { start = { x = 0, y = 0, z = 0 }, goal = { x = 99999, y = 0, z = 0 } })

    -- Invalid args
    local _, badArgsErr = field.commands.query_nav_path({
      find = function(name) return fix.objects[name] end,
      cache = {},
    }, { start = { x = 0 } })

    -- Missing NavigationSystemV1
    local _, missingNavErr = field.commands.query_nav_path({
      find = function(name) return fix.objects[name] end,
      cache = {},
    }, { start = { x = 0, y = 0, z = 0 }, goal = { x = 1, y = 2, z = 3 } })

    -- Throwing FindPathToLocationSynchronously
    local _, throwErr = field.commands.query_nav_path({
      find = function(name)
        if name == "World" then return fix.world end
        if name == "NavigationSystemV1" then
          return {
            IsValid = function() return true end,
            FindPathToLocationSynchronously = function() error("navmesh not built") end,
          }
        end
        return nil
      end,
      cache = {},
    }, { start = { x = 0, y = 0, z = 0 }, goal = { x = 1, y = 2, z = 3 } })

    return {
      okPath = okPath or json.null,
      okErr = okErr or json.null,
      fallbackPath = fallbackPath or json.null,
      noPath = noPath or json.null,
      badArgsErr = badArgsErr or json.null,
      missingNavErr = missingNavErr or json.null,
      throwErr = throwErr or json.null,
    }
  `);

  assert.equal(result.okErr, null);
  assert.deepEqual(result.okPath, {
    valid: true,
    partial: false,
    length: 512.3,
    points: [
      { x: 0, y: 0, z: 10 },
      { x: 250.5, y: 100, z: 10 },
      { x: 500, y: 120, z: 10 },
    ],
  });
  assert.deepEqual(result.fallbackPath, result.okPath);
  assert.deepEqual(result.noPath, { valid: false, partial: false, length: 0, points: [] });
  assert.match(result.badArgsErr ?? "", /start/u);
  assert.match(result.missingNavErr ?? "", /NavigationSystemV1/u);
  assert.match(result.throwErr ?? "", /navmesh not built/u);
});
