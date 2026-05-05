# 三术 Windows 安装脚本

param(
    [switch]$BuildOnly = $false
)

$ErrorActionPreference = "Stop"

Write-Host "🚀 开始安装 三术 (Windows)..." -ForegroundColor Green

# 检查必要的工具
function Test-Command {
    param($Command)
    try {
        Get-Command $Command -ErrorAction Stop | Out-Null
        return $true
    }
    catch {
        return $false
    }
}

Write-Host "🔧 检查必要工具..." -ForegroundColor Yellow

if (-not (Test-Command "cargo")) {
    Write-Host "❌ 错误: 未找到 cargo 命令" -ForegroundColor Red
    Write-Host "请先安装 Rust: https://rustup.rs/" -ForegroundColor Red
    exit 1
}

if (-not (Test-Command "pnpm")) {
    Write-Host "❌ 错误: 未找到 pnpm 命令" -ForegroundColor Red
    Write-Host "请先安装 pnpm: npm install -g pnpm" -ForegroundColor Red
    exit 1
}

# 构建前端
Write-Host "📦 构建前端资源..." -ForegroundColor Yellow
pnpm build

# 构建二进制文件
Write-Host "🔨 构建二进制文件..." -ForegroundColor Yellow
cargo build --release

# 检查构建结果
$UiBinaryPath = "target\release\等一下.exe"
$McpBinaryPath = "target\release\三术.exe"
foreach ($PathToCheck in @($UiBinaryPath, $McpBinaryPath)) {
    if (-not (Test-Path $PathToCheck)) {
        Write-Host "❌ 二进制文件构建失败: $PathToCheck" -ForegroundColor Red
        exit 1
    }
}

Write-Host "✅ 二进制文件构建成功:" -ForegroundColor Green
Write-Host "   UI:  $UiBinaryPath" -ForegroundColor Green
Write-Host "   MCP: $McpBinaryPath" -ForegroundColor Green

# 如果只构建不安装，则在这里退出
if ($BuildOnly) {
    Write-Host ""
    Write-Host "🎉 三术 构建完成！" -ForegroundColor Green
    Write-Host ""
    Write-Host "📋 UI 二进制文件位置:  $UiBinaryPath" -ForegroundColor Cyan
    Write-Host "📋 MCP 二进制文件位置: $McpBinaryPath" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "如需安装，请重新运行脚本而不使用 -BuildOnly 参数。"
    exit 0
}

# 创建安装目录
$LocalAppData = $env:LOCALAPPDATA
$InstallDir = "$LocalAppData\三术"
$BinDir = "$InstallDir\bin"

Write-Host "📁 创建安装目录: $InstallDir" -ForegroundColor Yellow
New-Item -ItemType Directory -Path $BinDir -Force | Out-Null

# 复制二进制文件
$AsciiMcpExe = "$BinDir\sanshu.exe"
$UiExe = "$BinDir\等一下.exe"
$McpExe = "$BinDir\三术.exe"

Write-Host "📋 安装二进制文件..." -ForegroundColor Yellow
# 中文说明：sanshu.exe 是三术.exe 的 ASCII 兼容副本，供不稳定支持中文命令的 MCP 客户端使用。
Copy-Item $UiBinaryPath $UiExe -Force
Copy-Item $McpBinaryPath $McpExe -Force
Copy-Item $McpBinaryPath $AsciiMcpExe -Force

Write-Host "✅ 二进制文件已安装到: $BinDir" -ForegroundColor Green

# 图标已移除，不再需要复制

# 检查PATH环境变量
$CurrentPath = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($CurrentPath -notlike "*$BinDir*") {
    Write-Host "🔧 添加到用户 PATH 环境变量..." -ForegroundColor Yellow
    
    try {
        $NewPath = if ($CurrentPath) { "$CurrentPath;$BinDir" } else { $BinDir }
        [Environment]::SetEnvironmentVariable("PATH", $NewPath, "User")
        Write-Host "✅ 已添加到 PATH: $BinDir" -ForegroundColor Green
        Write-Host "💡 请重新启动命令提示符或 PowerShell 以使 PATH 生效" -ForegroundColor Cyan
    }
    catch {
        Write-Host "⚠️  无法自动添加到 PATH，请手动添加: $BinDir" -ForegroundColor Yellow
    }
} else {
    Write-Host "✅ PATH 已包含安装目录" -ForegroundColor Green
}

# 创建开始菜单快捷方式
$StartMenuDir = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs"
$ShortcutPath = "$StartMenuDir\三术.lnk"

try {
    $WshShell = New-Object -ComObject WScript.Shell
    $Shortcut = $WshShell.CreateShortcut($ShortcutPath)
    $Shortcut.TargetPath = $UiExe
    $Shortcut.WorkingDirectory = $InstallDir
    $Shortcut.Description = "三术 - 道生一，一生二，二生三，三生万物"
    # 图标已移除，使用默认图标
    $Shortcut.Save()
    Write-Host "✅ 开始菜单快捷方式已创建" -ForegroundColor Green
}
catch {
    Write-Host "⚠️  无法创建开始菜单快捷方式: $($_.Exception.Message)" -ForegroundColor Yellow
}

Write-Host ""
Write-Host "🎉 三术 安装完成！" -ForegroundColor Green
Write-Host ""
Write-Host "📋 使用方法：" -ForegroundColor Cyan
Write-Host "  🖥️  GUI模式: 从开始菜单打开 '三术'" -ForegroundColor White
Write-Host "  💻 命令行模式:" -ForegroundColor White
Write-Host "    等一下                          - 启动 UI 界面" -ForegroundColor White
Write-Host "    等一下 --mcp-request file       - MCP 弹窗模式" -ForegroundColor White
Write-Host "    sanshu                          - 启动 MCP 服务器（推荐，ASCII 兼容入口）" -ForegroundColor White
Write-Host "    三术                            - 启动 MCP 服务器（中文兼容入口）" -ForegroundColor White
Write-Host ""
Write-Host "📝 配置 MCP 客户端：" -ForegroundColor Cyan
Write-Host "将以下内容添加到您的 MCP 客户端配置中：" -ForegroundColor White
Write-Host ""
$McpClientConfig = @'
{
  "mcpServers": {
    "sanshu": {
      "command": "sanshu"
    }
  }
}
'@
Write-Host $McpClientConfig -ForegroundColor Gray
Write-Host ""
Write-Host "📁 安装位置: $InstallDir" -ForegroundColor Cyan
Write-Host "🔗 命令行工具: $BinDir" -ForegroundColor Cyan
Write-Host ""
Write-Host "💡 如果命令行工具无法使用，请重新启动命令提示符或 PowerShell" -ForegroundColor Yellow
