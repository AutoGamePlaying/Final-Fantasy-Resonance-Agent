# 由计划任务 FFR-Agent-Session 在用户的桌面会话里运行。
# 读取 <Root>\session\request.json,启动其中的 Steam URL 或程序,把状态写到 <id>.status.json。
#
# request.json 的字段:
#   id          本次请求的编号,开发机据此找到状态文件
#   file        要启动的程序或 URL(例如 steam://rungameid/<AppId>)
#   args        参数数组,可选
#   cwd         工作目录,可选
#   wait        为 true 时等程序结束,记录退出码(可选)
#
# 状态文件的 state 是 running / started / done / error。写文件用 UTF-8 无 BOM,
# 开发机用 Get-Content -Encoding UTF8 读取。
param(
    [Parameter(Mandatory = $true)]
    [string]$Root
)
$ErrorActionPreference = 'Stop'

$dir = Join-Path $Root 'session'
$requestPath = Join-Path $dir 'request.json'
$req = Get-Content -LiteralPath $requestPath -Raw -Encoding UTF8 | ConvertFrom-Json
$statusPath = Join-Path $dir "$($req.id).status.json"

function Save-Status([hashtable]$Status) {
    $Status.id = $req.id
    $Status.time = (Get-Date).ToString('o')
    $json = $Status | ConvertTo-Json -Compress
    [IO.File]::WriteAllText($statusPath, $json, (New-Object Text.UTF8Encoding $false))
}

Save-Status @{ state = 'running' }
try {
    if (-not $req.file) { throw 'request.json 里没有 file' }

    # URL launches (steam://...) hand off to an already running program; there is no process
    # to track and -PassThru behaves inconsistently for them on Windows PowerShell 5.1.
    if ($req.file -match '^[a-zA-Z][a-zA-Z0-9+.-]*://') {
        Start-Process -FilePath $req.file
        Save-Status @{ state = 'started' }
        return
    }

    $params = @{ FilePath = $req.file; PassThru = $true }
    if ($req.PSObject.Properties['args'] -and $req.args) {
        $params.ArgumentList = @($req.args)
    }
    if ($req.PSObject.Properties['cwd'] -and $req.cwd) {
        $params.WorkingDirectory = $req.cwd
    }
    $wait = [bool]($req.PSObject.Properties['wait'] -and $req.wait)
    $proc = Start-Process @params
    # Windows PowerShell 5.1 reports ExitCode as null unless the handle was opened before exit.
    $null = $proc.Handle
    if ($wait) {
        $proc.WaitForExit()
        Save-Status @{ state = 'done'; pid = $proc.Id; exit_code = $proc.ExitCode }
    } else {
        Save-Status @{ state = 'started'; pid = $proc.Id }
    }
} catch {
    Save-Status @{ state = 'error'; error = $_.Exception.Message }
}
