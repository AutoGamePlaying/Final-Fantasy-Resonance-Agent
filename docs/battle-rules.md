# 战斗规则与执行器决策启发

本文整理《FINAL FANTASY RESONANCE》在游戏教学与真机快照中证实的核心战斗机制，并给出面向 R5 执行器战斗规则表的设计启发。本文只写通用规则、状态判据与行动优先级，不写特定章节或具体 Boss 的攻略。

---

## 1. 三条核心战斗规则（含快照与数据表实证）

### 规则一：命中弱点削减破防槽，归零触发破防（Break）

- **机制定义**（对应游戏教学 `ST_Tutorial_Weak`、`ST_Tutorial_Break`、`ST_Tutorial_EnemyLevel`）：
  - 每名敌人拥有独立的破防槽（BreakPoint，BP）与属性弱点/耐性。
  - 敌方弱点初始在 UI 显示为“？”，用对应属性攻击命中后会揭露弱点。
  - 命中弱点可造成更高伤害并大幅削减破防槽；不同招式在 `DT_SkillData` 中有不同的 `breakDamageValue`（破防削减威力，界面以子弹图标标示），有的擅长输出伤害、有的擅长削减破防槽。
  - 选目标时会预先计算该次攻击预计削减后的破防槽比例。
  - 敌我存在等级压制：面对高等级敌人时破防槽更难削减。
  - 破防槽归零后，敌人进入**破防状态**，该回合无法行动且承受更高伤害。
- **快照与数据表证据**：
  - `DT_UnitParameter` 定义了每类敌人的基础破防槽上限与属性抗性（例：钢蝠 `UnitId=2` BP=50、弱冰/风；贝尔摩达 `UnitId=401` BP=145、弱火/暗、雷减半）。
  - 弱点揭露实证：`target-a` / `target-b` 中钢蝠 `ElemResistDataList` 的 `(Ice, Weakness, bIsAnalyzed=false)` 在 `weakness-hit` 命中冰攻后变为 `true`；Boss 战中 `(Fire, Weakness, false)` 在 `tutorial-charge` 变为 `true`。同时，`ElemResistDataList` 在揭露前已包含真实弱点列表。
  - 破防预测实证：`target-a` / `target-b` 选中钢蝠并指向火炎（`DT_SkillData[220010]`，`breakDamageValue=17`）时，`WBP_PredictProgress_C.PredictPercent = 0.66`（`(50 - 17) / 50 = 0.66`）。
  - 破防状态实证：`tutorial-break` → `boss-break` 中，Boss 的 `BreakPointVM` 进度条从 `1.0` 归零到 `0.0`，`CPP_BtlUnitParameter.m_ContEffectList` 新增 `CPP_ContFxBreak` 实例，时间轴条目 `CPP_VM_TimeLineEntry.bIsBreak` 由 `false` 变为 `true`。

### 规则二：敌方危险行动（斗气预告）可用破防打断

- **机制定义**（对应游戏教学 `ST_Tutorial_Battle_Danger`）：
  - 部分敌人（主要是 Boss 或强敌）会在时间轴头像与场面上出现斗气/危险提示，预告下一次行动为高威力强力招式（多为全体攻击或高倍率技能）。
  - 在其出手前将其打入**破防状态**，即可中止本次危险行动。
  - 若当前回合内无法击破其破防槽，我方应选择**防御**（`SkillId=400010`）降低伤害以稳住阵脚。
- **快照与数据表证据**：
  - 数据表 `DT_BtlUnitAIParameter.highRiskSkillIdList` 显式列出敌方高危招式（如 `UnitId=401` 的 `505040`“强击冲撞”为 `TargetType::Group` 全体物理攻击）。
  - 预告与打断全链路实证：`tutorial-break` 中 `VM_Timeline_C.RiskUnitUniqueIdList=[]`、`bIsRisky=false`、`WBP_BtlTimeLine_RiskIcon_C` 隐藏 → 预告阶段（`tutorial-charge`、`boss-break`）`RiskUnitUniqueIdList=[2]`、`bIsRisky=true`、`RiskIcon` 显示、`CPP_BtlHighRiskAtkDelVal.uniqueIdList=[2]` → 破防结算进入追加阶段后（`tutorial-break-bonus`、`limit-2`）全部清零还原为 `[]` / `false`。

### 规则三：破防取消敌方本回合行动，我方获得追加行动 / 全面破防

- **机制定义**（对应游戏教学 `ST_Tutorial_Break`、`ST_Tutorial_ExtraPhaze`、`ST_Tutorial_FullBreak`、`ST_Tutorial_FinishBlow`、`ST_Tutorial_ResultBonus`）：
  - 敌人陷入破防后，**其本回合剩余行动被直接取消**。
  - **单体破防（追加阶段）**：打出破防的我方角色在本回合结束前进入追加阶段（`eBtlPhaseType::Additional`），额外获得一次行动机会。
  - **全面破防（Full Break）**：同一回合内将场上所有存活敌人都破防（单 Boss 战中破防 Boss 即满足全敌破防），触发全面破防，**全体存活我方**均在回合末追加阶段获得一次追加行动。
  - **终结技与结算奖励**：装备幻兵且触发全面破防时，追加阶段末尾可衔接共鸣终结技（`eBtlPhaseType::Finish`）；此外战斗结算包含三项可触发奖励——过量击杀（追加阶段中追击已全灭敌人，增加金币）、无伤（增加经验）、破防终结（最后一名敌人在破防态被击败，增加 MRP）。
- **快照证据**：
  - 时间轴实证：`tutorial-charge` 中 `VM_Timeline_C.TurnVMMap[0].EntryDataList` 为 `[(2, Normal, Current), (0, Normal, Current)]`（原本轮到 Boss `2` 行动）；破防后在 `boss-break` / `tutorial-break-bonus` / `limit-2` 中变为 `[(1, Additional, Current), (0, Additional, Current)]`——Boss `2` 从本回合时间轴消失，我方两人都进入 `Additional` 追加阶段。
  - 底层队列实证：`CPP_BtlUnitTimelineScheduler.m_AdditionalTimeline` 由 `[]` 变为 `[Unit(1), Unit(0)]`，全队 `ActionVM.bIsInExPhase` 翻为 `true`，`CPP_UI_BtlCmdManager.ActionUnitUniqueIdList` 变为 `[1, 0]`，且 `CPP_TaskManager.m_pTask` 在 `limit-2` 切为 `CPP_BtlTaskAdditionalTurn`、`BtlPhaseType=Additional`。

### 连带战斗机制（供规则表扩展）

- **极限技（Limit Burst，`ST_Tutorial_Limit` / `ST_Tutorial_LB2`）**：角色专属 `LimitPointVM` 蓄满至 `CurrentLevel >= 1`（Lv1 需 100，习得 Lv2 后上限 200）时，可通过极限技指令（`Cmd[10]`）释放高威力招式（如 `limit-2` 中拉斯韦尔 `100/100, Lv 1` 可选 `402040` 歼月）。
- **状态异常（`ST_Tutorial_Battle_Abnormal`）**：中毒、失明、睡眠、沉默、麻痹、石化（全员石化即全灭）、混乱、魅惑、狂暴、即死/死亡宣告；可用解状态白魔法或对应道具解除。
- **召唤（`ST_Tutorial_Summon` / `ST_Tutorial_SummonVision`）**：幻兽召唤（`Cmd[6]`）或幻兵召唤（`Cmd[5]`）最多同时在场 1 体，协同作战一定回合。

---

## 2. 对执行器战斗规则表的启发

执行器不调用 LLM 临场推理普通战斗操作，而是基于 FfrBridge v1 提供的结构化状态和离线数据表，用确定性规则表驱动回合决策。

### 2.1 状态读取清单

执行器每轮决策仅需读取以下字段（完整路径见 [battle.md](state-map/battle.md) §11）并结合离线 `DT_SkillData` / `DT_UnitParameter`：

| 类别 | 需读取的桥字段 | 用途 |
| --- | --- | --- |
| 流程关口 | `in_battle`、`task_class`、`in_talk_event`、`in_tutorial`、`battle.is_event`、`battle.is_ui_visible`、`battle.working_job_count`、`battle.result` | 区分可操作、需按确认推进、等待动画 |
| 回合与顺序 | `battle.phase`、`battle.current_actor_uid`、`battle.timeline`、`battle.risk_uids` | 判断谁行动、是否处于 `Additional` 爆发期、敌人在我方几动后出手 |
| 我方状态 | `units[relation==1]` 的 `hp/max_hp`、`mp/max_mp`、`is_dead`、`lb/lb_max/lb_level`、`in_ex_phase` | 血线危急度、蓝量预算、极限技是否就绪 |
| 敌方状态 | `units[relation==2 && !is_dead]` 的 `uid`、`unit_id`、`is_boss`、`hp/max_hp`、`bp/max_bp`、`is_break`、`is_risky`、`weaknesses` | 弱点匹配、剩余破防槽估算、高危威胁识别 |
| 菜单与目标 | `battle.cmd.{display, state, ancestors, focus_index, focus_action_id, primary_ids, secondary_ids, target}` | 按键导航与选招/选目标闭环校验 |

### 2.2 可操作前置关口（每次决策前先判）

按以下顺序决定本拍动作，**不满足可操作关口绝不发战斗指令键**：

1. **`in_tutorial == true` 或 `in_talk_event == true`**：若为弹窗/标准对话框，按确认推进；若为纯镜头拉近/字幕播片，等待其自然结束或按需推进。
2. **`battle.result ~= null`（`task_class == "CPP_BtlTaskWin"` 且结算未结束）**：处于报酬页/成长页/升级页，只发确认键推进至退出战斗。
3. **`battle.is_event == true` 或 `battle.is_ui_visible == false` 或 `battle.working_job_count > 0` 或 `battle.cmd.display == false`**：处于技能动画、极限技演出、全面破防过场或回合切换间隙，纯等待，不按键。
4. **仅当以上均否且 `battle.cmd.display == true` 且 `battle.current_actor_uid >= 0`**：进入下方规则表选择本回合行动。

### 2.3 规则表行动优先级（通用策略框架）

以当前行动者 `actor = units[current_actor_uid]` 评估候选动作，按优先级从高到低匹配：

| 优先级 | 触发条件（读桥状态） | 决策动作 | 依据的规则与考量 |
| --- | --- | --- | --- |
| **P1 救急/保命** | 有存活我方 `hp / max_hp < 危急阈值`（如 30%），或有我方 `is_dead == true` 且持有复活手段 | 优先使用回复招式或回复/复活道具（多成员残血且有 `can_aoe` 回复魔法时切全体化）；若当前角色无回复能力且自身残血，选防御（`400010`） | 防止减员导致全灭；回复优先于破防与输出 |
| **P2 打断危险行动** | 存在存活且未破防的高危敌人（`uid ∈ battle.risk_uids` 或 `is_risky == true`） | 统计 `battle.timeline` 中该高危敌人出手前我方剩余行动次数与预计破防削减量（查 `DT_SkillData.breakDamageValue` 与弱点匹配）：<br>• **可破防**：集中对该目标使用最高效的弱点/高 `breakDamageValue` 招式，强行打断危险行动；<br>• **不可破防**：若该目标下一动就会出手（且为群攻或威胁致命），改为选择**防御（`400010`）**减伤承伤 | 规则二：破防中止危险行动，破不掉则防御保命 |
| **P3 破防期 / 追加阶段爆发** | `battle.phase == "Additional"` 或场上存在 `is_break == true` 的存活敌人 | 敌方已破防承伤提高，优先倾泻高倍率输出招式；若 `actor.lb_level >= 1`（极限槽满）且目标是高血量敌人/Boss，释放极限技（`Cmd[10]`）；低血量杂兵则用普攻或低耗蓝技能收尾以省 MP | 规则一 & 三：破防增伤窗口 + 追加阶段白嫖行动；避免对濒死杂兵浪费 MP/极限槽 |
| **P4 削减破防槽 / 争取全面破防** | 场上存在未破防敌人且当前角色具备命中其弱点（`weaknesses` 含 `Weakness`）或高 `breakDamageValue` 招式 | 优先集火最接近破防（`bp` 最低）或本回合将在我方后行动（位于 `timeline` 靠前）的未破防敌人，使用弱点招式削减破防槽；敌方为多名同弱点杂兵且魔法支持 `can_aoe` 时考虑全体化 | 规则一 & 三：取消敌方本回合行动并触发追加行动/全面破防 |
| **P5 常规输出与资源控制** | 无上述高优条件，或 `actor.mp` 低于保留阈值 | 对血量最低或行动靠前的存活敌人执行普通攻击（`primary_ids[0]` 战斗）或低耗招式；敌方对某属性减半/无效/吸收（`ElemResistDataList` 非 `None`/`Weakness`）时禁止选用该属性招式 | 保留 MP 与道具用于 Boss 或危急回合，同时避免打中耐性浪费回合 |

> 逃跑（`400020`）原则上不由战斗规则表主动触发，仅在顶层规划器显式下达撤退策略且当前战斗组 `DT_BattleGroup.canEscape == true` 时启用。

### 2.4 操作执行闭环约束

执行器把规则表选出的目标动作 `(target_action_id, target_uid, want_all)` 映射为按键序列时，必须依赖桥回读做闭环而非盲按步数：

1. **一级菜单导航**（`cmd.state == "List"` 且 `#cmd.ancestors == 0`）：核对 `cmd.primary_ids` 定位目标项（或父分类如 `1`=招式、`7`=道具、`10`=极限技），逐次按上/下并确认 `cmd.focus_action_id` 命中后再按确认。
2. **二级列表导航**（`cmd.state == "List"` 且 `#cmd.ancestors >= 1`）：核对 `cmd.secondary_ids` 包含 `target_action_id`（若不在或蓝量不足则按取消退回并降级规则），导航至 `cmd.focus_action_id == target_action_id` 后确认。
3. **目标选择闭环**（`cmd.state == "Target"`）：若 `want_all && cmd.target.can_aoe && !cmd.target.is_all` 先切全体化并回读 `cmd.target.is_all == true`；否则按方向键并回读 `cmd.target.focus_uid == target_uid`（防死循环设最大步数为 `#cmd.target.all_target_uids + 1`）后再按确认提交指令。
4. **提交确认**：确认后等待 `cmd.display == false` 或 `working_job_count > 0` 或 `current_actor_uid` 变化，视为本步指令已被游戏接收。
