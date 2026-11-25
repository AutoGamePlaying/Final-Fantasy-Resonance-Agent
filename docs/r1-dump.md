# R1 只读转储：模组 FfrDump 和开发机命令

R1 要在几个场面（标题、野外、对话、战斗、菜单、存档）各取一份游戏状态，离线找出队伍、敌人、回合、菜单光标、地图和坐标、剧情进度分别在哪个类、哪个属性。本文是转储模组 `mod/FfrDump/` 和开发机命令 `dump-run` / `dump-fetch` / `dump-status` 之间的约定。两边都按本文实现，改协议要两边一起改。

## 1. 流程

1. 用户在游戏里走到要取的场面，停住不动。
2. 开发机运行 `node src/host/cli/dump-run.ts <任务> --label <场面名>`。
3. 命令经 SSH 在游戏机上写一个触发文件；模组每秒查一次，看到后在游戏线程里执行任务，写出结果和完成标记。
4. 命令等到完成标记，再把这次任务产生的文件打包拉回开发机的 `.deps/r1/<任务号>/`。

游戏机上不按任何键；模组只读游戏状态，唯一的写操作是往自己的输出目录写文件。

## 2. 目录

`<游戏目录>` 指游戏的 `Win64` 目录（`machine.env` 的 `FFR_GAME_DIR`）。

| 路径 | 谁写 | 内容 |
| --- | --- | --- |
| `<游戏目录>/ue4ss/ffr-out/` | 命令建 | 输出根目录（下称 `OUT`）。不放在 `ue4ss/Mods/FfrDump/` 里，因为 `mod-sync` 会整个替换模组目录 |
| `OUT/heartbeat.json` | 模组 | 心跳，每 5 秒写一次 |
| `OUT/trigger.txt` | 命令 | 触发文件，一次只能有一个 |
| `OUT/jobs/<任务号>/` | 命令建，模组写 | 这次任务的输出和完成标记 |
| `OUT/hotkey/` | 命令建，模组写 | 热键快照（`F9`）、时间标记 `markers.jsonl`（`F10`）和序号记录 `seq.txt`（第 8 节） |
| `<开发机仓库>/.deps/r1/<任务号>/` | 命令 | 拉回来的结果，不进版本库 |
| `<开发机仓库>/.deps/r1/hotkey/` | 命令 | `dump-fetch-hotkey` 增量拉回并整理好的热键快照与时间标记，不进版本库 |

模组不会建目录（Lua 没有 mkdir，也不许调用外部程序），所以 `OUT`、`OUT/hotkey/` 和 `OUT/jobs/<任务号>/` 都由命令预先建好（`mod-sync FfrDump` 和 `dump-run` 都会确保 `OUT/hotkey/` 存在）。`OUT` 不存在时模组写不了心跳，属于正常情况：命令建好 `OUT` 后最多等 15 秒等心跳出现。

## 3. 任务号、任务和场面名

- **场面名** `label`：`^[a-z0-9-]{1,32}$`，例如 `title`、`field-1`、`battle-boss`。
- **任务** `job`，只有这 6 种：

  | job | 做什么 | 输出位置 | 默认等待 |
  | --- | --- | --- | --- |
  | `snapshot` | 模组自己的场面快照（第 5 节） | `OUT/jobs/<任务号>/` | 180 秒 |
  | `objects` | UE4SS `DumpAllObjects()` | UE4SS 默认位置 | 300 秒 |
  | `usmap` | UE4SS `DumpUSMAP()`：解析 IoStore 资源要用的映射文件 | UE4SS 默认位置 | 180 秒 |
  | `jmap` | UE4SS `DumpJMAP()`：JSON 格式的反射数据（类、属性、偏移） | UE4SS 默认位置 | 600 秒 |
  | `sdk` | UE4SS `GenerateSDK()` | UE4SS 默认位置 | 900 秒 |
  | `uht` | UE4SS `GenerateUHTCompatibleHeaders()` | UE4SS 默认位置 | 900 秒 |

- **任务号** `id`：`<UTC 日期>-<UTC 时间>-<label>-<job>`，例如 `20261124-031500-field-1-snapshot`。开发机生成；格式 `^\d{8}-\d{6}-[a-z0-9-]{1,32}-(snapshot|objects|usmap|jmap|sdk|uht)$`。它会被拼进路径，两边都要校验。

UE4SS 自己的转储写到哪里由 UE4SS 决定（`ue4ss/` 下或 `Win64/` 下，各版本不同），模组不管；`dump-fetch` 按修改时间去收（第 6 节）。

## 4. 文件格式

**心跳** `OUT/heartbeat.json`（模组写，先写 `heartbeat.tmp` 再改名）：

```json
{"mod":"FfrDump","version":1,"time":1795000000,"busy":false}
```

`time` 是游戏机上的 Unix 秒数（`os.time()`）。任务在游戏线程里跑的时候心跳可能停，命令在等完成标记期间不看心跳。

**触发文件** `OUT/trigger.txt`（命令写，先写 `trigger.tmp` 再改名为 `trigger.txt`）：三行 `键=值`，换行用 `\r\n` 或 `\n` 都要能读：

```text
id=20261124-031500-field-1-snapshot
job=snapshot
label=field-1
```

模组读到后先删掉 `trigger.txt`，再执行。内容不合法（键缺失、格式不符、`job` 和 `id` 的结尾不一致、`OUT/jobs/<id>/` 不存在）时：删掉触发文件；能取到合法 `id` 且目录存在，就写一个 `ok:false` 的完成标记，否则只在 UE4SS 日志里记一行。

**完成标记** `OUT/jobs/<id>/done.json`（模组写，先写 `done.tmp` 再改名）：

```json
{
  "id": "20261124-031500-field-1-snapshot",
  "job": "snapshot",
  "label": "field-1",
  "ok": true,
  "error": null,
  "startedAt": 1795000000,
  "finishedAt": 1795000012,
  "files": ["context.json", "classes.json", "objects.jsonl", "widgets.jsonl", "texts.jsonl"],
  "stats": { "objectsScanned": 412345, "objectsDumped": 3120, "truncated": false }
}
```

`files` 只列模组自己写进 `OUT/jobs/<id>/` 的文件（相对路径）；UE4SS 自己的转储不列。`ok:false` 时 `error` 是中文或英文原文的错误说明。

## 5. `snapshot` 的内容

全部在一次 `ExecuteInGameThread` 里完成，游戏会卡几秒到几十秒，属于正常现象。

- `context.json`：当前 World 的全名、PlayerController 和 Pawn 的类和名字、Pawn 的位置和朝向、GameInstance / GameMode / GameState 的类、时间。
- `classes.json`：所有存活对象按类计数（跳过 `Default__` 开头的 CDO）：类全名、所在包、父类链、存活实例数、写进 `objects.jsonl` 的实例数。
- `objects.jsonl`：“游戏自己的类”的实例，一行一个对象。“游戏自己的类”指父类链上有一个类在 `/Script/mucha` 包里，或路径以 `/Game/` 开头（蓝图类）。只写这些类自己声明的属性，引擎类声明的属性不写。每个类最多 50 个实例，总共最多 20000 个。
- `widgets.jsonl`：所有 `UserWidget` 实例：名字、类、`IsInViewport()`、`Visibility`。最多 2000 个。
- `texts.jsonl`：所有 `TextBlock`、`RichTextBlock` 实例：全名（能看出属于哪个界面）、`Text`、`Visibility`。最多 2000 个。

属性值的写法：

| 属性类型 | JSON |
| --- | --- |
| 整数、浮点 | 数字；NaN、无穷写成字符串 |
| 布尔 | `true` / `false` |
| `Str`、`Name`、`Text` | 字符串，超过 512 字符截断并以 `…` 结尾 |
| 枚举（`EnumProperty`，或带枚举的 `ByteProperty`） | `{"enum":"<枚举全名>","value":<数字>,"name":"<值名，取不到就省略>"}` |
| 对象引用（`Object`、`WeakObject`、`SoftObject`、`Class` 等） | 被引用对象的全名，空引用写 `null`；不往下展开 |
| 结构 | 字段对象，最多 3 层，再深写 `"<depth>"` |
| 数组 | `{"len":<长度>,"items":[前 32 个]}` |
| `Map`、`Set` | `{"len":<长度>,"items":[[键,值], …前 32 个]}`；读不出来写 `"<Map>"` |
| 委托、接口和其他 | `"<类型名>"` |
| 读取出错 | `"<error: 错误说明>"`（截断到 200 字符），不中断整份快照 |

## 6. `dump-fetch` 收哪些文件

1. `OUT/jobs/<id>/` 下的全部文件。
2. `<游戏目录>` 下（递归，不含 `ue4ss/Mods/`、`ue4ss/ffr-out/`）修改时间落在 `[startedAt - 2 秒, finishedAt + 30 秒]` 的文件，`UE4SS.log` 除外：这就是 UE4SS 这次写出的转储。
3. 一份 `UE4SS.log` 的拷贝（共享读）。
4. `meta.json`：任务号、任务、场面名、起止时间、文件数、总字节数、游戏版本 `buildid`（读 Steam 的 `appmanifest_<appid>.acf`，读不到写 `null`）。

在游戏机上用系统自带的 `tar.exe` 打成一个 `.tgz`，包里的路径相对 `<游戏目录>`。拷回开发机后解开到 `.deps/r1/<id>/`，再删掉两边的压缩包。UE4SS 的转储原件留在游戏机上不删。

本地解开后，第 1、3、4 项（任务目录里的文件、`UE4SS.log`、`meta.json`）挪到 `.deps/r1/<id>/` 的根上；UE4SS 的转储保持相对 `<游戏目录>` 的路径，例如 `.deps/r1/<id>/ue4ss/UE4SS_ObjectDump.txt`。`meta.json` 里的 `buildid` 在开发机上回填。

## 7. 规矩

- 模组只读：不给任何属性赋值，不模拟按键，不改存档。除了 UE4SS 的遍历和取名函数外，只调用这几个纯读取函数：`K2_GetActorLocation`、`K2_GetActorRotation`、`IsInViewport`。
- 每次读取都要用 `pcall` 包住，并处理对象失效（换图后旧引用不能用）。
- 一次只跑一个任务。`trigger.txt` 已经存在时，命令拒绝再写。
- 转储（对象表、头文件、映射文件、快照）只放在 `.deps/`，不进版本库。

## 8. 热键

为了在开宝箱、切画面等几秒钟的瞬间零延迟取样，`FfrDump` 通过 UE4SS 的 `RegisterKeyBind` 注册两个游戏内热键（只被动监听按键，不向游戏发送任何按键，不改游戏状态）：

| 热键 | 作用 | 输出 |
| --- | --- | --- |
| `F9` | 立即取一份 `snapshot` 场面快照，场面名 `hotkey-<4位序号>`（如 `hotkey-0003`） | 写到 `OUT/hotkey/`，文件名带 `<id>-` 前缀 |
| `F10` | 只打一个时间标记，不取快照，不阻塞正在进行的快照 | 追加一行 JSON 到 `OUT/hotkey/markers.jsonl` |

按键冲突核实：已检查解包后的全部 168 个 `FFRS/Content/Input/` 资产（含全部 `IMC_*_KBM` 键鼠输入映射）以及 `FFRS/Config/DefaultInput.ini`，游戏本身未绑定 `F9` 和 `F10`。（注意：UE4SS 自带的 `ConsoleEnablerMod` 默认可能绑定 `F10`，本项目的 `mods.txt` 不启用该模组。）

### 输出目录和文件格式

Lua 不能建目录，`OUT/hotkey/` 由 `mod-sync FfrDump` 和 `dump-run` 在游戏机上预先建好。序号保存在 `OUT/hotkey/seq.txt`（`snapshot=<n>`、`marker=<m>`），重启游戏后继续递增。

- **F9 快照**：复用 `snapshot.lua` 并走分批机制（`LoopAsync` + `ExecuteInGameThread`）。任务号 `id` 格式为 `hotkey-<4位序号>-<UTC 日期>-<UTC 时间>`（例如 `hotkey-0003-20261125-023045`），产出文件带 `<id>-` 前缀：
  - `<id>-context.json`
  - `<id>-classes.json`
  - `<id>-objects.jsonl`
  - `<id>-widgets.jsonl`
  - `<id>-texts.jsonl`
  - `<id>-done.json`（最后原子写入，格式与第 4 节 `done.json` 一致，其中 `job` 为 `"snapshot"`、`label` 为 `"hotkey-0003"`）
- **正在取时忽略**：如果当前 `busy` 为 `true`（正在执行 `trigger.txt` 任务或上一份热键快照），再按 `F9` 会直接忽略并在 `UE4SS.log` 写一行 `[FfrDump] hotkey F9 ignored: ...`。
- **F10 时间标记**：按键时当场记录本地时间和 `clock_ms`，并在游戏线程读取当前 `World` 完整名（取不到写 `"<unknown>"`），向 `OUT/hotkey/markers.jsonl` 追加一行 JSON：

```json
{"clock_ms":12345,"localTime":"2026-11-25 02:30:45","seq":1,"time":1795458645,"world":"World /Game/Maps/Field01.Field01"}
```

字段说明：`seq` 为从 1 递增的标记序号，`localTime` 为游戏机本地时间（`YYYY-MM-DD HH:MM:SS`），`time` 为 Unix 秒数，`clock_ms` 为模组加载后的毫秒计数，`world` 为当前世界完整名。

### 开发机命令

- `node src/host/cli/mod-sync.ts FfrDump` 与 `node src/host/cli/dump-run.ts ...`：执行时都会确保游戏机上 `OUT/hotkey/` 存在。
- `node src/host/cli/dump-status.ts`：额外显示 `OUT/hotkey/` 中已完成的热键快照数量（按 `*-done.json` 计数）和 `markers.jsonl` 的标记条数。
- `node src/host/cli/dump-fetch-hotkey.ts [--dry-run] [--machine-env <文件>]`：增量拉取 `OUT/hotkey/` 中尚未拉取的已完成快照和新增时间标记到 `.deps/r1/hotkey/`：
  - 每份已完成快照按序号整理到 `.deps/r1/hotkey/<id>/`，去掉文件名前缀并补写 `meta.json`，目录结构与普通 `snapshot` 一致（`context.json`、`classes.json`、`objects.jsonl`、`widgets.jsonl`、`texts.jsonl`、`done.json`、`meta.json`）；
  - 时间标记按字节偏移增量追加到 `.deps/r1/hotkey/markers.jsonl`（只读取到最后一个完整换行符，避免读到半行）；
  - 已拉取的快照号和标记偏移保存在 `.deps/r1/hotkey/fetched.json`；游戏机上的 `OUT/hotkey/` 原件不删除，只在拉完后删掉临时压缩包。
