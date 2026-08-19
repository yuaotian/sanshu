param(
    [switch]$SkipFrontendBuild
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$OutputEncoding = [Console]::OutputEncoding

$ProjectRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
Set-Location -LiteralPath $ProjectRoot

function Invoke-CheckedCommand {
    param(
        [Parameter(Mandatory = $true)][string]$Title,
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    Write-Host ""
    Write-Host "==> $Title" -ForegroundColor Cyan
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Title 失败，退出码：$LASTEXITCODE"
    }
}

function Assert-SourceContract {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Pattern,
        [Parameter(Mandatory = $true)][string]$Description
    )

    $content = Get-Content -LiteralPath $Path -Raw
    if ($content -notmatch $Pattern) {
        throw "源码契约缺失：$Description"
    }
}

Invoke-CheckedCommand -Title "运行 ACE 范围保护单元测试" -Command "cargo" -Arguments @(
    "test", "scope_guard", "--lib"
)
Invoke-CheckedCommand -Title "检查两个 Rust 二进制目标" -Command "cargo" -Arguments @(
    "check", "--bins", "--features", "custom-protocol"
)
Invoke-CheckedCommand -Title "检查范围风险前端文件" -Command "pnpm" -Arguments @(
    "exec", "eslint",
    "src/frontend/components/AppContent.vue",
    "src/frontend/components/common/ProjectScopeRiskDialog.vue",
    "src/frontend/types/tauri.d.ts"
)

if (-not $SkipFrontendBuild) {
    Invoke-CheckedCommand -Title "编译前端生产包" -Command "pnpm" -Arguments @("build")
}

$mcpPath = Join-Path $ProjectRoot "src/rust/mcp/tools/acemcp/mcp.rs"
$guardPath = Join-Path $ProjectRoot "src/rust/mcp/tools/acemcp/scope_guard.rs"
$appContentPath = Join-Path $ProjectRoot "src/frontend/components/AppContent.vue"

# 中文说明：这些断言防止后续重构重新引入全量内容 clone 或绕过风险弹窗。
Assert-SourceContract -Path $mcpPath -Pattern 'blobs\s*\.into_iter\(\)' -Description "BlobItem 集合必须通过移动转换"
Assert-SourceContract -Path $mcpPath -Pattern 'blob_entries\.retain' -Description "未完成集合必须原地收缩"
Assert-SourceContract -Path $guardPath -Pattern 'MAX_CANDIDATE_FILES:\s*usize\s*=\s*50_000' -Description "候选文件预检阈值"
Assert-SourceContract -Path $guardPath -Pattern 'MAX_CANDIDATE_BYTES:\s*u64\s*=\s*1024\s*\*\s*1024\s*\*\s*1024' -Description "候选体积预检阈值"
Assert-SourceContract -Path $appContentPath -Pattern 'ProjectScopeRiskDialog' -Description "全局范围风险弹窗"

Write-Host ""
Write-Host "ACE 项目范围与内存保护验证通过。" -ForegroundColor Green
