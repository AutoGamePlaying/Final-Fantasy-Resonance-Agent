# FfrBridge

只读文件桥模组（v0）：每 500 毫秒在游戏线程读取已验证的游戏状态，按 AGP 文件桥协议 v1 写入 `state.json`，并处理 `command.json`（v0 支持 `ping`）。模组不修改游戏数据，也不模拟按键。

在开发机启用：

```sh
node src/host/cli/mod-sync.ts FfrBridge
```

停用：

```sh
node src/host/cli/mod-sync.ts FfrBridge --disable
```

然后重启游戏。桥目录位于游戏 `Win64` 目录下的 `ue4ss/ffr-out/bridge/`（由 AGP 一侧创建；目录不存在时模组什么都不写，也不刷日志）。

字段说明与共存约定见 [`docs/bridge.md`](../../docs/bridge.md)。
