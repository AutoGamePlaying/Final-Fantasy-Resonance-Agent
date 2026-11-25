# ffr-export：R2 数据表导出工具

`ffr-export` 用 CUE4Parse 只读挂载《FINAL FANTASY RESONANCE》的 IoStore / Pak 资源，支持列出文件、保存单个包的原始字节，以及在取得 `.usmap` 后把包内全部导出对象序列化成 JSON。

## 前置条件

在仓库根目录安装 .NET 10 SDK（不要改用系统里的旧版 SDK）：

```bash
mkdir -p .deps
curl -fsSL https://dot.net/v1/dotnet-install.sh -o .deps/dotnet-install.sh
bash .deps/dotnet-install.sh --channel 10.0 --install-dir .deps/dotnet --no-path
```

把游戏安装目录放在 `.deps/ffr/files/`（也可以是指向它的软链接）。默认资源目录是 `.deps/ffr/files/FFRS/Content/Paks`，工具只读取该目录，绝不向其中写入。

`list`、`raw` 和 `locres` 不需要映射文件。`export` 需要 R1 在游戏机上用 UE4SS 导出的 `.usmap`，取得方法见 [docs/r1-dump.md](../../docs/r1-dump.md)。

## 命令

所有命令都通过仓库根目录下的 `tools/ffr-export/run.sh` 运行；脚本会自行定位仓库根目录。

### 列出资源

```bash
tools/ffr-export/run.sh list [--paks <目录>] [--prefix <前缀>]...
```

逐行打印匹配路径和字节数，最后打印总数。默认前缀是 `FFRS/Content/Datatable/` 和 `FFRS/Content/L10N/`；指定一个或多个 `--prefix` 后只使用指定前缀。

### 保存原始包字节

```bash
tools/ffr-export/run.sh raw FFRS/Content/Datatable/Scenario/DT_ChapterData.uasset \
  --out .deps/tmp/DT_ChapterData.bin
```

打印字节长度和 SHA-256，便于与其他解码器交叉核对。`--out` 必须位于 `.deps/` 下。

### 导出 JSON

```bash
tools/ffr-export/run.sh export --usmap .deps/r1/<任务号>/Mappings.usmap \
  [--out .deps/ffr/export] [--prefix <前缀>]... [--paks <目录>] [--force]
```

每个匹配前缀的 `.uasset` 导出为 `<out>/<包路径去掉扩展名>.json`，JSON 缩进两个空格、UTF-8 编码。单个包失败不会中断其余包；最后写出 `<out>/manifest.json`，记录 CUE4Parse 版本、映射文件 SHA-256、起止时间、成功数、失败数和失败详情。任一失败时退出码为 1。

输出目录非空时命令会拒绝执行；`--force` 也只清空包含本工具 `manifest.json`（带 `cue4ParseVersion` 字段且属于同一子命令）的旧输出目录，并在清空前打印目录和文件数。游戏文件、R1 转储、SDK 和 NuGet 缓存等受保护目录不能作为输出目录。所有输出只能放在 `.deps/` 下，`.deps/` 整体不进版本库。

### 导出多语言文本（`.locres`）

```bash
tools/ffr-export/run.sh locres [--lang <语言>]... [--out .deps/ffr/export/Localization] \
  [--paks <目录>] [--force]
```

从游戏包的 `FFRS/Content/Localization/Game/<语言>/Game.locres` 读取 UE 本地化资源（通过 CUE4Parse 的 `FTextLocalizationResource`），导出为 `<out>/<语言>.json`，JSON 格式为 `{ namespace: { key: text } }`（缩进两空格、UTF-8 无 BOM），并写出 `<out>/manifest.json` 记录 `cue4ParseVersion`、起止时间以及各语言的语言代码、源包路径、源文件 SHA-256、命名空间数和词条数。

不传 `--lang` 时默认导出 `zh-Hans`、`en`、`ja`；可重复传入 `--lang` 或用逗号分隔指定其他语言（例如 `--lang zh-Hans --lang zh-Hant` 或 `--lang zh-Hans,en,ja`）。不需要 `.usmap`。`--out` 必须位于 `.deps/` 下且避开受保护目录；输出目录非空时拒绝执行，`--force` 仅清空包含本工具 `locres` manifest 的旧目录，不会误清 `export` 导出的数据表目录。

## Oodle 与联网

Oodle 使用 CUE4Parse 默认的开源纯 C# 解码器 `OodleSharp`。工具不初始化或下载专有 Oodle 库，也不在资源读取、解码和导出过程中发起任何网络请求。
