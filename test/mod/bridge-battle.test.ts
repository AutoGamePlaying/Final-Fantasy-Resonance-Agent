import assert from "node:assert/strict";
import { join } from "node:path";
import test from "node:test";
import { lua, lauxlib, lualib, to_jsstring, to_luastring } from "fengari";

const SCRIPTS = join(import.meta.dirname, "..", "..", "mod", "FfrBridge", "Scripts");

function runLua(body: string, encodeResult: boolean): string {
  const state = lauxlib.luaL_newstate();
  try {
    lualib.luaL_openlibs(state);
    const prelude = `package.path = ${JSON.stringify(join(SCRIPTS, "?.lua"))} .. ";" .. package.path\n`;
    const suffix = encodeResult
      ? '\nresult = require("ffrbridge.json").encode((function()\n'
      : "\nresult = (function()\n";
    const statusLoad = lauxlib.luaL_loadbuffer(
      state,
      to_luastring(`${prelude}${suffix}${body}\nend)()${encodeResult ? ")" : ""}`),
      null,
      to_luastring("bridge-battle-test"),
    );
    if (statusLoad !== lua.LUA_OK) {
      const message = lua.lua_tostring(state, -1);
      throw new Error(message ? to_jsstring(message) : "Lua 编译失败");
    }
    const statusRun = lua.lua_pcall(state, 0, lua.LUA_MULTRET, 0);
    if (statusRun !== lua.LUA_OK) {
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

interface BattleCmdTarget {
  skill_id: number | null;
  focus_uid: number | null;
  target_uids: number[] | null;
  all_target_uids: number[] | null;
  can_aoe: boolean | null;
  is_all: boolean | null;
}

interface BattleCmd {
  display: boolean | null;
  type: string | null;
  state: string | null;
  ancestors: number[] | null;
  focus_index: number | null;
  focus_action_id: number | null;
  primary_ids: number[] | null;
  secondary_ids: number[] | null;
  target: BattleCmdTarget | null;
}

interface BattleUnitWeakness {
  element: string;
  resistance: string;
  analyzed: boolean;
}

interface BattleUnit {
  uid: number | null;
  unit_id: number | null;
  name: string | null;
  relation: number | null;
  is_boss: boolean | null;
  is_dead: boolean | null;
  hp: number | null;
  max_hp: number | null;
  mp: number | null;
  max_mp: number | null;
  bp: number | null;
  max_bp: number | null;
  lb: number | null;
  lb_max: number | null;
  lb_level: number | null;
  is_break: boolean | null;
  is_risky: boolean | null;
  scheduled_skill_id: number | null;
  in_action: boolean | null;
  in_ex_phase: boolean | null;
  target_state: string | null;
  weaknesses: BattleUnitWeakness[] | null;
}

interface BattleResultItem {
  item_id: number;
  num: number;
}

interface BattleResult {
  page_index: number | null;
  total_exp: number | null;
  total_money: number | null;
  total_ap: number | null;
  enemy_ids: number[] | null;
  items: BattleResultItem[] | null;
}

interface BattleState {
  in_battle: boolean | null;
  task_class: string | null;
  in_talk_event: boolean | null;
  in_tutorial: boolean | null;
  phase: string | null;
  is_event: boolean | null;
  is_ui_visible: boolean | null;
  working_job_count: number | null;
  current_actor_uid: number | null;
  timeline: number[] | null;
  risk_uids: number[] | null;
  cmd: BattleCmd | null;
  units: BattleUnit[] | null;
  result: BattleResult | null;
}

interface RecordedError {
  field: string;
  message: string;
}

const FAKE_BATTLE_HELPER = `
  local json = require("ffrbridge.json")
  local battle = require("ffrbridge.topics.battle")

  local function makeObj(props, classFullName)
    local obj = { _valid = true }
    function obj:IsValid() return self._valid end
    if classFullName then
      local cls = {
        IsValid = function() return true end,
        GetFullName = function() return classFullName end,
      }
      function obj:GetClass() return cls end
    end
    if props then
      for k, v in pairs(props) do obj[k] = v end
    end
    return obj
  end

  local function makeText(str)
    return { ToString = function() return str end }
  end

  local function makeTArray(items)
    local arr = {}
    for i = 1, #items do arr[i] = items[i] end
    function arr:GetArrayNum() return #items end
    return arr
  end

  local function wrapParam(v)
    return { get = function() return v end }
  end

  local function makeTMap(pairsList)
    local mapObj = {}
    function mapObj:ForEach(cb)
      for _, pair in ipairs(pairsList) do
        local stop = cb(wrapParam(pair[1]), wrapParam(pair[2]))
        if stop then break end
      end
    end
    return mapObj
  end

  local function makeActionKeys(ids)
    local items = {}
    for i = 1, #ids do
      items[i] = { Category = 1, ID = ids[i] }
    end
    return makeTArray(items)
  end

  local function makePointVm(cur, max)
    return makeObj({ CurrentPoint = cur, MaxPoint = max })
  end

  local function makeLimitVm(cur, max, lv)
    return makeObj({ CurrentPoint = cur, MaxPoint = max, CurrentLevel = lv })
  end

  local function makeActionVm(inAction, inExPhase)
    return makeObj({ bIsInAction = inAction, bIsInExPhase = inExPhase })
  end

  local function makeUnitVm(spec)
    local ai = spec.ai
    if ai == nil and spec.scheduledSkillId ~= nil then
      ai = makeObj({ m_ScheduledSkillId = spec.scheduledSkillId })
    end
    local owner = spec.owner
    if owner == nil and ai ~= nil then
      owner = makeObj({ m_pAI = ai })
    end
    local param = spec.param
    if param == nil then
      param = makeObj({
        m_pOwnerBtlUnit = owner,
        m_ContEffectList = makeTArray(spec.contEffects or {}),
        GetRelation = function() return spec.relation end,
        IsBoss = function() return spec.isBoss == true end,
        IsDead = function() return spec.isDead == true end,
        IsBreak = function() return spec.isBreak == true end,
        GetHitPoint = function() return spec.hp end,
        GetMaxHitPoint = function() return spec.maxHp end,
        GetMagicPoint = function() return spec.mp end,
        GetMaxMagicPoint = function() return spec.maxMp end,
        GetBreakPoint = function() return spec.bp end,
        GetMaxBreakPoint = function() return spec.maxBp end,
      })
    end
    local vm = makeObj({
      UniqueID = spec.uid,
      UnitId = spec.unitId,
      ShortName = makeText(spec.name),
      bIsVisible = spec.visible ~= false,
      BtlUnitParam = param,
      HitPointVM = spec.hpVm ~= nil and spec.hpVm or makePointVm(spec.hp, spec.maxHp),
      MagicPointVM = spec.mpVm ~= nil and spec.mpVm or makePointVm(spec.mp, spec.maxMp),
      BreakPointVM = spec.bpVm ~= nil and spec.bpVm or makePointVm(spec.bp, spec.maxBp),
      LimitPointVM = spec.lbVm ~= nil and spec.lbVm or makeLimitVm(spec.lb or 0, spec.lbMax or 100, spec.lbLevel or 0),
      ActionVM = spec.actionVm ~= nil and spec.actionVm or makeActionVm(spec.inAction == true, spec.inExPhase == true),
      TargetSelectState = spec.targetState or 0,
      ElemResistDataList = makeTArray(spec.resists or {}),
      IsScheduledHighRiskSkill = spec.isRiskyFn or function() return spec.isRisky == true end,
    })
    return vm
  end

  local function makeCtx(registry, opts)
    opts = opts or {}
    local finds = {}
    local findAllCalls = 0
    local errors = {}
    local ctx = {
      cache = opts.cache or {},
      clockMs = opts.clockMs or 1000,
      tick = opts.tick or 1,
      world = opts.world,
      map = opts.map,
      find = function(cls)
        finds[#finds + 1] = cls
        local obj = registry[cls]
        if obj == nil then return nil end
        if type(obj.IsValid) == "function" and not obj:IsValid() then return nil end
        return obj
      end,
      findAll = function()
        findAllCalls = findAllCalls + 1
        return {}
      end,
      err = function(field, message)
        errors[#errors + 1] = { field = field, message = message }
      end,
    }
    return ctx, {
      finds = finds,
      getFindAllCalls = function() return findAllCalls end,
      errors = errors,
    }
  end

  local function makeHealthyBattleRegistry()
    local rainVm = makeUnitVm({
      uid = 0,
      unitId = 1001,
      name = "雷因",
      relation = 1,
      isBoss = false,
      isDead = false,
      hp = 270,
      maxHp = 270,
      mp = 30,
      maxMp = 35,
      bp = 0,
      maxBp = 0,
      lb = 25,
      lbMax = 100,
      lbLevel = 0,
      inAction = false,
      inExPhase = true,
      targetState = 0,
    })
    local lasswellVm = makeUnitVm({
      uid = 1,
      unitId = 1002,
      name = "拉斯韦尔",
      relation = 1,
      isBoss = false,
      isDead = false,
      hp = 126,
      maxHp = 336,
      mp = 45,
      maxMp = 50,
      bp = 0,
      maxBp = 0,
      lb = 100,
      lbMax = 100,
      lbLevel = 1,
      inAction = true,
      inExPhase = true,
      targetState = 0,
    })
    local bossVm = makeUnitVm({
      uid = 2,
      unitId = 401,
      name = "贝尔摩达",
      relation = 2,
      isBoss = true,
      isDead = false,
      hp = 1420,
      maxHp = 2000,
      mp = 0,
      maxMp = 0,
      bp = 0,
      maxBp = 145,
      lb = 0,
      lbMax = 100,
      lbLevel = 0,
      isBreak = true,
      isRisky = false,
      scheduledSkillId = 505040,
      inAction = false,
      inExPhase = false,
      targetState = 1,
      resists = {
        { element = 0, resistance = 0, bIsAnalyzed = false },
        { element = 8, resistance = 1, bIsAnalyzed = false },
        { element = 1, resistance = 1, bIsAnalyzed = true },
        { element = 5, resistance = 2, bIsAnalyzed = false },
        { element = 2, resistance = 0, bIsAnalyzed = true },
      },
    })

    local cmdMgr = makeObj({
      CmdType = 1,
      CmdState = 0,
      ActionUnitUniqueIdList = makeTArray({ 1, 0 }),
    })
    -- Intentionally pass unsorted TMap order (2, 0, 1) to verify uid sorting.
    local unitMgr = makeObj({
      UnitParamVMMap = makeTMap({
        { 2, bossVm },
        { 0, rainVm },
        { 1, lasswellVm },
      }),
    })
    local pageLayout = makeObj({ CurrentPageIndex = 1 })
    local resWidget = makeObj({
      IsFinishPage = true,
      WBP_BtlResultPage_Layout = pageLayout,
    })
    local uiBattle = makeObj({
      bDuringBattle = true,
      BtlPhaseType = 1,
      CurrentActionUnitUniqueId = 1,
      BtlCmdManager = cmdMgr,
      BtlUnitManager = unitMgr,
      ResultWidget = resWidget,
    })
    local taskMgr = makeObj({
      m_pTask = makeObj({}, "Class /Script/mucha.CPP_BtlTaskAdditionalTurn"),
    })
    local talkSub = makeObj({ m_pEventManager = nil })
    local tutSub = makeObj({ m_TutorialWidget = nil })
    local vmBattle = makeObj({
      bIsEvent = false,
      bIsDisplayBattleWidget = true,
    })
    local jobMgr = makeObj({
      m_WorkingJobList = makeTArray({}),
      m_WaitingJobList = makeTArray({}),
    })
    local btlMgr = makeObj({ m_pBattleJobManager = jobMgr })
    local vmTimeline = makeObj({
      TimelineUnified = { UnifiedIds = makeTArray({ 1, 0, 1, 2, 0 }) },
      RiskUnitUniqueIdList = makeTArray({ 2 }),
    })
    local unitSelectVm = makeObj({
      CmdValue = { ActionKey = { Category = 1, ID = 220010 }, IsAll = false },
      FocusBaseUniqueId = 2,
      targetUniqueIdList = makeTArray({ 2 }),
      AllTargetUniqueIdList = makeTArray({ 0, 1, 2 }),
      bCanBeAoE = true,
    })
    local vmCmd = makeObj({
      bDisplayCmd = true,
      AncestorIdList = makeTArray({ 10 }),
      FocusCmdData = { EntryIndex = 0, Value = { ActionKey = { Category = 1, ID = 402040 } } },
      UnitSelectVM = unitSelectVm,
    })
    local primaryList = makeObj({ WBP_List = makeObj({ ActionKeyList = makeActionKeys({ 402010, 1, 400010, 7, 400020 }) }) })
    local secondaryList = makeObj({ WBP_List = makeObj({ ActionKeyList = makeActionKeys({ 402040 }) }) })
    local contentWidget = makeObj({
      WBP_BtlCmd_PrimaryList = primaryList,
      WBP_BtlCmd_SecondaryList = secondaryList,
    })
    local battleLayout = makeObj({
      WBP_BtlCmdLayout = makeObj({ ContentWidget = contentWidget }),
    })
    local vmResult = makeObj({
      BtlResultData = {
        totalExperience = 300,
        totalMoney = 750,
        totalAp = 45,
        enemyDataList = makeTArray({ 401 }),
        itemDataList = makeTArray({ { ItemId = 4, Num = 1 } }),
      },
    })

    return {
      CPP_UI_BattleSubsystem = uiBattle,
      CPP_TaskManager = taskMgr,
      CPP_TalkEventSubsystem = talkSub,
      CPP_TutorialSubsystem = tutSub,
      CPP_VM_Battle = vmBattle,
      CPP_BtlManager = btlMgr,
      VM_Timeline_C = vmTimeline,
      VM_BtlCmd_C = vmCmd,
      WBP_BattleLayout_C = battleLayout,
      CPP_VM_BattleResult = vmResult,
    }
  end
`;

void test("out of battle reads only 4 resident singletons and leaves battle-only fields null", () => {
  const result = luaJson<{
    idle: BattleState;
    idleFinds: string[];
    idleErrors: RecordedError[];
    idleFindAll: number;
    eventAndTutorial: BattleState;
    staleEventAndTutorial: BattleState;
  }>(`
    ${FAKE_BATTLE_HELPER}
    local reg = {
      CPP_UI_BattleSubsystem = makeObj({ bDuringBattle = false }),
      CPP_TaskManager = makeObj({ m_pTask = makeObj({}, "Class /Script/mucha.CPP_MapTaskMain") }),
      CPP_TalkEventSubsystem = makeObj({ m_pEventManager = nil }),
      CPP_TutorialSubsystem = makeObj({ m_TutorialWidget = nil }),
    }
    local ctx1, stats1 = makeCtx(reg)
    local idle = battle.collect(ctx1)

    reg.CPP_TalkEventSubsystem.m_pEventManager = makeObj({})
    reg.CPP_TutorialSubsystem.m_TutorialWidget = makeObj({})
    local ctx2 = makeCtx(reg, { cache = ctx1.cache, tick = 2 })
    local eventAndTutorial = battle.collect(ctx2)

    reg.CPP_TalkEventSubsystem.m_pEventManager._valid = false
    reg.CPP_TutorialSubsystem.m_TutorialWidget._valid = false
    local ctx3 = makeCtx(reg, { cache = ctx1.cache, tick = 3 })
    local staleEventAndTutorial = battle.collect(ctx3)

    return {
      idle = idle,
      idleFinds = json.array(stats1.finds),
      idleErrors = json.array(stats1.errors),
      idleFindAll = stats1.getFindAllCalls(),
      eventAndTutorial = eventAndTutorial,
      staleEventAndTutorial = staleEventAndTutorial,
    }
  `);

  assert.deepEqual(result.idle, {
    in_battle: false,
    task_class: "CPP_MapTaskMain",
    in_talk_event: false,
    in_tutorial: false,
    phase: null,
    is_event: null,
    is_ui_visible: null,
    working_job_count: null,
    current_actor_uid: null,
    timeline: null,
    risk_uids: null,
    cmd: null,
    units: null,
    result: null,
  });
  assert.deepEqual(result.idleFinds, [
    "CPP_UI_BattleSubsystem",
    "CPP_TaskManager",
    "CPP_TalkEventSubsystem",
    "CPP_TutorialSubsystem",
  ]);
  assert.deepEqual(result.idleErrors, []);
  assert.equal(result.idleFindAll, 0);

  assert.equal(result.eventAndTutorial.in_talk_event, true);
  assert.equal(result.eventAndTutorial.in_tutorial, true);
  assert.equal(result.staleEventAndTutorial.in_talk_event, false);
  assert.equal(result.staleEventAndTutorial.in_tutorial, false);
});

void test("in-battle normal output covers Additional phase, List submenus, units, weaknesses, risk_uids, UTF-8 truncation, and integer JSON", () => {
  const raw = runLua(
    `
      ${FAKE_BATTLE_HELPER}
      local reg = makeHealthyBattleRegistry()
      -- Also verify long UTF-8 name truncation (>200 Chinese chars) and root list (#ancestors == 0).
      local longName = string.rep("炎", 205)
      local rainVm = makeUnitVm({
        uid = 0,
        unitId = 1001,
        name = longName,
        relation = 1,
        hp = 270.0,
        maxHp = 270.0,
        mp = 30.0,
        maxMp = 35.0,
        bp = 0.0,
        maxBp = 0.0,
        lb = 25.0,
        lbMax = 100.0,
        lbLevel = 0.0,
        inAction = false,
        inExPhase = true,
      })
      local bossVm = nil
      local lasswellVm = nil
      reg.CPP_UI_BattleSubsystem.BtlUnitManager.UnitParamVMMap:ForEach(function(k, v)
        local key = k:get()
        if key == 1 then lasswellVm = v:get() end
        if key == 2 then bossVm = v:get() end
      end)
      reg.CPP_UI_BattleSubsystem.BtlUnitManager.UnitParamVMMap = makeTMap({
        { 2, bossVm },
        { 0, rainVm },
        { 1, lasswellVm },
      })
      reg.CPP_BtlManager.m_pBattleJobManager.m_WorkingJobList = makeTArray({ makeObj({}) })
      reg.CPP_BtlManager.m_pBattleJobManager.m_WaitingJobList = makeTArray({ makeObj({}), makeObj({}) })

      local ctx1, stats1 = makeCtx(reg)
      local additional = battle.collect(ctx1)

      -- Switch to root List (#ancestors == 0) in Normal phase
      reg.CPP_UI_BattleSubsystem.BtlPhaseType = 0
      reg.CPP_UI_BattleSubsystem.BtlCmdManager.CmdType = 0
      reg.VM_BtlCmd_C.AncestorIdList = makeTArray({})
      reg.VM_BtlCmd_C.FocusCmdData = { EntryIndex = 1, Value = { ActionKey = { Category = 1, ID = 1 } } }
      local ctx2, stats2 = makeCtx(reg, { cache = ctx1.cache, tick = 2 })
      local rootList = battle.collect(ctx2)

      return {
        additional = additional,
        rootList = rootList,
        errors1 = json.array(stats1.errors),
        errors2 = json.array(stats2.errors),
        findAll1 = stats1.getFindAllCalls(),
      }
    `,
    true,
  );

  // Integer fields must not serialize with ".0" suffix.
  assert.doesNotMatch(raw, /"(hp|max_hp|mp|max_mp|unit_id|uid|working_job_count)":\d+\.0\b/u);

  const parsed = JSON.parse(raw) as {
    additional: BattleState;
    rootList: BattleState;
    errors1: RecordedError[];
    errors2: RecordedError[];
    findAll1: number;
  };

  assert.deepEqual(parsed.errors1, []);
  assert.deepEqual(parsed.errors2, []);
  assert.equal(parsed.findAll1, 0);

  const st = parsed.additional;
  assert.equal(st.in_battle, true);
  assert.equal(st.task_class, "CPP_BtlTaskAdditionalTurn");
  assert.equal(st.in_talk_event, false);
  assert.equal(st.in_tutorial, false);
  assert.equal(st.phase, "Additional");
  assert.equal(st.is_event, false);
  assert.equal(st.is_ui_visible, true);
  assert.equal(st.working_job_count, 3);
  assert.equal(st.current_actor_uid, 1);
  assert.deepEqual(st.timeline, [1, 0, 1, 2, 0]);
  assert.deepEqual(st.risk_uids, [2]);
  assert.equal(st.result, null);

  assert.deepEqual(st.cmd, {
    display: true,
    type: "Additional",
    state: "List",
    ancestors: [10],
    focus_index: 0,
    focus_action_id: 402040,
    primary_ids: [402010, 1, 400010, 7, 400020],
    secondary_ids: [402040],
    target: null,
  });

  // Units are sorted by uid ascending: 0, 1, 2.
  assert.ok(st.units);
  assert.equal(st.units.length, 3);
  const [unit0, unit1, unit2] = st.units;
  assert.ok(unit0);
  assert.ok(unit1);
  assert.ok(unit2);
  assert.equal(unit0.uid, 0);
  assert.equal(unit1.uid, 1);
  assert.equal(unit2.uid, 2);

  // 205-char Chinese string truncated to 199 chars + "…" = 200 Unicode characters.
  assert.equal(unit0.name, `${"炎".repeat(199)}…`);
  assert.equal(unit0.scheduled_skill_id, null);
  assert.equal(unit0.weaknesses, null);

  assert.deepEqual(unit1, {
    uid: 1,
    unit_id: 1002,
    name: "拉斯韦尔",
    relation: 1,
    is_boss: false,
    is_dead: false,
    hp: 126,
    max_hp: 336,
    mp: 45,
    max_mp: 50,
    bp: 0,
    max_bp: 0,
    lb: 100,
    lb_max: 100,
    lb_level: 1,
    is_break: false,
    is_risky: false,
    scheduled_skill_id: null,
    in_action: true,
    in_ex_phase: true,
    target_state: "None",
    weaknesses: null,
  });

  // Boss unit: is_risky true via risk_uids union, None resistances filtered out.
  assert.deepEqual(unit2, {
    uid: 2,
    unit_id: 401,
    name: "贝尔摩达",
    relation: 2,
    is_boss: true,
    is_dead: false,
    hp: 1420,
    max_hp: 2000,
    mp: 0,
    max_mp: 0,
    bp: 0,
    max_bp: 145,
    lb: 0,
    lb_max: 100,
    lb_level: 0,
    is_break: true,
    is_risky: true,
    scheduled_skill_id: 505040,
    in_action: false,
    in_ex_phase: false,
    target_state: "Main",
    weaknesses: [
      { element: "Dark", resistance: "Weakness", analyzed: false },
      { element: "Fire", resistance: "Weakness", analyzed: true },
      { element: "Thunder", resistance: "Halved", analyzed: false },
    ],
  });

  // Root list (#ancestors == 0) does not populate secondary_ids.
  assert.equal(parsed.rootList.phase, "Normal");
  assert.deepEqual(parsed.rootList.cmd, {
    display: true,
    type: "Normal",
    state: "List",
    ancestors: [],
    focus_index: 1,
    focus_action_id: 1,
    primary_ids: [402010, 1, 400010, 7, 400020],
    secondary_ids: null,
    target: null,
  });
});

void test("target selection mode populates cmd.target and skips WBP_BattleLayout_C", () => {
  const result = luaJson<{
    state: BattleState;
    finds: string[];
    errors: RecordedError[];
  }>(`
    ${FAKE_BATTLE_HELPER}
    local reg = makeHealthyBattleRegistry()
    reg.CPP_UI_BattleSubsystem.BtlCmdManager.CmdState = 1 -- Target
    reg.VM_BtlCmd_C.AncestorIdList = makeTArray({ 1, 220010 })
    reg.VM_BtlCmd_C.UnitSelectVM.CmdValue = { ActionKey = { Category = 1, ID = 220010 }, IsAll = true }
    reg.VM_BtlCmd_C.UnitSelectVM.FocusBaseUniqueId = 2
    reg.VM_BtlCmd_C.UnitSelectVM.targetUniqueIdList = makeTArray({ 2, 3 })
    reg.VM_BtlCmd_C.UnitSelectVM.AllTargetUniqueIdList = makeTArray({ 0, 1, 2, 3 })
    reg.VM_BtlCmd_C.UnitSelectVM.bCanBeAoE = true
    reg.WBP_BattleLayout_C = nil

    local ctx, stats = makeCtx(reg)
    local st = battle.collect(ctx)
    return {
      state = st,
      finds = json.array(stats.finds),
      errors = json.array(stats.errors),
    }
  `);

  assert.deepEqual(result.errors, []);
  assert.ok(!result.finds.includes("WBP_BattleLayout_C"));
  assert.deepEqual(result.state.cmd, {
    display: true,
    type: "Additional",
    state: "Target",
    ancestors: [1, 220010],
    focus_index: 0,
    focus_action_id: 402040,
    primary_ids: null,
    secondary_ids: null,
    target: {
      skill_id: 220010,
      focus_uid: 2,
      target_uids: [2, 3],
      all_target_uids: [0, 1, 2, 3],
      can_aoe: true,
      is_all: true,
    },
  });
});

void test("victory settlement reads battle.result only when CPP_BtlTaskWin and IsFinishPage == false", () => {
  const result = luaJson<{
    activeResult: BattleState;
    finishedResult: BattleState;
    finishedFinds: string[];
    wrongTaskResult: BattleState;
    errorsActive: RecordedError[];
    errorsFinished: RecordedError[];
  }>(`
    ${FAKE_BATTLE_HELPER}
    local reg = makeHealthyBattleRegistry()
    reg.CPP_TaskManager.m_pTask = makeObj({}, "Class /Script/mucha.CPP_BtlTaskWin")
    reg.CPP_UI_BattleSubsystem.ResultWidget.IsFinishPage = false
    reg.CPP_UI_BattleSubsystem.ResultWidget.WBP_BtlResultPage_Layout.CurrentPageIndex = 0
    reg.VM_BtlCmd_C.bDisplayCmd = false

    local ctx1, stats1 = makeCtx(reg)
    local activeResult = battle.collect(ctx1)

    -- Pooled ResultWidget with IsFinishPage = true must return result = null without querying VM
    local reg2 = makeHealthyBattleRegistry()
    reg2.CPP_TaskManager.m_pTask = makeObj({}, "Class /Script/mucha.CPP_BtlTaskWin")
    reg2.CPP_UI_BattleSubsystem.ResultWidget.IsFinishPage = true
    reg2.VM_BtlCmd_C.bDisplayCmd = false
    local ctx2, stats2 = makeCtx(reg2)
    local finishedResult = battle.collect(ctx2)

    -- Even if IsFinishPage = false, non-Win task must not emit result
    local reg3 = makeHealthyBattleRegistry()
    reg3.CPP_TaskManager.m_pTask = makeObj({}, "Class /Script/mucha.CPP_BtlTaskCommand")
    reg3.CPP_UI_BattleSubsystem.ResultWidget.IsFinishPage = false
    reg3.VM_BtlCmd_C.bDisplayCmd = false
    local ctx3 = makeCtx(reg3)
    local wrongTaskResult = battle.collect(ctx3)

    return {
      activeResult = activeResult,
      finishedResult = finishedResult,
      finishedFinds = json.array(stats2.finds),
      wrongTaskResult = wrongTaskResult,
      errorsActive = json.array(stats1.errors),
      errorsFinished = json.array(stats2.errors),
    }
  `);

  assert.deepEqual(result.errorsActive, []);
  assert.deepEqual(result.errorsFinished, []);
  assert.deepEqual(result.activeResult.result, {
    page_index: 0,
    total_exp: 300,
    total_money: 750,
    total_ap: 45,
    enemy_ids: [401],
    items: [{ item_id: 4, num: 1 }],
  });
  assert.equal(result.finishedResult.result, null);
  assert.ok(!result.finishedFinds.includes("CPP_VM_BattleResult"));
  assert.equal(result.wrongTaskResult.result, null);
});

void test("unit fallbacks use C++ getters, ContFxBreak scan, bIsVisible, and risk_uids when primary paths are unavailable", () => {
  const result = luaJson<{
    units: BattleUnit[] | null;
    errors: RecordedError[];
  }>(`
    ${FAKE_BATTLE_HELPER}
    local reg = makeHealthyBattleRegistry()
    local staleVm = makeObj({})
    staleVm._valid = false

    local contBreak = makeObj({}, "Class /Script/mucha.CPP_ContFxBreak")
    local fallbackParam = makeObj({
      m_pOwnerBtlUnit = makeObj({ m_pAI = makeObj({ m_ScheduledSkillId = 500020 }) }),
      m_ContEffectList = makeTArray({ contBreak }),
      GetRelation = function() return 2 end,
      IsBoss = function() return false end,
      -- Omit IsDead and IsBreak to trigger fallbacks
      GetHitPoint = function() return 18 end,
      GetMaxHitPoint = function() return 68 end,
      GetMagicPoint = function() return 5 end,
      GetMaxMagicPoint = function() return 10 end,
      GetBreakPoint = function() return 0 end,
      GetMaxBreakPoint = function() return 50 end,
    })

    local enemyVm = makeObj({
      UniqueID = 2,
      UnitId = 2,
      ShortName = "钢蝠",
      bIsVisible = false,
      BtlUnitParam = fallbackParam,
      HitPointVM = staleVm,
      MagicPointVM = staleVm,
      BreakPointVM = staleVm,
      LimitPointVM = makeLimitVm(0, 100, 0),
      ActionVM = makeActionVm(false, false),
      TargetSelectState = 2,
      ElemResistDataList = makeTArray({}),
      -- Omit IsScheduledHighRiskSkill to fall back to risk_uids ([2])
    })

    -- Plain Lua table map (no ForEach) also supported
    reg.CPP_UI_BattleSubsystem.BtlUnitManager.UnitParamVMMap = { [2] = enemyVm }
    reg.VM_BtlCmd_C.bDisplayCmd = false

    local ctx, stats = makeCtx(reg)
    local st = battle.collect(ctx)
    return {
      units = st.units,
      errors = json.array(stats.errors),
    }
  `);

  assert.deepEqual(result.errors, []);
  assert.deepEqual(result.units, [
    {
      uid: 2,
      unit_id: 2,
      name: "钢蝠",
      relation: 2,
      is_boss: false,
      is_dead: true,
      hp: 18,
      max_hp: 68,
      mp: 5,
      max_mp: 10,
      bp: 0,
      max_bp: 50,
      lb: 0,
      lb_max: 100,
      lb_level: 0,
      is_break: true,
      is_risky: true,
      scheduled_skill_id: 500020,
      in_action: false,
      in_ex_phase: false,
      target_state: "Sub",
      weaknesses: [],
    },
  ]);
});

void test("missing/stale objects and property errors record errors without breaking sibling fields", () => {
  const result = luaJson<{
    missingAll: BattleState;
    missingAllErrors: RecordedError[];
    partialBroken: BattleState;
    partialErrors: RecordedError[];
  }>(`
    ${FAKE_BATTLE_HELPER}
    -- Case 1: all resident singletons missing
    local ctxEmpty, statsEmpty = makeCtx({})
    local missingAll = battle.collect(ctxEmpty)

    -- Case 2: in battle with broken properties, stale objects, unknown enum, and throwing unit getter
    local reg = makeHealthyBattleRegistry()
    reg.CPP_UI_BattleSubsystem.BtlPhaseType = 99 -- invalid enum
    reg.CPP_VM_Battle = setmetatable({
      IsValid = function() return true end,
      bIsDisplayBattleWidget = true,
    }, {
      __index = function(_, key)
        if key == "bIsEvent" then error("read bIsEvent failed") end
        return nil
      end,
    })
    reg.CPP_BtlManager.m_pBattleJobManager._valid = false
    reg.VM_Timeline_C = nil

    -- Unit 0 healthy, Unit 2 dead enemy with nil AI (no spurious error), Unit 3 throwing ShortName
    local deadEnemy = makeUnitVm({
      uid = 2,
      unitId = 2,
      name = "钢蝠",
      relation = 2,
      isDead = true,
      hp = 0,
      maxHp = 68,
      mp = 0,
      maxMp = 0,
      bp = 0,
      maxBp = 50,
      param = makeObj({
        m_pOwnerBtlUnit = nil,
        GetRelation = function() return 2 end,
        IsBoss = function() return false end,
        IsDead = function() return true end,
        IsBreak = function() return false end,
      }),
    })
    local brokenUnit = makeUnitVm({
      uid = 3,
      unitId = 2,
      name = "钢蝠",
      relation = 2,
      hp = 50,
      maxHp = 68,
      mp = 0,
      maxMp = 0,
      bp = 50,
      maxBp = 50,
      scheduledSkillId = 500020,
    })
    brokenUnit.ShortName = { ToString = function() error("ftext broken") end }

    local healthyRain = makeUnitVm({
      uid = 0,
      unitId = 1001,
      name = "雷因",
      relation = 1,
      hp = 200,
      maxHp = 270,
      mp = 20,
      maxMp = 35,
      bp = 0,
      maxBp = 0,
    })
    reg.CPP_UI_BattleSubsystem.BtlUnitManager.UnitParamVMMap = makeTMap({
      { 0, healthyRain },
      { 2, deadEnemy },
      { 3, brokenUnit },
    })
    reg.VM_BtlCmd_C.bDisplayCmd = false

    local ctxBroken, statsBroken = makeCtx(reg)
    local partialBroken = battle.collect(ctxBroken)

    return {
      missingAll = missingAll,
      missingAllErrors = json.array(statsEmpty.errors),
      partialBroken = partialBroken,
      partialErrors = json.array(statsBroken.errors),
    }
  `);

  assert.equal(result.missingAll.in_battle, null);
  assert.equal(result.missingAll.task_class, null);
  assert.equal(result.missingAll.in_talk_event, null);
  assert.equal(result.missingAll.in_tutorial, null);
  assert.equal(result.missingAll.phase, null);
  assert.equal(result.missingAllErrors.length, 4);

  const st = result.partialBroken;
  assert.equal(st.in_battle, true);
  assert.equal(st.phase, null);
  assert.equal(st.is_event, null);
  assert.equal(st.is_ui_visible, true);
  assert.equal(st.working_job_count, null);
  assert.equal(st.timeline, null);
  assert.equal(st.risk_uids, null);
  assert.equal(st.current_actor_uid, 1);

  assert.ok(st.units);
  assert.equal(st.units.length, 3);
  const [brokenU0, brokenU1, brokenU2] = st.units;
  assert.ok(brokenU0);
  assert.ok(brokenU1);
  assert.ok(brokenU2);
  assert.equal(brokenU0.name, "雷因");
  assert.equal(brokenU0.hp, 200);
  // Dead enemy with nil m_pOwnerBtlUnit produces scheduled_skill_id = null without error.
  assert.equal(brokenU1.is_dead, true);
  assert.equal(brokenU1.scheduled_skill_id, null);
  // Broken unit keeps other fields intact while name becomes null.
  assert.equal(brokenU2.uid, 3);
  assert.equal(brokenU2.name, null);
  assert.equal(brokenU2.hp, 50);

  const errFields = result.partialErrors.map((entry) => entry.field);
  assert.ok(errFields.includes("battle.phase"));
  assert.ok(errFields.includes("battle.is_event"));
  assert.ok(errFields.includes("battle.working_job_count"));
  assert.ok(errFields.includes("battle.timeline"));
  assert.ok(errFields.includes("battle.units[3].name"));
  assert.ok(!errFields.includes("battle.units[2].scheduled_skill_id"));
});

void test("singleton cache reuses objects across ticks and invalidates on IsValid false, map change, or battle exit/re-enter", () => {
  const result = luaJson<{
    tick1Finds: string[];
    tick2Finds: string[];
    tick3Finds: string[];
    tick4Finds: string[];
    tick5Finds: string[];
    tick6Finds: string[];
    tick7Finds: string[];
    tick8Finds: string[];
    tick5Event: boolean | null;
    tick8Actor: number | null;
  }>(`
    ${FAKE_BATTLE_HELPER}
    local reg = makeHealthyBattleRegistry()
    reg.CPP_UI_BattleSubsystem.bDuringBattle = false
    local sharedCache = {}

    -- Tick 1: out of battle (cold) -> 4 resident lookups
    local ctx1, s1 = makeCtx(reg, { cache = sharedCache, tick = 1, map = "Dng_01Gra_40_PL" })
    battle.collect(ctx1)

    -- Tick 2: out of battle (warm) -> 0 lookups
    local ctx2, s2 = makeCtx(reg, { cache = sharedCache, tick = 2, map = "Dng_01Gra_40_PL" })
    battle.collect(ctx2)

    -- Tick 3: enter battle (List open) -> only 5 battle singletons looked up, resident 4 reused
    reg.CPP_UI_BattleSubsystem.bDuringBattle = true
    local ctx3, s3 = makeCtx(reg, { cache = sharedCache, tick = 3, map = "Dng_01Gra_40_PL" })
    battle.collect(ctx3)

    -- Tick 4: stay in battle (warm) -> 0 lookups
    local ctx4, s4 = makeCtx(reg, { cache = sharedCache, tick = 4, map = "Dng_01Gra_40_PL" })
    battle.collect(ctx4)

    -- Tick 5: invalidate CPP_VM_Battle -> only CPP_VM_Battle re-looked up
    reg.CPP_VM_Battle._valid = false
    reg.CPP_VM_Battle = makeObj({ bIsEvent = true, bIsDisplayBattleWidget = false })
    local ctx5, s5 = makeCtx(reg, { cache = sharedCache, tick = 5, map = "Dng_01Gra_40_PL" })
    local st5 = battle.collect(ctx5)

    -- Tick 6: map change -> all needed singletons re-looked up
    local ctx6, s6 = makeCtx(reg, { cache = sharedCache, tick = 6, map = "Dng_01Gra_41_PL" })
    battle.collect(ctx6)

    -- Tick 7: exit battle -> 0 lookups (resident singletons stay cached; battle singletons evicted)
    reg.CPP_UI_BattleSubsystem.bDuringBattle = false
    local ctx7, s7 = makeCtx(reg, { cache = sharedCache, tick = 7, map = "Dng_01Gra_41_PL" })
    battle.collect(ctx7)

    -- Tick 8: enter new battle -> fresh battle singletons looked up, resident 4 still reused
    reg.CPP_UI_BattleSubsystem.bDuringBattle = true
    reg.CPP_UI_BattleSubsystem.CurrentActionUnitUniqueId = 0
    local ctx8, s8 = makeCtx(reg, { cache = sharedCache, tick = 8, map = "Dng_01Gra_41_PL" })
    local st8 = battle.collect(ctx8)

    return {
      tick1Finds = json.array(s1.finds),
      tick2Finds = json.array(s2.finds),
      tick3Finds = json.array(s3.finds),
      tick4Finds = json.array(s4.finds),
      tick5Finds = json.array(s5.finds),
      tick6Finds = json.array(s6.finds),
      tick7Finds = json.array(s7.finds),
      tick8Finds = json.array(s8.finds),
      tick5Event = st5.is_event,
      tick8Actor = st8.current_actor_uid,
    }
  `);

  assert.equal(result.tick1Finds.length, 4);
  assert.deepEqual(result.tick2Finds, []);
  assert.deepEqual(result.tick3Finds, [
    "CPP_VM_Battle",
    "CPP_BtlManager",
    "VM_Timeline_C",
    "VM_BtlCmd_C",
    "WBP_BattleLayout_C",
  ]);
  assert.deepEqual(result.tick4Finds, []);
  assert.deepEqual(result.tick5Finds, ["CPP_VM_Battle"]);
  assert.equal(result.tick5Event, true);
  assert.equal(result.tick6Finds.length, 9);
  assert.deepEqual(result.tick7Finds, []);
  assert.deepEqual(result.tick8Finds, [
    "CPP_VM_Battle",
    "CPP_BtlManager",
    "VM_Timeline_C",
    "VM_BtlCmd_C",
    "WBP_BattleLayout_C",
  ]);
  assert.equal(result.tick8Actor, 0);
});
