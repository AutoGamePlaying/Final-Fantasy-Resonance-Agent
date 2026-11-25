# FfrDump

R1 只读转储模组：按触发文件生成场面快照，或调用 UE4SS 自带的对象、映射、SDK、头文件转储。模组不修改游戏数据，也不模拟按键。

在开发机启用：

```sh
node src/host/cli/mod-sync.ts FfrDump
```

然后重启游戏。心跳、触发文件和任务完成标记位于游戏 `Win64` 目录下的 `ue4ss/ffr-out/`；快照位于 `ue4ss/ffr-out/jobs/<任务号>/`；热键（`F9` 当场分批快照、`F10` 打时间标记）输出位于 `ue4ss/ffr-out/hotkey/`。UE4SS 自带转储的位置由 UE4SS 决定。

协议、输出格式和只读约束见 [`docs/r1-dump.md`](../../docs/r1-dump.md)。
