# 测试图标弹窗模式脚本 (自动构建版)
# 用法: ./test_icon_popup.ps1 [[-Mode] <debug|release>] [-Clean] [-SkipFrontend]

param (
    [string]$Mode = "debug",
    [switch]$Clean,        # 清理依赖和构建产物
    [switch]$SkipFrontend  # 跳过前端构建 (仅编译 Rust)
)

$ErrorActionPreference = "Stop"

Write-Host "=== 启动测试流程 ($Mode) ===" -ForegroundColor Cyan

# 0. 检查 pnpm
if (-not (Get-Command "pnpm" -ErrorAction SilentlyContinue)) {
    Write-Host "错误: 未找到 pnpm。请先安装 pnpm (npm install -g pnpm)" -ForegroundColor Red
    exit 1
}

# 1. 清理 (如果有要求)
if ($Clean) {
    Write-Host "`n[清理阶段]" -ForegroundColor Yellow
    if (Test-Path "target") {
        Write-Host "清理 Rust 构建目录 (cargo clean)..."
        cargo clean
    }
    if (Test-Path "dist") {
        Write-Host "清理前端构建目录 (dist)..."
        Remove-Item -Path "dist" -Recurse -Force
    }
    # 通常不需要删除 node_modules 除非出大问题，这里为了效率仅清理构建产物
    # if (Test-Path "node_modules") { Remove-Item "node_modules" -Recurse -Force }
}

# 2. 前端构建
if (-not $SkipFrontend) {
    Write-Host "`n[前端构建阶段]" -ForegroundColor Cyan
    
    # 安装依赖 (如果需要)
    if (-not (Test-Path "node_modules")) {
        Write-Host "安装前端依赖 (pnpm install)..."
        pnpm install
    }

    # 编译前端
    Write-Host "编译前端资源 (pnpm build)..."
    try {
        pnpm build
    } catch {
        Write-Host "前端构建失败！" -ForegroundColor Red
        exit 1
    }
} else {
    Write-Host "`n[前端构建阶段] 已跳过" -ForegroundColor DarkGray
}

# 3. 后端编译
Write-Host "`n[后端编译阶段]" -ForegroundColor Cyan
$BuildArgs = @("build")
if ($Mode -eq "release") { $BuildArgs += "--release" }

Write-Host "正在编译 Rust 后端 (cargo $($BuildArgs -join ' '))..."
& cargo $BuildArgs

if ($LASTEXITCODE -ne 0) {
    Write-Host "Rust 编译失败！" -ForegroundColor Red
    exit 1
}

# 4. 运行测试
Write-Host "`n[测试运行阶段]" -ForegroundColor Cyan

# 确定可执行文件路径
$ExePath = "target/$Mode/等一下.exe"

if (-not (Test-Path $ExePath)) {
    Write-Host "错误: 找不到可执行文件 $ExePath" -ForegroundColor Red
    exit 1
}

# 测试用例 1: 基础搜索
Write-Host "`n--> 测试 1: 基础搜索 'settings'" -ForegroundColor Yellow
Start-Process -FilePath $ExePath -ArgumentList "--icon-search", "settings" -Wait
Write-Host "测试 1 完成" -ForegroundColor Green

# 测试用例 2: 指定风格 (线性) 和 关键词
Write-Host "`n--> 测试 2: 搜索 'user'，风格 'line'" -ForegroundColor Yellow
$Env:SANSHU_LOG = "debug" # 开启日志以便观察输出
Start-Process -FilePath $ExePath -ArgumentList "--icon-search", "user", "--style", "line" -Wait
Write-Host "测试 2 完成" -ForegroundColor Green

# 测试用例 3: 指定保存路径
Write-Host "`n--> 测试 3: 搜索 'file'，指定保存路径 './test_output'" -ForegroundColor Yellow
# 创建输出目录
if (-not (Test-Path "./test_output")) {
    New-Item -ItemType Directory -Force -Path "./test_output" | Out-Null
}
Start-Process -FilePath $ExePath -ArgumentList "--icon-search", "file", "--save-path", "./test_output" -Wait
Write-Host "测试 3 完成。请检查 ./test_output 目录。" -ForegroundColor Green

Write-Host "`n=== 所有流程执行完毕 ===" -ForegroundColor Cyan
