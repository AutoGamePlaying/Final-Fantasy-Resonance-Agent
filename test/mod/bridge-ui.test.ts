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
    to_luastring("bridge-ui-test"),
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

const HELPER_LUA = `
  local ui = require("ffrbridge.topics.ui")
  local json = require("ffrbridge.json")

  local nextAddr = 1000
  local function makeObj(classShort, props, opts)
    nextAddr = nextAddr + 1
    local addr = nextAddr
    local valid = not (opts and opts.valid == false)
    local fullName = (opts and opts.fullName) or (classShort .. " /Engine/Transient." .. classShort .. "_" .. tostring(addr))
    local cls = {
      IsValid = function() return true end,
      GetFullName = function() return "Class /Script/mucha." .. classShort end,
      GetFName = function() return { ToString = function() return classShort end } end,
    }
    local obj = {
      _valid = valid,
      IsValid = function(self) return self._valid == true end,
      GetClass = function() return cls end,
      GetFullName = function() return fullName end,
      GetFName = function() return { ToString = function() return (opts and opts.fname) or classShort end } end,
      GetAddress = function() return addr end,
    }
    if props then
      for k, v in pairs(props) do obj[k] = v end
    end
    return obj
  end

  local function makeText(str)
    return { ToString = function() return str end }
  end

  local function makeTextWidget(str)
    return makeObj("CommonTextBlock", {
      GetText = function() return makeText(str) end,
    })
  end

  local function makeTag(name)
    return { TagName = makeText(name) }
  end

  local function makeTArray(items)
    local arr = {
      GetArrayNum = function() return #items end,
    }
    for i = 1, #items do arr[i] = items[i] end
    return arr
  end

  local function makeTSet(items)
    return {
      ForEach = function(_, cb)
        for _, v in ipairs(items) do
          local stop = cb({ get = function() return makeText(v) end })
          if stop == true then break end
        end
      end,
    }
  end

  local function makeTMap(pairsList)
    return {
      ForEach = function(_, cb)
        for _, pair in ipairs(pairsList) do
          local stop = cb(
            { get = function() return pair[1] end },
            { get = function() return pair[2] end }
          )
          if stop == true then break end
        end
      end,
    }
  end

  local function makeStack(name, activeWidget)
    return makeObj("CPP_EU_ActivatableWidgetStack", {
      _active = activeWidget,
      GetActiveWidget = function(self) return self._active end,
      DisplayedWidget = activeWidget,
    }, { fname = name })
  end

  local function makeBaseRig(overrides)
    local ov = overrides or {}
    local stackGame = makeStack("Stack_Game", ov.gameWidget)
    local stackEvent = makeStack("Stack_Event", ov.eventWidget)
    local stackSystem = makeStack("Stack_System", ov.systemWidget)
    local stackFront1 = makeStack("Stack_System_Front1", ov.front1Widget)
    local stackModal = makeStack("Stack_Modal", ov.modalWidget)

    local layersMap = makeTMap({
      { makeTag("UI.Layer.Game"), stackGame },
      { makeTag("UI.Layer.Event"), stackEvent },
      { makeTag("UI.Layer.System"), stackSystem },
      { makeTag("UI.Layer.System.Front1"), stackFront1 },
      { makeTag("UI.Layer.Modal"), stackModal },
    })

    local activeLayerObj = ov.activeLayerObj
    if ov.activeLayerName == "Game" then activeLayerObj = stackGame
    elseif ov.activeLayerName == "Event" then activeLayerObj = stackEvent
    elseif ov.activeLayerName == "System" then activeLayerObj = stackSystem
    elseif ov.activeLayerName == "System.Front1" then activeLayerObj = stackFront1
    elseif ov.activeLayerName == "Modal" then activeLayerObj = stackModal
    end

    local layout = makeObj("WBP_OverAllLayout_C", {
      Layers = layersMap,
      Stack_Game = stackGame,
      Stack_Event = stackEvent,
      Stack_System = stackSystem,
      Stack_System_Front1 = stackFront1,
      Stack_Modal = stackModal,
      CurrentInputTargetLayer = activeLayerObj,
      GetInputTargetLayer = function() return activeLayerObj end,
    })

    local inputMgr = makeObj("CPP_GameWidgetInputManager", {
      InputBlockSet = { TokenSet = makeTSet(ov.blockedTokens or {}) },
    })
    local dlgMgr = makeObj("CPP_DialogManager", {
      DialogWidget = ov.dialogWidget,
    })
    local loadMgr = makeObj("CPP_LoadingScreenManager", {
      IsShowLoadingScreen = ov.isLoading == true,
    })
    local gwMgr = makeObj("CPP_GameWidgetManagerSubsystem", {
      m_GameLayout = layout,
      m_GameWidgetInputManager = inputMgr,
      m_DialogManager = dlgMgr,
      m_LoadScrManager = loadMgr,
    })
    local guideSub = makeObj("CPP_ActionGuideSubsystem", {
      GuideTag = makeTag(ov.guideTag or "None"),
    })
    local movieSub = makeObj("CPP_MovieSubsystem", {
      m_pMovieScreen = ov.movieScreen,
    })
    local talkSub = makeObj("CPP_TalkEventSubsystem", {
      m_pEventManager = ov.eventManager,
    })
    local tutSub = makeObj("CPP_TutorialSubsystem", {
      m_TutorialWidget = ov.tutorialWidget,
    })
    local taskMgr = makeObj("CPP_TaskManager", {
      m_pTask = ov.taskClass and makeObj(ov.taskClass) or makeObj("CPP_MapTaskMain"),
    })
    local uiBattle = makeObj("CPP_UI_BattleSubsystem", {
      bDuringBattle = ov.inBattle == true,
      ResultWidget = ov.resultWidget,
    })
    local mainMenuSub = makeObj("CPP_MainMenuSubsystem", {
      IsOpenMenu = function() return ov.mainMenuOpen == true end,
      GetMainMenuVM = function() return ov.mainMenuVm end,
      GetLayoutWidget = function() return ov.mainMenuLayout end,
    })
    local world = makeObj("World", {}, { fullName = ov.worldName or "World /Game/Maps/Town01.Town01" })

    local objects = {
      World = world,
      CPP_GameWidgetManagerSubsystem = gwMgr,
      CPP_ActionGuideSubsystem = guideSub,
      CPP_MovieSubsystem = movieSub,
      CPP_TalkEventSubsystem = talkSub,
      CPP_TutorialSubsystem = tutSub,
      CPP_TaskManager = taskMgr,
      CPP_UI_BattleSubsystem = uiBattle,
      CPP_MainMenuSubsystem = mainMenuSub,
    }
    if ov.extraObjects then
      for k, v in pairs(ov.extraObjects) do objects[k] = v end
    end

    local findCounts = {}
    local errors = {}
    local ctx = {
      find = function(clsName)
        findCounts[clsName] = (findCounts[clsName] or 0) + 1
        local o = objects[clsName]
        if o ~= nil and o.IsValid and not o:IsValid() then return nil end
        return o
      end,
      findAll = function()
        error("ui 主题不应调用 ctx.findAll")
      end,
      err = function(field, msg)
        errors[#errors + 1] = field .. ": " .. tostring(msg)
      end,
      cache = {},
      clockMs = 1000,
      tick = 1,
    }

    return {
      ctx = ctx,
      objects = objects,
      findCounts = findCounts,
      errors = errors,
      stacks = {
        game = stackGame,
        event = stackEvent,
        system = stackSystem,
        front1 = stackFront1,
        modal = stackModal,
      },
      subsystems = {
        gwMgr = gwMgr,
        guideSub = guideSub,
        movieSub = movieSub,
        talkSub = talkSub,
        tutSub = tutSub,
        taskMgr = taskMgr,
        uiBattle = uiBattle,
        mainMenuSub = mainMenuSub,
      },
    }
  end
`;

interface UiMovie {
  active: boolean;
  subtitle: string | null;
}

interface UiTalk {
  event_id: string | null;
  window_class: string | null;
  is_cutscene: boolean;
  speaker: string | null;
  text: string | null;
  waiting_key: boolean;
  text_finished: boolean | null;
  can_skip: boolean;
  is_auto: boolean;
}

interface UiChoice {
  source: "dialog" | "event_choice";
  dialog_tag: string | null;
  choice_tag: string | null;
  message: string;
  options: string[];
  focus_index: number | null;
}

interface UiTutorial {
  name: string;
  page_index: number;
  page_count: number;
  caption: string;
  text: string;
  can_close: boolean;
}

interface UiMenu {
  kind: "system_menu" | "main_menu" | "savepoint" | "title";
  scene_stack: string[];
  entries: string[];
  focus_index: number | null;
  sub_state: Record<string, unknown> | null;
}

interface UiShopItem {
  id: number | null;
  name: string | null;
  price: number | null;
  owned: number | null;
  stock: number | null;
  limit: number | null;
}

interface UiShop {
  shop_id: number | null;
  shop_type: string | null;
  scene_stack: string[];
  is_buy: boolean | null;
  category_index: number | null;
  categories: string[];
  cmd_focus_index: number | null;
  item_focus_index: number | null;
  tooltip_item_id: number | null;
  total_amount: number | null;
  items: UiShopItem[];
}

interface UiSaveUi {
  mode: "save" | "load";
  slot_indices: number[];
  focus_index: number | null;
}

interface UiBattleResult {
  page_index: number | null;
  has_level_up_window: boolean;
}

interface UiTopicResult {
  screen_kind: string;
  active_layer: string | null;
  active_widget: string | null;
  action_guide_tag: string | null;
  input_blocked: string[];
  movie: UiMovie | null;
  talk: UiTalk | null;
  choice: UiChoice | null;
  tutorial: UiTutorial | null;
  menu: UiMenu | null;
  shop: UiShop | null;
  save_ui: UiSaveUi | null;
  battle_result: UiBattleResult | null;
}

interface InputStateResult {
  active_imcs: string[];
  swap_decide: boolean | null;
  input_enabled: boolean | null;
  input_blocked: string[];
  key_bindings: Record<string, string | null>;
}

void test("field idle returns 5 routing fields and nulls for all 8 substructures, ignoring pooled inactive widgets", () => {
  const out = luaJson<{ data: UiTopicResult; pooled: UiTopicResult; errors: string[] }>(`
    ${HELPER_LUA}
    local fieldLayout = makeObj("WBP_FieldLayout_C", {
      IsActivated = function() return true end,
      IsOpeningProcess = false,
      IsClosingProcess = false,
    })
    local rig = makeBaseRig({
      gameWidget = fieldLayout,
      activeLayerName = "Game",
      guideTag = "UI.System.ActionGuide.Declare.Field",
      blockedTokens = { "WindowDeactivate" },
    })
    local data = ui.collect(rig.ctx)

    -- CommonUI pooling guard: deactivated or closing widget must NOT surface as active_widget or system_menu.
    local pooledSysMenu = makeObj("WBP_SystemMenuLayout_C", {
      IsActivated = function() return false end,
    })
    local closingModal = makeObj("WBP_DialogLayout_C", {
      IsActivated = function() return true end,
      IsClosingProcess = true,
    })
    local rig2 = makeBaseRig({
      systemWidget = pooledSysMenu,
      modalWidget = closingModal,
      activeLayerName = "Modal",
      guideTag = "None",
    })
    local pooled = ui.collect(rig2.ctx)
    return { data = data, pooled = pooled, errors = json.array(rig.errors) }
  `);

  assert.deepEqual(out.errors, []);
  assert.deepEqual(out.data, {
    screen_kind: "field",
    active_layer: "UI.Layer.Game",
    active_widget: "WBP_FieldLayout_C",
    action_guide_tag: "UI.System.ActionGuide.Declare.Field",
    input_blocked: ["WindowDeactivate"],
    movie: null,
    talk: null,
    choice: null,
    tutorial: null,
    menu: null,
    shop: null,
    save_ui: null,
    battle_result: null,
  });

  assert.equal(out.pooled.screen_kind, "field");
  assert.equal(out.pooled.active_layer, "UI.Layer.Modal");
  assert.equal(out.pooled.active_widget, null);
  assert.equal(out.pooled.action_guide_tag, null);
  assert.equal(out.pooled.menu, null);
});

void test("loading, movie with caption subtitle, and tutorial with priority arbitration over talk/battle", () => {
  const out = luaJson<{
    loading: UiTopicResult;
    movie: UiTopicResult;
    tutorialPage1: UiTopicResult;
    tutorialFinal: UiTopicResult;
  }>(`
    ${HELPER_LUA}
    -- 1. Loading overrides movie & battle
    local loadingRig = makeBaseRig({
      isLoading = true,
      movieScreen = makeObj("WBP_MovieScreen_C"),
      inBattle = true,
    })
    local loading = ui.collect(loadingRig.ctx)

    -- 2. Movie + WBP_TalkEvent_Caption_C subtitle (talk must stay null during movie)
    local captionWin = makeObj("WBP_TalkEvent_Caption_C", {
      m_pTextBlock = makeTextWidget("是你们输了。"),
    })
    local talkLayout = makeObj("WBP_TalkEventLayout_C", {
      WindowManager = makeObj("CPP_WBP_TalkWindowManager", { m_pWindow = captionWin }),
    })
    local movieRig = makeBaseRig({
      movieScreen = makeObj("WBP_MovieScreen_C"),
      eventManager = makeObj("CPP_TalkEventManager", { m_pWidget = talkLayout }),
      guideTag = "UI.System.ActionGuide.Declare.Event",
    })
    local movie = ui.collect(movieRig.ctx)

    -- 3. Tutorial page 1 of 2 (can_close = false) overriding battle + talk
    local pages = makeTArray({
      { Caption = makeText("主菜单"), Text = makeText("第一页说明内容") },
      { Caption = makeText("主要功能"), Text = makeText("第二页说明内容") },
    })
    local tutInner = makeObj("WBP_Tutorial_C", {
      PageIndex = 0,
      Data = { Name = makeText("主菜单"), Pages = pages },
    })
    local tutLayout = makeObj("WBP_TutorialLayout_C", { WBP_Tutorial = tutInner })
    local tutRig = makeBaseRig({
      tutorialWidget = tutLayout,
      inBattle = true,
      eventManager = makeObj("CPP_TalkEventManager", { m_pWidget = talkLayout }),
      guideTag = "UI.System.ActionGuide.Declare.Common.Close",
    })
    local tutorialPage1 = ui.collect(tutRig.ctx)

    -- Advance to page 2 of 2 (can_close = true)
    tutInner.PageIndex = 1
    local tutorialFinal = ui.collect(tutRig.ctx)

    return {
      loading = loading,
      movie = movie,
      tutorialPage1 = tutorialPage1,
      tutorialFinal = tutorialFinal,
    }
  `);

  assert.equal(out.loading.screen_kind, "loading");
  assert.deepEqual(out.loading.movie, { active: true, subtitle: null });

  assert.equal(out.movie.screen_kind, "movie");
  assert.deepEqual(out.movie.movie, { active: true, subtitle: "是你们输了。" });
  assert.equal(out.movie.talk, null);

  assert.equal(out.tutorialPage1.screen_kind, "tutorial");
  assert.deepEqual(out.tutorialPage1.tutorial, {
    name: "主菜单",
    page_index: 0,
    page_count: 2,
    caption: "主菜单",
    text: "第一页说明内容",
    can_close: false,
  });

  assert.equal(out.tutorialFinal.screen_kind, "tutorial");
  assert.deepEqual(out.tutorialFinal.tutorial, {
    name: "主菜单",
    page_index: 1,
    page_count: 2,
    caption: "主要功能",
    text: "第二页说明内容",
    can_close: true,
  });
});

void test("talk reads standard dialogue, UTF-8 200-char truncation, waiting_key, caption telop cutscene, and silent cutscene", () => {
  const out = luaJson<{
    standard: UiTopicResult;
    fallbackData: UiTopicResult;
    captionCutscene: UiTopicResult;
    silentCutscene: UiTopicResult;
  }>(`
    ${HELPER_LUA}
    local bflTalk = makeObj("CPP_BFL_TalkEvent", {
      GetCurrentEventId = function() return makeText("C01_003_01_B") end,
    })

    -- Long Chinese text > 200 chars to verify UTF-8 codepoint truncation
    local longLine = string.rep("格兰谢尔特", 50) -- 250 Chinese chars
    local textBlock = makeObj("CPP_GradualTextBlock", {
      GetText = function() return makeText(longLine) end,
      IsTextFinished = function() return true end,
    })
    local keyWait = makeObj("WBP_TalkKeyWait_C", {
      GetVisibility = function() return 0 end, -- Visible
    })
    local talkWin = makeObj("WBP_TalkWindow_C", {
      Text_Name = makeTextWidget("雷因"),
      m_pTextBlock = textBlock,
      WBP_TalkKeyWait = keyWait,
    })
    local talkLayout = makeObj("WBP_TalkEventLayout_C", {
      WindowManager = makeObj("CPP_WBP_TalkWindowManager", { m_pWindow = talkWin }),
      CanSkip = function() return true end,
      IsAutoPlay = function() return false end,
    })
    local rig1 = makeBaseRig({
      eventManager = makeObj("CPP_TalkEventManager", { m_pWidget = talkLayout }),
      extraObjects = { CPP_BFL_TalkEvent = bflTalk },
      guideTag = "UI.System.ActionGuide.Declare.Event",
    })
    local standard = ui.collect(rig1.ctx)

    -- Fallback to m_pCurrentTalkData when Text_Name/m_pTextBlock return empty, keyWait Collapsed (1)
    local emptyWin = makeObj("WBP_TalkWindow_C", {
      Text_Name = makeTextWidget(""),
      m_pTextBlock = makeObj("CPP_GradualTextBlock", {
        GetText = function() return makeText("") end,
        IsTextFinished = function() return false end,
      }),
      WBP_TalkKeyWait = makeObj("WBP_TalkKeyWait_C", { GetVisibility = function() return 1 end }),
    })
    local curTalkData = makeObj("CPP_TalkEventTalkData", {
      GetSpeaker = function() return makeText("格兰谢尔特王") end,
      GetText = function() return makeText("前往土之神殿调查。") end,
    })
    local talkLayout2 = makeObj("WBP_TalkEventLayout_C", {
      WindowManager = makeObj("CPP_WBP_TalkWindowManager", { m_pWindow = emptyWin }),
      m_pCurrentTalkData = curTalkData,
      CanSkip = function() return false end,
      IsAutoPlay = function() return true end,
    })
    local rig2 = makeBaseRig({
      eventManager = makeObj("CPP_TalkEventManager", { m_pWidget = talkLayout2 }),
      extraObjects = { CPP_BFL_TalkEvent = bflTalk },
    })
    local fallbackData = ui.collect(rig2.ctx)

    -- Caption telop cutscene (boss-closeup-talk)
    local telopWin = makeObj("WBP_TalkEvent_Caption_telop_C", {
      m_pTextBlock = makeObj("CPP_GradualTextBlock", {
        GetText = function() return makeText("嘎喔喔喔喔喔喔！！！") end,
        IsTextFinished = function() return true end,
      }),
    })
    local talkLayout3 = makeObj("WBP_TalkEventLayout_C", {
      WindowManager = makeObj("CPP_WBP_TalkWindowManager", { m_pWindow = telopWin }),
      CanSkip = function() return false end,
      IsAutoPlay = function() return false end,
    })
    local rig3 = makeBaseRig({
      eventManager = makeObj("CPP_TalkEventManager", { m_pWidget = talkLayout3 }),
      extraObjects = { CPP_BFL_TalkEvent = bflTalk },
    })
    local captionCutscene = ui.collect(rig3.ctx)

    -- Silent cutscene (m_pWindow == nil)
    local talkLayout4 = makeObj("WBP_TalkEventLayout_C", {
      WindowManager = makeObj("CPP_WBP_TalkWindowManager", { m_pWindow = nil }),
    })
    local rig4 = makeBaseRig({
      eventManager = makeObj("CPP_TalkEventManager", { m_pWidget = talkLayout4 }),
    })
    local silentCutscene = ui.collect(rig4.ctx)

    return {
      standard = standard,
      fallbackData = fallbackData,
      captionCutscene = captionCutscene,
      silentCutscene = silentCutscene,
    }
  `);

  assert.equal(out.standard.screen_kind, "talk");
  assert.ok(out.standard.talk);
  assert.equal(out.standard.talk.event_id, "C01_003_01_B");
  assert.equal(out.standard.talk.window_class, "WBP_TalkWindow_C");
  assert.equal(out.standard.talk.is_cutscene, false);
  assert.equal(out.standard.talk.speaker, "雷因");
  assert.equal(out.standard.talk.waiting_key, true);
  assert.equal(out.standard.talk.text_finished, true);
  assert.equal(out.standard.talk.can_skip, true);
  assert.equal(out.standard.talk.is_auto, false);
  // 199 chars + "…" = 200 UTF-8 codepoints
  assert.equal(Array.from(out.standard.talk.text ?? "").length, 200);
  assert.ok((out.standard.talk.text ?? "").endsWith("…"));

  assert.equal(out.fallbackData.screen_kind, "talk");
  assert.deepEqual(out.fallbackData.talk, {
    event_id: "C01_003_01_B",
    window_class: "WBP_TalkWindow_C",
    is_cutscene: false,
    speaker: "格兰谢尔特王",
    text: "前往土之神殿调查。",
    waiting_key: false,
    text_finished: false,
    can_skip: false,
    is_auto: true,
  });

  assert.equal(out.captionCutscene.screen_kind, "cutscene");
  assert.deepEqual(out.captionCutscene.talk, {
    event_id: "C01_003_01_B",
    window_class: "WBP_TalkEvent_Caption_telop_C",
    is_cutscene: true,
    speaker: null,
    text: "嘎喔喔喔喔喔喔！！！",
    waiting_key: false,
    text_finished: true,
    can_skip: false,
    is_auto: false,
  });

  assert.equal(out.silentCutscene.screen_kind, "cutscene");
  assert.deepEqual(out.silentCutscene.talk, {
    event_id: null,
    window_class: null,
    is_cutscene: true,
    speaker: null,
    text: null,
    waiting_key: false,
    text_finished: null,
    can_skip: false,
    is_auto: false,
  });
});

void test("choice distinguishes system dialog (m_ChoiceData + GetFocusedItemIndex) and event choice (EntryWidgetList + Image_Select fallback)", () => {
  const out = luaJson<{ dialog: UiTopicResult; eventChoice: UiTopicResult }>(`
    ${HELPER_LUA}
    -- 1. System confirmation dialog (tent-confirm-no -> default focus 1 "否")
    local choiceMenu = makeObj("CPP_EU_WBP_DialogChoice", {
      m_ChoiceData = {
        ChoiceTag = makeTag("UI.Dialog.Choice.YesNo"),
        ChoiceList = makeTArray({ makeText("是"), makeText("否") }),
      },
      GetFocusedItemIndex = function(_, timeType)
        assert(timeType == 0)
        return 1
      end,
    })
    local dialogChoice = makeObj("WBP_DialogChoice_C", { PathMenu = choiceMenu })
    local dialogBox = makeObj("WBP_DialogBox_C", { PathMenu = dialogChoice })
    local dialogLayout = makeObj("WBP_DialogLayout_C", {
      DialogBoxInfo = {
        DialogTag = makeTag("UI.Dialog.Box.Item.Use"),
        ChoiceTag = makeTag("UI.Dialog.Choice.YesNo"),
        Message = makeText("确定要使用吗？"),
      },
      WBP_DialogBox = dialogBox,
    })
    local rig1 = makeBaseRig({
      dialogWidget = dialogLayout,
      modalWidget = dialogLayout,
      activeLayerName = "Modal",
      guideTag = "UI.System.ActionGuide.Declare.Common.Cancel",
    })
    local dialog = ui.collect(rig1.ctx)

    -- 2. Event branch choice (choice-strategy) via WBP_ChoiceDialogLayout_C + DirectionGridPanel fallback
    local function makeChoiceEntry(label, selected)
      return makeObj("WBP_DialogChoiceEntry_C", {
        Text = makeTextWidget(label),
        Image_Select = makeObj("Image", {
          GetVisibility = function() return selected and 0 or 1 end,
        }),
      })
    end
    local dirGrid = makeObj("WBP_DirectionGridPanel_C", {
      EntryWidgetList = makeTArray({
        makeChoiceEntry("我来打头阵！", false),
        makeChoiceEntry("以调查为优先。", true),
        makeChoiceEntry("合作最重要。", false),
      }),
    })
    local dirMenu = makeObj("WBP_DirectionLayoutMenu_C", {
      DirectionGridPanel = dirGrid,
    })
    local choiceBox = makeObj("WBP_ChoiceDialogBox_C", {
      PathMenu = makeObj("WBP_DialogChoice_C", { PathMenu = dirMenu }),
    })
    local choiceLayout = makeObj("WBP_ChoiceDialogLayout_C", {
      IsActivated = function() return true end,
      DialogBoxInfo = { DialogTag = makeTag("None"), ChoiceTag = makeTag("None"), Message = makeText("") },
      WBP_PathMenu = choiceBox,
    })
    local talkWin = makeObj("WBP_TalkWindow_C", {
      Text_Name = makeTextWidget("拉斯韦尔"),
      m_pTextBlock = makeTextWidget("决定作战方针吧。"),
    })
    local talkLayout = makeObj("WBP_TalkEventLayout_C", {
      m_DialogChoiceTag = makeTag("UI.Dialog.Choice.C01_006_01"),
      WindowManager = makeObj("CPP_WBP_TalkWindowManager", { m_pWindow = talkWin }),
      WidgetStack = makeStack("WidgetStack", choiceLayout),
    })
    local rig2 = makeBaseRig({
      eventManager = makeObj("CPP_TalkEventManager", { m_pWidget = talkLayout }),
      eventWidget = talkLayout,
      activeLayerName = "Event",
    })
    local eventChoice = ui.collect(rig2.ctx)

    return { dialog = dialog, eventChoice = eventChoice }
  `);

  assert.equal(out.dialog.screen_kind, "dialog");
  assert.deepEqual(out.dialog.choice, {
    source: "dialog",
    dialog_tag: "UI.Dialog.Box.Item.Use",
    choice_tag: "UI.Dialog.Choice.YesNo",
    message: "确定要使用吗？",
    options: ["是", "否"],
    focus_index: 1,
  });

  assert.equal(out.eventChoice.screen_kind, "choice");
  assert.deepEqual(out.eventChoice.choice, {
    source: "event_choice",
    dialog_tag: null,
    choice_tag: "UI.Dialog.Choice.C01_006_01",
    message: "",
    options: ["我来打头阵！", "以调查为优先。", "合作最重要。"],
    focus_index: 1,
  });
  // Underlying talk prompt remains readable alongside event choice.
  assert.ok(out.eventChoice.talk);
  assert.equal(out.eventChoice.talk.speaker, "拉斯韦尔");
  assert.equal(out.eventChoice.talk.text, "决定作战方针吧。");
});

void test("menu covers system_menu (including stale FingerCursor fallback), savepoint, title, and main_menu 4 subpages", () => {
  const out = luaJson<{
    sysMenu: UiTopicResult;
    savepoint: UiTopicResult;
    title: UiTopicResult;
    journal: UiTopicResult;
    heal: UiTopicResult;
    item: UiTopicResult;
    worldMap: UiTopicResult;
  }>(`
    ${HELPER_LUA}
    -- 1. System menu with stale CursorWidget (Collapsed) on Save and active CursorWidget (HitTestInvisible=3) on Load
    local function makeSysItem(tagName, cursorVis)
      local cursor = cursorVis ~= nil and makeObj("WBP_FingerCursor_C", {
        GetVisibility = function() return cursorVis end,
      }) or nil
      return makeObj("WBP_SystemList_Item_C", {
        EntryType = makeTag(tagName),
        PathButton = makeObj("WBP_LeftLineButton_C", {
          PathButton = makeObj("WBP_ListEntryButtonBase_C", {
            CursorWidget = cursor,
          }),
        }),
      })
    end
    local sysGrid = makeObj("WBP_DirectionGridPanel_C", {
      EntryWidgetList = makeTArray({
        makeSysItem("UI.System.SystemMenu.Entry.Save", 1), -- stale pointer, Collapsed
        makeSysItem("UI.System.SystemMenu.Entry.Load", 3), -- active focus, HitTestInvisible
        makeSysItem("UI.System.SystemMenu.Entry.Help", nil),
      }),
    })
    local vmSys = makeObj("VM_SystemMenu_C", {
      EnableEntryList = makeTArray({
        makeTag("UI.System.SystemMenu.Entry.Save"),
        makeTag("UI.System.SystemMenu.Entry.Load"),
        makeTag("UI.System.SystemMenu.Entry.Help"),
      }),
    })
    local sysLayout = makeObj("WBP_SystemMenuLayout_C", {
      IsActivated = function() return true end,
      WBP_SystemMenu = makeObj("WBP_SystemMenu_C", {
        VM_SystemMenu = vmSys,
        PathMenu = makeObj("WBP_DirectionLayoutMenu_C", { DirectionGridPanel = sysGrid }),
      }),
    })
    local sysMenu = ui.collect(makeBaseRig({
      systemWidget = sysLayout,
      activeLayerName = "System",
      guideTag = "UI.System.ActionGuide.Declare.Common.Cancel",
    }).ctx)

    -- 2. Savepoint menu (inside TalkEvent pipeline -> talk must be null, screen_kind = savepoint)
    local spOptList = makeObj("WBP_OptionList_C", { FocuedIndex = 2 })
    local spCmdWin = makeObj("WBP_SHOP_CommandWindow_C", {
      WBP_SHOP_CommonLists = makeObj("WBP_SHOP_CommandLists_C", {
        WBP_SHOP_MainList = makeObj("WBP_SHOP_CommandList_C", { WBP_CommandList = spOptList }),
      }),
    })
    local spLayout = makeObj("WBP_SavePointLayout_C", {
      IsActivated = function() return true end,
      CommandWindow = spCmdWin,
      m_VM_SavePoint = makeObj("CPP_VM_SavePoint", {
        GetCommandTypeList = function() return makeTArray({ 1, 2, 3 }) end,
      }),
    })
    local savepoint = ui.collect(makeBaseRig({
      systemWidget = spLayout,
      eventManager = makeObj("CPP_TalkEventManager"),
      activeLayerName = "System",
      guideTag = "UI.System.ActionGuide.Declare.Shop.SavePoint",
    }).ctx)

    -- 3. Title screen
    local titleMenu = makeObj("WBP_DirectionLayoutMenu_C", {
      FocuedItemDataMap = makeTMap({ { 0, { Index = 0 } } }),
      DirectionGridPanel = makeObj("WBP_DirectionGridPanel_C", {
        EntryWidgetList = makeTArray({
          makeObj("WBP_Title_ListEntry_C", { CommonTextBlock_47 = makeTextWidget("新游戏"), EntryType = 2 }),
          makeObj("WBP_Title_ListEntry_C", { CommonTextBlock_47 = makeTextWidget("选项"), EntryType = 3 }),
        }),
      }),
    })
    local titleLayout = makeObj("WBP_TitleLayout_C", {
      IsActivated = function() return true end,
      WBP_DirectionLayoutMenu = titleMenu,
    })
    local title = ui.collect(makeBaseRig({
      gameWidget = titleLayout,
      taskClass = "CPP_TitleTaskMain",
      activeLayerName = "Game",
    }).ctx)

    -- 4. Main menu subpages: Journal, Heal, Item, Map
    local function collectMainSub(sceneTag, innerWidget, extraObjects)
      local vmMain = makeObj("CPP_VM_MainMenu", {
        SceneDataStack = makeTArray({ makeTag(sceneTag) }),
      })
      local mainLayout = makeObj("WBP_MainMenuLayout_C", {
        IsActivated = function() return true end,
        CPP_VM_MainMenu = vmMain,
        CPP_EU_ActivatableWidgetStack_51 = makeStack("Inner", innerWidget),
      })
      return ui.collect(makeBaseRig({
        systemWidget = mainLayout,
        activeLayerName = "System",
        mainMenuOpen = true,
        mainMenuVm = vmMain,
        mainMenuLayout = mainLayout,
        extraObjects = extraObjects,
      }).ctx)
    end

    local journal = collectMainSub("UI.MainMenu.Scene.Journal", nil, {
      VM_Journal_C = makeObj("VM_Journal_C", {
        DisplayQuestCategory = 1,
        FocusQuestId = 1001,
        DisplayQuestDataList = makeTArray({
          { ID = 1001, Name = makeText("第1章探究的开端"), Subtitle = makeText("调查土之神殿") },
        }),
      }),
    })

    local heal = collectMainSub(
      "UI.MainMenu.Scene.Heal",
      makeObj("WBP_HealLayout_C", { IsActivated = function() return true end, CurrentState = 1 }),
      {
        VM_HealLayout_C = makeObj("VM_HealLayout_C", {
          UnitId = 1001,
          TargetUnitId = 1002,
          bCanAoe = true,
          DisplayActionList = makeTArray({
            { ActionKey = { Category = 2, ID = 1 }, TmpNum = 5, UserUnitId = -1 },
          }),
        }),
      }
    )

    local item = collectMainSub(
      "UI.MainMenu.Scene.Item",
      makeObj("WBP_ItemLayout_C", { IsActivated = function() return true end, CurrentState = 3 }),
      {}
    )

    local worldMap = collectMainSub("UI.MainMenu.Scene.Map", nil, {
      VM_WldMap_C = makeObj("VM_WldMap_C", {
        CurrentIslandType = 1,
        CurrentAreaId = 1,
      }),
    })

    return {
      sysMenu = sysMenu,
      savepoint = savepoint,
      title = title,
      journal = journal,
      heal = heal,
      item = item,
      worldMap = worldMap,
    }
  `);

  assert.equal(out.sysMenu.screen_kind, "system_menu");
  assert.deepEqual(out.sysMenu.menu, {
    kind: "system_menu",
    scene_stack: [],
    entries: [
      "UI.System.SystemMenu.Entry.Save",
      "UI.System.SystemMenu.Entry.Load",
      "UI.System.SystemMenu.Entry.Help",
    ],
    focus_index: 1,
    sub_state: null,
  });

  assert.equal(out.savepoint.screen_kind, "savepoint");
  assert.equal(out.savepoint.talk, null);
  assert.deepEqual(out.savepoint.menu, {
    kind: "savepoint",
    scene_stack: [],
    entries: ["SavePoint", "Tent", "Teleport"],
    focus_index: 2,
    sub_state: null,
  });

  assert.equal(out.title.screen_kind, "title");
  assert.deepEqual(out.title.menu, {
    kind: "title",
    scene_stack: [],
    entries: ["新游戏", "选项"],
    focus_index: 0,
    sub_state: null,
  });

  assert.equal(out.journal.screen_kind, "main_menu");
  assert.deepEqual(out.journal.menu, {
    kind: "main_menu",
    scene_stack: ["UI.MainMenu.Scene.Journal"],
    entries: [],
    focus_index: null,
    sub_state: {
      display_quest_category: "Main",
      focus_quest_id: 1001,
      quests: [{ id: 1001, name: "第1章探究的开端", subtitle: "调查土之神殿" }],
    },
  });

  assert.equal(out.heal.screen_kind, "main_menu");
  assert.deepEqual(out.heal.menu?.sub_state, {
    current_state: 1,
    unit_id: 1001,
    target_unit_id: 1002,
    can_aoe: true,
    actions: [{ category: "Item", id: 1, count: 5, user_unit_id: -1 }],
  });

  assert.deepEqual(out.item.menu?.sub_state, { current_state: 3 });
  assert.deepEqual(out.worldMap.menu?.sub_state, {
    current_island_type: "Grandshelt",
    current_area_id: 1,
  });
});

void test("shop reads buy/sell pricing from VM_Shop_TradeWindow_C, ignores CPP_VM_Colosseum_Shop.IsBuy, and drops when unbound", () => {
  const out = luaJson<{ buy: UiTopicResult; sell: UiTopicResult; closed: UiTopicResult }>(`
    ${HELPER_LUA}
    local function makeShopRig(isBuy, unbound)
      local vmCol = makeObj("CPP_VM_Colosseum_Shop", {
        IsBuy = false, -- Always false in-game; must NOT be trusted!
        ShopID = 2,
        ShopType = 1, -- ToolShop
        SceneTypeStack = makeTArray({ 1, 2 }), -- Command, Shop
      })
      local vmTrade = makeObj("VM_Shop_TradeWindow_C", {
        ShopID = 2,
        IsBuy = isBuy,
        CurrentTotalAmount = isBuy and 200 or 50,
        CurrentItemList = makeTArray({ 1, 24 }),
        ItemInfoList = makeTArray({
          { ID = 1, ItemName = makeText("恢复药"), buyingPrice = 100, sellingPrice = 25, ItemNum = 4, Stock = -1, BuyCountLimit = -1 },
          { ID = 24, ItemName = makeText("烟雾弹"), buyingPrice = 150, sellingPrice = 37, ItemNum = 0, Stock = 3, BuyCountLimit = 3 },
        }),
      })
      local catTab = makeObj("WBP_InventoryCategoryTab_C", {
        CurSelectedIdx = isBuy and 0 or 1,
        CategoryList = makeTArray({
          makeTag("UI.Common.Item.Category.Procedure"),
          makeTag("UI.Common.Item.Category.Consumable"),
        }),
      })
      local function wrapOptionList(idx)
        return makeObj("WBP_SHOP_CommandWindow_C", {
          WBP_SHOP_CommonLists = makeObj("WBP_SHOP_CommandLists_C", {
            WBP_SHOP_MainList = makeObj("WBP_SHOP_CommandList_C", {
              WBP_CommandList = makeObj("WBP_OptionList_C", { FocuedIndex = idx }),
            }),
          }),
        })
      end
      local tradeWin = makeObj("WBP_SHOP_TradeWindow_C", {
        IsActivated = function() return true end,
        WBP_InventoryCategoryTab = catTab,
        WBP_SHOP_CommandWindow = wrapOptionList(isBuy and 0 or 1),
      })
      local shopLayout = makeObj("WBP_SHOP_Layout_C", {
        IsActivated = function() return true end,
        CPP_VM_Colosseum_Shop = (not unbound) and vmCol or nil,
        VM_Shop_TradeWindow = vmTrade,
        CommandWindow = wrapOptionList(isBuy and 0 or 1),
        WidgetStack = makeStack("WidgetStack", tradeWin),
      })
      local vmTooltip = makeObj("VM_Tooltip_C", {
        ActionKey = { Category = 2, ID = isBuy and 1 or 24 },
      })
      return makeBaseRig({
        systemWidget = shopLayout,
        eventManager = makeObj("CPP_TalkEventManager"),
        activeLayerName = "System",
        guideTag = "UI.System.ActionGuide.Declare.Shop",
        extraObjects = { VM_Tooltip_C = vmTooltip },
      })
    end

    local buy = ui.collect(makeShopRig(true, false).ctx)
    local sell = ui.collect(makeShopRig(false, false).ctx)
    local closed = ui.collect(makeShopRig(true, true).ctx)
    return { buy = buy, sell = sell, closed = closed }
  `);

  assert.equal(out.buy.screen_kind, "shop");
  assert.equal(out.buy.talk, null);
  assert.deepEqual(out.buy.shop, {
    shop_id: 2,
    shop_type: "ToolShop",
    scene_stack: ["Command", "Shop"],
    is_buy: true,
    category_index: 0,
    categories: ["UI.Common.Item.Category.Procedure", "UI.Common.Item.Category.Consumable"],
    cmd_focus_index: 0,
    item_focus_index: 0,
    tooltip_item_id: 1,
    total_amount: 200,
    items: [
      { id: 1, name: "恢复药", price: 100, owned: 4, stock: -1, limit: -1 },
      { id: 24, name: "烟雾弹", price: 150, owned: 0, stock: 3, limit: 3 },
    ],
  });

  assert.ok(out.sell.shop);
  assert.equal(out.sell.shop.is_buy, false);
  assert.equal(out.sell.shop.category_index, 1);
  assert.equal(out.sell.shop.cmd_focus_index, 1);
  assert.equal(out.sell.shop.item_focus_index, 1);
  assert.equal(out.sell.shop.tooltip_item_id, 24);
  assert.equal(out.sell.shop.items[0]?.price, 25);
  assert.equal(out.sell.shop.items[1]?.price, 37);

  // When CPP_VM_Colosseum_Shop is unbound on exit, ui.shop must be null.
  assert.equal(out.closed.shop, null);
});

void test("save_ui and battle_result handle save/load modes, level-up sub-window, and pooled ResultWidget guard", () => {
  const out = luaJson<{
    saveMode: UiTopicResult;
    loadMode: UiTopicResult;
    rewardPage: UiTopicResult;
    levelUpPage: UiTopicResult;
    staleResult: UiTopicResult;
    inBattle: UiTopicResult;
  }>(`
    ${HELPER_LUA}
    -- 1. Save UI (mode = save)
    local saveDataList = makeObj("WBP_SaveDataList_C", {
      IsSaveLayout = true,
      SlotIndexList = makeTArray({ 1, 2, 3, 4, 5, 6, 7, 8, 9, 10 }),
      PathMenu = makeObj("WBP_ListViewMenu_C", {
        GetFocusedItemIndex = function() return 2 end,
      }),
    })
    local saveLayout = makeObj("WBP_SaveLayout_C", {
      IsActivated = function() return true end,
      PathMenu = saveDataList,
    })
    local saveMode = ui.collect(makeBaseRig({
      front1Widget = saveLayout,
      eventManager = makeObj("CPP_TalkEventManager"),
      activeLayerName = "System.Front1",
    }).ctx)

    -- 2. Load UI (mode = load)
    local loadLayout = makeObj("WBP_LoadLayout_C", {
      IsActivated = function() return true end,
      PathMenu = makeObj("WBP_SaveDataList_C", {
        IsSaveLayout = false,
        SlotIndexList = makeTArray({ 0, 1, 2 }),
        PathMenu = makeObj("WBP_ListViewMenu_C", {
          GetFocusedItemIndex = function() return 0 end,
        }),
      }),
    })
    local loadMode = ui.collect(makeBaseRig({
      front1Widget = loadLayout,
      activeLayerName = "System.Front1",
    }).ctx)

    -- 3. Battle result reward page vs growth/level-up page vs stale IsFinishPage = true
    local function makeResult(pageIdx, showLevelUp, isFinishPage)
      local levelUpWin = makeObj("WBP_BtlResultPage_LevelUpWindow_C", {
        IsShouldDisplay = showLevelUp,
      })
      local growthWin = makeObj("WBP_BtlResultPage_GrowthWindow_C", {
        WBP_BtlResultPage_LevelUpWindow = levelUpWin,
      })
      local rewardWin = makeObj("WBP_BtlResultPage_RewardWindow_C")
      local switcher = makeObj("WidgetSwitcher", {
        GetActiveWidget = function() return pageIdx == 1 and growthWin or rewardWin end,
      })
      return makeObj("WBP_Btl_ResultLayout_C", {
        IsFinishPage = isFinishPage,
        WBP_BtlResultPage_Layout = makeObj("WBP_BtlResultPage_Layout_C", {
          CurrentPageIndex = pageIdx,
          WidgetSwitcher = switcher,
        }),
      })
    end

    local rewardPage = ui.collect(makeBaseRig({
      taskClass = "CPP_BtlTaskWin",
      resultWidget = makeResult(0, false, false),
      guideTag = "UI.System.ActionGuide.Declare.Battle.Result",
    }).ctx)

    local levelUpPage = ui.collect(makeBaseRig({
      taskClass = "CPP_BtlTaskWin",
      resultWidget = makeResult(1, true, false),
    }).ctx)

    local staleResult = ui.collect(makeBaseRig({
      taskClass = "CPP_BtlTaskWin",
      resultWidget = makeResult(1, true, true),
    }).ctx)

    local inBattle = ui.collect(makeBaseRig({
      taskClass = "CPP_BtlTaskCommand",
      inBattle = true,
      resultWidget = makeResult(1, true, true),
    }).ctx)

    return {
      saveMode = saveMode,
      loadMode = loadMode,
      rewardPage = rewardPage,
      levelUpPage = levelUpPage,
      staleResult = staleResult,
      inBattle = inBattle,
    }
  `);

  assert.equal(out.saveMode.screen_kind, "save_ui");
  assert.equal(out.saveMode.talk, null);
  assert.deepEqual(out.saveMode.save_ui, {
    mode: "save",
    slot_indices: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
    focus_index: 2,
  });

  assert.equal(out.loadMode.screen_kind, "save_ui");
  assert.deepEqual(out.loadMode.save_ui, {
    mode: "load",
    slot_indices: [0, 1, 2],
    focus_index: 0,
  });

  assert.equal(out.rewardPage.screen_kind, "battle_result");
  assert.deepEqual(out.rewardPage.battle_result, {
    page_index: 0,
    has_level_up_window: false,
  });

  assert.equal(out.levelUpPage.screen_kind, "battle_result");
  assert.deepEqual(out.levelUpPage.battle_result, {
    page_index: 1,
    has_level_up_window: true,
  });

  assert.equal(out.staleResult.battle_result, null);
  assert.equal(out.staleResult.screen_kind, "field");

  assert.equal(out.inBattle.screen_kind, "battle");
  assert.equal(out.inBattle.battle_result, null);
});

void test("on-demand command input_state returns active IMCs, bindings, fallback AppliedInputContexts, and error when missing", () => {
  const out = luaJson<{
    normal: InputStateResult;
    normalErr: string | null;
    fallback: InputStateResult;
    missingRes: InputStateResult | null;
    missingErr: string | null;
  }>(`
    ${HELPER_LUA}
    local keyTable = {
      UICommon_Decide = { CurrentKey = { KeyName = makeText("Enter") }, DefaultKey = { KeyName = makeText("Z") } },
      UICommon_Cancel = { CurrentKey = { KeyName = makeText("None") }, DefaultKey = { KeyName = makeText("Escape") } },
      Map_OpenMainMenu_Map = { CurrentKey = { KeyName = makeText("M") } },
    }
    local inputMgr = makeObj("CPP_InputManager", {
      bSwapDecideButton = false,
      IsEnableInput = function() return true end,
      GetAllIMCStackList = function()
        return makeTArray({
          { parentMappingContext = 1, childMappingContextList = makeTArray({ 14, 12 }) },
        })
      end,
      FindCurrentMappingForSlot = function(_, name, slot)
        assert(slot == 0)
        return keyTable[name]
      end,
    })
    local rig1 = makeBaseRig({
      blockedTokens = { "WindowDeactivate" },
      extraObjects = { CPP_InputManager = inputMgr },
    })
    local normal, normalErr = ui.commands.input_state(rig1.ctx, {
      mapping_names = { "UICommon_Decide", "UICommon_Cancel", "Map_OpenMainMenu_Map", "Unknown_Action" },
    })

    -- Fallback to PlayerController.PlayerInput.AppliedInputContexts when GetAllIMCStackList returns empty
    local emptyInputMgr = makeObj("CPP_InputManager", {
      bSwapDecideButton = true,
      IsEnableInput = function() return false end,
      GetAllIMCStackList = function() return makeTArray({}) end,
      FindCurrentMappingForSlot = function() return nil end,
    })
    local imcObj = makeObj("InputMappingContext", {}, { fname = "IMC_Field_Default" })
    local pc = makeObj("PlayerController", {
      PlayerInput = makeObj("EnhancedPlayerInput", {
        AppliedInputContexts = makeTMap({ { imcObj, 1 } }),
      }),
    })
    local rig2 = makeBaseRig({
      extraObjects = { CPP_InputManager = emptyInputMgr, PlayerController = pc },
    })
    local fallback = ui.commands.input_state(rig2.ctx, { mapping_names = { "UICommon_Decide" } })

    -- Missing CPP_InputManager returns nil + Chinese error message
    local rig3 = makeBaseRig({})
    local missingRes, missingErr = ui.commands.input_state(rig3.ctx, {})

    return {
      normal = normal,
      normalErr = normalErr or json.null,
      fallback = fallback,
      missingRes = missingRes or json.null,
      missingErr = missingErr or json.null,
    }
  `);

  assert.equal(out.normalErr, null);
  assert.deepEqual(out.normal, {
    active_imcs: ["MapUnit", "UI_Field", "UI_Common"],
    swap_decide: false,
    input_enabled: true,
    input_blocked: ["WindowDeactivate"],
    key_bindings: {
      UICommon_Decide: "Enter",
      UICommon_Cancel: "Escape",
      Map_OpenMainMenu_Map: "M",
      Unknown_Action: null,
    },
  });

  assert.deepEqual(out.fallback.active_imcs, ["IMC_Field_Default"]);
  assert.equal(out.fallback.swap_decide, true);
  assert.equal(out.fallback.input_enabled, false);
  assert.deepEqual(out.fallback.key_bindings, { UICommon_Decide: null });

  assert.equal(out.missingRes, null);
  assert.match(out.missingErr ?? "", /CPP_InputManager/u);
});

void test("missing/invalid objects and property getter exceptions record errors without breaking sibling fields", () => {
  const out = luaJson<{
    missingResult: UiTopicResult;
    missingErrors: string[];
    throwingResult: UiTopicResult;
    throwingErrors: string[];
  }>(`
    ${HELPER_LUA}
    -- Case 1: missing/invalid core subsystems; fallback CPP_BFL_UI_Input:IsBlocked() works
    local errors1 = {}
    local bflInput = makeObj("CPP_BFL_UI_Input", {
      IsBlocked = function() return true end,
    })
    local staleSub = makeObj("CPP_GameWidgetManagerSubsystem", {}, { valid = false })
    local ctx1 = {
      find = function(cls)
        if cls == "CPP_GameWidgetManagerSubsystem" then return staleSub end
        if cls == "CPP_BFL_UI_Input" then return bflInput end
        return nil
      end,
      findAll = function() error("no findAll") end,
      err = function(field, msg) errors1[#errors1 + 1] = field .. ": " .. tostring(msg) end,
      cache = {},
      clockMs = 0,
      tick = 1,
    }
    local missingResult = ui.collect(ctx1)

    -- Case 2: property read & getter throws mid-tick; other fields still succeed
    local throwingLayout = makeObj("WBP_OverAllLayout_C", {
      GetInputTargetLayer = function() error("layer pointer stale") end,
    })
    local throwingGuide = setmetatable({
      IsValid = function() return true end,
    }, {
      __index = function(_, key)
        if key == "GuideTag" then error("guide tag crashed") end
        return nil
      end,
    })
    local throwingTut = makeObj("WBP_TutorialLayout_C", {
      WBP_Tutorial = setmetatable({
        IsValid = function() return true end,
      }, {
        __index = function(_, key)
          if key == "PageIndex" then error("tutorial page boom") end
          return nil
        end,
      }),
    })
    local rig2 = makeBaseRig({
      tutorialWidget = throwingTut,
      blockedTokens = { "SystemBusy" },
    })
    rig2.subsystems.gwMgr.m_GameLayout = throwingLayout
    rig2.objects.CPP_ActionGuideSubsystem = throwingGuide
    local throwingResult = ui.collect(rig2.ctx)

    return {
      missingResult = missingResult,
      missingErrors = json.array(errors1),
      throwingResult = throwingResult,
      throwingErrors = json.array(rig2.errors),
    }
  `);

  assert.equal(out.missingResult.screen_kind, "field");
  assert.equal(out.missingResult.active_layer, null);
  assert.equal(out.missingResult.active_widget, null);
  assert.equal(out.missingResult.action_guide_tag, null);
  assert.deepEqual(out.missingResult.input_blocked, ["Blocked"]);
  assert.ok(out.missingErrors.some((e) => e.includes("CPP_GameWidgetManagerSubsystem")));
  assert.ok(out.missingErrors.some((e) => e.includes("CPP_ActionGuideSubsystem")));

  assert.equal(out.throwingResult.screen_kind, "tutorial");
  assert.deepEqual(out.throwingResult.input_blocked, ["SystemBusy"]);
  assert.equal(out.throwingResult.action_guide_tag, null);
  assert.equal(out.throwingResult.active_layer, null);
  assert.ok(out.throwingErrors.some((e) => e.includes("layer pointer stale")));
  assert.ok(out.throwingErrors.some((e) => e.includes("guide tag crashed")));
  assert.ok(out.throwingErrors.some((e) => e.includes("tutorial page boom")));
});

void test("cache reuses singletons on tick 2, re-fetches on invalidation, and drops WorldSubsystems on map change", () => {
  const out = luaJson<{
    tick1Counts: Record<string, number>;
    tick2Delta: Record<string, number>;
    afterInvalidateDelta: Record<string, number>;
    afterMapChangeDelta: Record<string, number>;
  }>(`
    ${HELPER_LUA}
    local rig = makeBaseRig({ guideTag = "UI.System.ActionGuide.Declare.Field" })

    local function snapshotCounts()
      local copy = {}
      for k, v in pairs(rig.findCounts) do copy[k] = v end
      return copy
    end
    local function delta(before, after)
      local d = {}
      for k, v in pairs(after) do
        local diff = v - (before[k] or 0)
        if diff > 0 then d[k] = diff end
      end
      return json.object(d)
    end

    -- Tick 1: cold start looks up singletons once
    ui.collect(rig.ctx)
    local c1 = snapshotCounts()

    -- Tick 2: warm cache should NOT call ctx.find for any subsystem (only World check)
    rig.ctx.tick = 2
    ui.collect(rig.ctx)
    local c2 = snapshotCounts()

    -- Tick 3: invalidate CPP_ActionGuideSubsystem -> only it (plus World) is looked up again
    rig.subsystems.guideSub._valid = false
    rig.objects.CPP_ActionGuideSubsystem = makeObj("CPP_ActionGuideSubsystem", {
      GuideTag = makeTag("UI.System.ActionGuide.Declare.Field"),
    })
    rig.ctx.tick = 3
    ui.collect(rig.ctx)
    local c3 = snapshotCounts()

    -- Tick 4: map change (new World fullName) drops WorldSubsystems but keeps GameInstanceSubsystems
    rig.objects.World = makeObj("World", {}, { fullName = "World /Game/Maps/Dungeon01.Dungeon01" })
    rig.ctx.tick = 4
    ui.collect(rig.ctx)
    local c4 = snapshotCounts()

    return {
      tick1Counts = c1,
      tick2Delta = delta(c1, c2),
      afterInvalidateDelta = delta(c2, c3),
      afterMapChangeDelta = delta(c3, c4),
    }
  `);

  assert.equal(out.tick1Counts.CPP_GameWidgetManagerSubsystem, 1);
  assert.equal(out.tick1Counts.CPP_ActionGuideSubsystem, 1);
  assert.equal(out.tick1Counts.CPP_UI_BattleSubsystem, 1);
  assert.equal(out.tick1Counts.CPP_MainMenuSubsystem, 1);

  // Tick 2: zero singleton re-lookups (only World check)
  assert.deepEqual(out.tick2Delta, { World: 1 });

  // Tick 3: invalidated singleton re-looked up
  assert.deepEqual(out.afterInvalidateDelta, {
    World: 1,
    CPP_ActionGuideSubsystem: 1,
  });

  // Tick 4: map change re-fetches WorldSubsystems only, GI subsystems remain cached
  assert.deepEqual(out.afterMapChangeDelta, {
    World: 1,
    CPP_UI_BattleSubsystem: 1,
    CPP_MainMenuSubsystem: 1,
  });
});
