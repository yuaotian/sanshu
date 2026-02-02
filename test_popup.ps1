# 三术弹窗测试脚本 (PowerShell 版本)
# 使用 target\release 或 target\debug 中的 CLI 工具测试弹窗功能

param()

# 确保脚本在项目根目录执行，避免相对路径导致资源找不到
Set-Location -Path $PSScriptRoot

$ErrorActionPreference = "Stop"

$script:ProjectRoot = $PSScriptRoot
$script:CliType = "local"   # local 或 global
$script:BuildType = "release"
$script:SanshuExeName = "三术.exe"
$script:DengExeName = "等一下.exe"
$script:CliPath = Join-Path -Path $script:ProjectRoot -ChildPath "target\$($script:BuildType)"

$script:SimplePopupFile = Join-Path -Path $script:ProjectRoot -ChildPath "test_simple_popup.json"
$script:MarkdownPopupFile = Join-Path -Path $script:ProjectRoot -ChildPath "test_markdown_popup.json"

function Test-Command {
    param([string]$Command)
    try {
        Get-Command $Command -ErrorAction Stop | Out-Null
        return $true
    }
    catch {
        return $false
    }
}

function Resolve-CommandPath {
    param([string]$Command)
    $cmd = Get-Command $Command -ErrorAction Stop
    if ($cmd.Path) { return $cmd.Path }
    if ($cmd.Source) { return $cmd.Source }
    return $cmd.Name
}

function Write-Utf8NoBom {
    param(
        [string]$Path,
        [string]$Content
    )
    $utf8NoBom = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText($Path, $Content, $utf8NoBom)
}

function Update-CliPath {
    $script:CliPath = Join-Path -Path $script:ProjectRoot -ChildPath "target\$($script:BuildType)"
}

function Select-BuildType {
    if ($script:CliType -ne "local") {
        return
    }

    Write-Host "🔧 选择构建类型:" -ForegroundColor Yellow
    Write-Host "  1. Release (推荐，性能最佳)" -ForegroundColor Green
    Write-Host "  2. Debug (包含调试信息)" -ForegroundColor Green
    Write-Host ""

    $selected = $false
    while (-not $selected) {
        $buildChoice = Read-Host "请选择构建类型 (1-2)"
        switch ($buildChoice) {
            "1" {
                $script:BuildType = "release"
                Update-CliPath
                Write-Host "✅ 已选择 Release 构建" -ForegroundColor Green
                $selected = $true
            }
            "2" {
                $script:BuildType = "debug"
                Update-CliPath
                Write-Host "✅ 已选择 Debug 构建" -ForegroundColor Green
                $selected = $true
            }
            default {
                Write-Host "❌ 无效选项，请选择 1 或 2" -ForegroundColor Red
            }
        }
    }
    Write-Host ""
}

function Check-GlobalCli {
    Write-Host "🔍 检查全局CLI工具..." -ForegroundColor Yellow

    $sanshuFound = $false
    $dengFound = $false

    if (Test-Command $script:SanshuExeName) {
        $path = Resolve-CommandPath $script:SanshuExeName
        Write-Host "✅ 找到全局 三术 CLI: $path" -ForegroundColor Green
        $sanshuFound = $true
    }
    else {
        Write-Host "❌ 未找到全局 三术 CLI" -ForegroundColor Red
    }

    if (Test-Command $script:DengExeName) {
        $path = Resolve-CommandPath $script:DengExeName
        Write-Host "✅ 找到全局 等一下 CLI: $path" -ForegroundColor Green
        $dengFound = $true
    }
    else {
        Write-Host "❌ 未找到全局 等一下 CLI" -ForegroundColor Red
    }

    if (-not $sanshuFound -or -not $dengFound) {
        Write-Host "💡 全局CLI工具未完全安装，安装方法:" -ForegroundColor Yellow
        Write-Host "   cargo install --path . --bins" -ForegroundColor Cyan
        Write-Host "   或者选择使用本地编译版本" -ForegroundColor Yellow
        Write-Host ""

        Write-Host "🔄 是否切换到本地编译版本？ (y/n)" -ForegroundColor Cyan
        $switchChoice = Read-Host "请选择"
        if ($switchChoice -match '^[Yy]$') {
            $script:CliType = "local"
            Select-BuildType
            return
        }
        else {
            Write-Host "❌ 无法继续，请先安装全局CLI工具" -ForegroundColor Red
            exit 1
        }
    }

    $script:CliPath = ""  # 全局CLI不需要路径前缀
    Write-Host "✅ 全局CLI工具检查完成" -ForegroundColor Green
    Write-Host ""
}

function Select-CliType {
    Write-Host "🔧 选择CLI类型:" -ForegroundColor Yellow
    Write-Host "  1. 本地编译版本 (从项目target目录)" -ForegroundColor Green
    Write-Host "  2. 全局安装版本 (系统PATH中)" -ForegroundColor Green
    Write-Host ""

    $selected = $false
    while (-not $selected) {
        $cliChoice = Read-Host "请选择CLI类型 (1-2)"
        switch ($cliChoice) {
            "1" {
                $script:CliType = "local"
                Write-Host "✅ 已选择本地编译版本" -ForegroundColor Green
                Select-BuildType
                $selected = $true
            }
            "2" {
                $script:CliType = "global"
                Write-Host "✅ 已选择全局安装版本" -ForegroundColor Green
                Check-GlobalCli
                $selected = $true
            }
            default {
                Write-Host "❌ 无效选项，请选择 1 或 2" -ForegroundColor Red
            }
        }
    }
    Write-Host ""
}

function Compile-Project {
    if ($script:CliType -eq "global") {
        Write-Host "⚠️  使用全局CLI，跳过编译步骤" -ForegroundColor Yellow
        return
    }

    Write-Host "🔨 开始编译项目..." -ForegroundColor Yellow

    $cargoToml = Join-Path -Path $script:ProjectRoot -ChildPath "Cargo.toml"
    if (-not (Test-Path $cargoToml)) {
        Write-Host "❌ 未找到 Cargo.toml 文件" -ForegroundColor Red
        Write-Host "💡 请确保在Rust项目根目录中运行此脚本" -ForegroundColor Yellow
        exit 1
    }

    if (-not (Test-Command "cargo")) {
        Write-Host "❌ 未找到 cargo 命令" -ForegroundColor Red
        Write-Host "💡 请先安装 Rust: https://rustup.rs/" -ForegroundColor Yellow
        exit 1
    }

    if ($script:BuildType -eq "release") {
        Write-Host "📦 编译 Release 版本..." -ForegroundColor Cyan
        & cargo build --release
        if ($LASTEXITCODE -eq 0) {
            Write-Host "✅ Release 编译完成" -ForegroundColor Green
        }
        else {
            Write-Host "❌ Release 编译失败" -ForegroundColor Red
            exit 1
        }
    }
    else {
        Write-Host "📦 编译 Debug 版本..." -ForegroundColor Cyan
        & cargo build
        if ($LASTEXITCODE -eq 0) {
            Write-Host "✅ Debug 编译完成" -ForegroundColor Green
        }
        else {
            Write-Host "❌ Debug 编译失败" -ForegroundColor Red
            exit 1
        }
    }
    Write-Host ""
}

function Check-CliTools {
    if ($script:CliType -eq "global") {
        Write-Host "📋 检查全局CLI工具..." -ForegroundColor Yellow
        Check-GlobalCli
        return
    }

    Write-Host "📋 检查本地CLI工具 ($($script:BuildType))..." -ForegroundColor Yellow

    $sanshuPath = Join-Path -Path $script:CliPath -ChildPath $script:SanshuExeName
    if (-not (Test-Path $sanshuPath)) {
        Write-Host "❌ 未找到 三术 CLI工具" -ForegroundColor Red
        if ($script:BuildType -eq "release") {
            Write-Host "💡 请先编译项目: cargo build --release" -ForegroundColor Yellow
        }
        else {
            Write-Host "💡 请先编译项目: cargo build" -ForegroundColor Yellow
        }

        Write-Host "🔨 是否现在编译项目？ (y/n)" -ForegroundColor Cyan
        $compileChoice = Read-Host "请选择"
        if ($compileChoice -match '^[Yy]$') {
            Compile-Project
        }
        else {
            exit 1
        }
    }

    $dengPath = Join-Path -Path $script:CliPath -ChildPath $script:DengExeName
    if (-not (Test-Path $dengPath)) {
        Write-Host "❌ 未找到 等一下 CLI工具" -ForegroundColor Red
        if ($script:BuildType -eq "release") {
            Write-Host "💡 请先编译项目: cargo build --release" -ForegroundColor Yellow
        }
        else {
            Write-Host "💡 请先编译项目: cargo build" -ForegroundColor Yellow
        }

        Write-Host "🔨 是否现在编译项目？ (y/n)" -ForegroundColor Cyan
        $compileChoice = Read-Host "请选择"
        if ($compileChoice -match '^[Yy]$') {
            Compile-Project
        }
        else {
            exit 1
        }
    }

    Write-Host "✅ 本地CLI工具检查完成 ($($script:BuildType))" -ForegroundColor Green
    Write-Host "   构建类型: $($script:BuildType)"
    Write-Host "   三术: $sanshuPath"
    Write-Host "   等一下: $dengPath"
}

function Check-TestFiles {
    Write-Host "📋 检查测试文件..." -ForegroundColor Yellow

    if (-not (Test-Path $script:SimplePopupFile)) {
        Write-Host "❌ 未找到 test_simple_popup.json" -ForegroundColor Red
        exit 1
    }

    if (-not (Test-Path $script:MarkdownPopupFile)) {
        Write-Host "❌ 未找到 test_markdown_popup.json" -ForegroundColor Red
        exit 1
    }

    Write-Host "✅ 测试文件检查完成" -ForegroundColor Green
}

function Show-TestOptions {
    Write-Host "🎨 可用的测试选项:" -ForegroundColor Cyan
    if ($script:CliType -eq "global") {
        Write-Host "当前CLI类型: 全局安装版本" -ForegroundColor Yellow
    }
    else {
        Write-Host "当前CLI类型: 本地编译版本 ($($script:BuildType))" -ForegroundColor Yellow
    }
    Write-Host ""
    Write-Host "  1. 测试简单弹窗 (test_simple_popup.json)" -ForegroundColor Green
    Write-Host "  2. 测试Markdown弹窗 (test_markdown_popup.json)" -ForegroundColor Green
    Write-Host "  3. 测试自定义弹窗" -ForegroundColor Green
    Write-Host "  4. 启动前端测试环境" -ForegroundColor Green
    Write-Host "  5. 查看CLI工具帮助" -ForegroundColor Green
    Write-Host "  6. 切换CLI类型" -ForegroundColor Green
    Write-Host "  7. 安装/重新编译" -ForegroundColor Green
    Write-Host "  q. 退出" -ForegroundColor Green
    Write-Host ""
}

function Show-JsonContent {
    param([string]$FilePath)
    if (Test-Command "jq") {
        & jq "." $FilePath
    }
    else {
        Write-Host "JSON内容:"
        Get-Content -Path $FilePath
    }
}

function Get-CliCommand {
    param([string]$CliName)
    if ($script:CliType -eq "global") {
        return $CliName
    }
    return (Join-Path -Path $script:CliPath -ChildPath $CliName)
}

function Test-SimplePopup {
    Write-Host "🚀 启动简单弹窗测试..." -ForegroundColor Yellow
    Write-Host "使用文件: test_simple_popup.json" -ForegroundColor Cyan

    Write-Host "📄 文件内容:" -ForegroundColor Yellow
    Show-JsonContent -FilePath $script:SimplePopupFile
    Write-Host ""

    $cliCmd = Get-CliCommand $script:DengExeName
    Write-Host "🎯 启动弹窗..." -ForegroundColor Green
    Write-Host "执行命令: $cliCmd --mcp-request test_simple_popup.json" -ForegroundColor Cyan

    & $cliCmd --mcp-request $script:SimplePopupFile
    if ($LASTEXITCODE -eq 0) {
        Write-Host "✅ 弹窗测试完成" -ForegroundColor Green
    }
    else {
        Write-Host "❌ 弹窗测试失败" -ForegroundColor Red
        Write-Host "💡 请检查CLI工具是否正常工作" -ForegroundColor Yellow
    }
}

function Test-MarkdownPopup {
    Write-Host "🚀 启动Markdown弹窗测试..." -ForegroundColor Yellow
    Write-Host "使用文件: test_markdown_popup.json" -ForegroundColor Cyan

    Write-Host "📄 文件内容:" -ForegroundColor Yellow
    Show-JsonContent -FilePath $script:MarkdownPopupFile
    Write-Host ""

    $cliCmd = Get-CliCommand $script:DengExeName
    Write-Host "🎯 启动弹窗..." -ForegroundColor Green
    Write-Host "执行命令: $cliCmd --mcp-request test_markdown_popup.json" -ForegroundColor Cyan

    & $cliCmd --mcp-request $script:MarkdownPopupFile
    if ($LASTEXITCODE -eq 0) {
        Write-Host "✅ Markdown弹窗测试完成" -ForegroundColor Green
    }
    else {
        Write-Host "❌ Markdown弹窗测试失败" -ForegroundColor Red
        Write-Host "💡 请检查CLI工具是否正常工作" -ForegroundColor Yellow
    }
}

function Test-CustomPopup {
    Write-Host "🚀 创建自定义弹窗测试..." -ForegroundColor Yellow

    $tempFile = Join-Path -Path ([System.IO.Path]::GetTempPath()) -ChildPath "custom_popup_test.json"
    $jsonContent = @'
{
  "id": "custom-test-001",
  "message": "# 🎨 自定义弹窗测试\n\n这是一个自定义的弹窗测试，用于验证弹窗功能的完整性。\n\n## ✨ 测试功能\n- 头部固定显示\n- 工具栏固定显示\n- 图片组件渲染\n- 输入框组件\n- 禁止选中非内容区域\n- Markdown紧凑渲染\n\n## 🔧 操作说明\n1. 测试主题切换按钮\n2. 测试打开主界面按钮\n3. 测试预定义选项选择\n4. 测试文本输入功能\n5. 测试图片粘贴功能\n\n```typescript\n// 示例代码\ninterface PopupTest {\n  header: 'fixed'\n  toolbar: 'fixed'\n  content: 'scrollable'\n  images: 'component-rendered'\n  input: 'component-based'\n}\n```\n\n> **注意**: 请测试所有交互功能以确保弹窗工作正常。",
  "predefined_options": [
    "🎨 测试主题切换",
    "🏠 测试主界面按钮", 
    "📝 测试文本输入",
    "🖼️ 测试图片功能",
    "⚡ 测试快捷键",
    "✅ 测试完成",
    "❌ 发现问题"
  ],
  "is_markdown": true
}
'@

    Write-Utf8NoBom -Path $tempFile -Content $jsonContent

    Write-Host "📄 自定义测试内容:" -ForegroundColor Yellow
    Show-JsonContent -FilePath $tempFile
    Write-Host ""

    $cliCmd = Get-CliCommand $script:DengExeName
    Write-Host "🎯 启动自定义弹窗..." -ForegroundColor Green
    Write-Host "执行命令: $cliCmd --mcp-request $tempFile" -ForegroundColor Cyan

    & $cliCmd --mcp-request $tempFile
    if ($LASTEXITCODE -eq 0) {
        Write-Host "✅ 自定义弹窗测试完成" -ForegroundColor Green
    }
    else {
        Write-Host "❌ 自定义弹窗测试失败" -ForegroundColor Red
        Write-Host "💡 请检查CLI工具是否正常工作" -ForegroundColor Yellow
    }

    Remove-Item -Path $tempFile -Force -ErrorAction SilentlyContinue
}

function Start-FrontendTest {
    Write-Host "🚀 启动前端测试环境..." -ForegroundColor Yellow
    Write-Host "测试环境将在 http://localhost:5174 启动" -ForegroundColor Cyan
    Write-Host "💡 按 Ctrl+C 停止测试环境" -ForegroundColor Yellow
    Write-Host ""

    if (-not (Test-Command "pnpm")) {
        Write-Host "❌ 未找到 pnpm 命令" -ForegroundColor Red
        Write-Host "💡 请先安装 pnpm: npm install -g pnpm" -ForegroundColor Yellow
        return
    }

    $packageJson = Join-Path -Path $script:ProjectRoot -ChildPath "package.json"
    if (-not (Test-Path $packageJson)) {
        Write-Host "❌ 未找到 package.json 文件" -ForegroundColor Red
        return
    }

    Push-Location -Path $script:ProjectRoot
    try {
        & pnpm "test:ui"
    }
    finally {
        Pop-Location
    }
}

function Show-CliHelp {
    Write-Host "📖 CLI工具帮助信息:" -ForegroundColor Yellow
    Write-Host ""

    $sanshuCmd = Get-CliCommand $script:SanshuExeName
    $dengCmd = Get-CliCommand $script:DengExeName

    Write-Host "三术 CLI:" -ForegroundColor Cyan
    Write-Host "命令: $sanshuCmd" -ForegroundColor Cyan
    & $sanshuCmd --help 2>$null
    if ($LASTEXITCODE -eq 0) {
        Write-Host "✅ 帮助信息显示完成" -ForegroundColor Green
    }
    else {
        Write-Host "⚠️  三术 CLI 无帮助信息或不支持 --help 参数" -ForegroundColor Yellow
        Write-Host "尝试直接运行: $sanshuCmd" -ForegroundColor Cyan
    }
    Write-Host ""

    Write-Host "等一下 CLI:" -ForegroundColor Cyan
    Write-Host "命令: $dengCmd" -ForegroundColor Cyan
    & $dengCmd --help 2>$null
    if ($LASTEXITCODE -eq 0) {
        Write-Host "✅ 帮助信息显示完成" -ForegroundColor Green
    }
    else {
        Write-Host "⚠️  等一下 CLI 无帮助信息或不支持 --help 参数" -ForegroundColor Yellow
        Write-Host "尝试直接运行: $dengCmd" -ForegroundColor Cyan
        Write-Host "MCP请求参数: $dengCmd --mcp-request <json_file>" -ForegroundColor Cyan
    }
}

function Switch-CliType {
    Write-Host "🔄 切换CLI类型" -ForegroundColor Yellow
    if ($script:CliType -eq "global") {
        Write-Host "当前CLI类型: 全局安装版本"
    }
    else {
        Write-Host "当前CLI类型: 本地编译版本 ($($script:BuildType))"
    }
    Write-Host ""

    if ($script:CliType -eq "global") {
        $script:CliType = "local"
        Write-Host "✅ 已切换到本地编译版本" -ForegroundColor Green
        Select-BuildType
    }
    else {
        $script:CliType = "global"
        Write-Host "✅ 已切换到全局安装版本" -ForegroundColor Green
        Check-GlobalCli
    }
    Write-Host ""
}

function Install-OrCompile {
    if ($script:CliType -eq "global") {
        Write-Host "🔨 安装全局CLI工具..." -ForegroundColor Yellow
        Write-Host "执行命令: cargo install --path . --bins" -ForegroundColor Cyan

        if (-not (Test-Command "cargo")) {
            Write-Host "❌ 未找到 cargo 命令" -ForegroundColor Red
            Write-Host "💡 请先安装 Rust: https://rustup.rs/" -ForegroundColor Yellow
            return
        }

        & cargo install --path . --bins
        if ($LASTEXITCODE -eq 0) {
            Write-Host "✅ 全局CLI工具安装完成" -ForegroundColor Green
            Check-GlobalCli
        }
        else {
            Write-Host "❌ 全局CLI工具安装失败" -ForegroundColor Red
        }
    }
    else {
        Write-Host "🔨 重新编译本地项目 ($($script:BuildType))..." -ForegroundColor Yellow
        Compile-Project
        Check-CliTools
    }
}

function Main {
    Write-Host "🎯 三术弹窗测试脚本" -ForegroundColor Cyan
    Write-Host "================================" -ForegroundColor Cyan

    Select-CliType
    Check-CliTools
    Check-TestFiles

    Write-Host ""

    while ($true) {
        Show-TestOptions
        $choice = Read-Host "请选择测试选项 (1-7, q)"
        Write-Host ""

        switch ($choice) {
            "1" { Test-SimplePopup }
            "2" { Test-MarkdownPopup }
            "3" { Test-CustomPopup }
            "4" { Start-FrontendTest }
            "5" { Show-CliHelp }
            "6" { Switch-CliType }
            "7" { Install-OrCompile }
            "q" { Write-Host "👋 测试结束，再见！" -ForegroundColor Green; exit 0 }
            "Q" { Write-Host "👋 测试结束，再见！" -ForegroundColor Green; exit 0 }
            default { Write-Host "❌ 无效选项，请重新选择" -ForegroundColor Red }
        }

        Write-Host ""
        Write-Host "按回车键继续..." -ForegroundColor Yellow
        [void](Read-Host)
        Write-Host ""
    }
}

# 检查依赖工具
Write-Host "🔍 检查依赖工具..." -ForegroundColor Cyan
if (-not (Test-Command "jq")) {
    Write-Host "⚠️  建议安装 jq 以获得更好的JSON显示效果" -ForegroundColor Yellow
    Write-Host "   Windows: winget install jqlang.jq 或 choco install jq" -ForegroundColor Yellow
    Write-Host ""
}
else {
    Write-Host "✅ jq 已安装" -ForegroundColor Green
}

if (-not (Test-Command "pnpm")) {
    Write-Host "⚠️  建议安装 pnpm 以使用前端测试环境" -ForegroundColor Yellow
    Write-Host "   安装命令: npm install -g pnpm" -ForegroundColor Yellow
    Write-Host ""
}
else {
    Write-Host "✅ pnpm 已安装" -ForegroundColor Green
}
Write-Host ""

# 运行主函数
Main
