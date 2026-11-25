<#
.SYNOPSIS
    注册「只在你登录时运行」的计划任务,让开发机能通过 SSH 在你的桌面上启动游戏。

.DESCRIPTION
    开发机用本地普通账号登录 SSH,它启动的程序不在你的桌面会话里,游戏窗口不会出现。
    这个计划任务以你的身份、在你的桌面会话里运行 run-in-session.ps1:开发机先写好请求文件,
    再用 schtasks /run 触发它。

    由你本人在「以管理员身份运行」的 PowerShell 里运行(首次和每次更新本脚本后)。可以重复执行。

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File C:\ffr\deploy\laptop\register-session-task.ps1 -AllowUser ffr
#>
#Requires -RunAsAdministrator
[CmdletBinding()]
param(
    [string]$TaskName = 'FFR-Agent-Session',
    [Parameter(Mandatory = $true)]
    [string]$AllowUser,
    [string]$Root = ''
)
$ErrorActionPreference = 'Stop'

if (-not $Root) {
    # run-in-session.ps1 is expected at <Root>\deploy\laptop\.
    $Root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
}
$runner = Join-Path $PSScriptRoot 'run-in-session.ps1'
if (-not (Test-Path -LiteralPath $runner -PathType Leaf)) {
    throw "找不到 $runner"
}

$ps = Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\powershell.exe'

# Windows 11's default terminal is Windows Terminal, which ignores
# `powershell.exe -WindowStyle Hidden`. conhost.exe --headless (build 17763+) gives PowerShell a
# console but no window, and Windows Terminal does not take it over. Older systems keep the old
# action.
$conhost = Join-Path $env:SystemRoot 'System32\conhost.exe'
$build = 0
try {
    $build = [int](Get-ItemProperty -Path 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' `
        -Name CurrentBuildNumber -ErrorAction Stop).CurrentBuildNumber
} catch {
    $build = [Environment]::OSVersion.Version.Build
}
if ((Test-Path $conhost) -and ($build -ge 17763)) {
    $action = New-ScheduledTaskAction -Execute $conhost `
        -Argument "--headless `"$ps`" -NoProfile -ExecutionPolicy Bypass -File `"$runner`" -Root `"$Root`""
} else {
    $action = New-ScheduledTaskAction -Execute $ps `
        -Argument "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$runner`" -Root `"$Root`""
    Write-Host '这台系统没有 conhost.exe 或版本低于 Windows 10 1809(build 17763),计划任务仍用原来的写法,触发时可能会闪一下窗口。'
}
$principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" `
    -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
    -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew
Register-ScheduledTask -TaskName $TaskName -Action $action -Principal $principal `
    -Settings $settings -Force | Out-Null

# Let $AllowUser start the task: GENERIC_READ | GENERIC_EXECUTE on its DACL, nothing more.
$sid = ([Security.Principal.NTAccount]$AllowUser).Translate([Security.Principal.SecurityIdentifier])
$service = New-Object -ComObject Schedule.Service
$service.Connect()
$task = $service.GetFolder('\').GetTask($TaskName)
$dacl = New-Object Security.AccessControl.RawSecurityDescriptor ($task.GetSecurityDescriptor(4))
if (-not @($dacl.DiscretionaryAcl | Where-Object { $_.SecurityIdentifier -eq $sid })) {
    $readExecute = -1610612736  # 0xA0000000
    $ace = New-Object Security.AccessControl.CommonAce 'None', 'AccessAllowed', $readExecute, $sid, $false, $null
    $dacl.DiscretionaryAcl.InsertAce($dacl.DiscretionaryAcl.Count, $ace)
    $task.SetSecurityDescriptor($dacl.GetSddlForm('Access'), 0)
}

New-Item -ItemType Directory -Force -Path (Join-Path $Root 'session') | Out-Null
Write-Host "已注册计划任务 $TaskName,并允许 $AllowUser 启动它"
