[CmdletBinding()]
param(
    [switch]$SkipFrontendBuild,
    [switch]$SkipRustBuild
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$OutputEncoding = [Console]::OutputEncoding

$projectRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
Push-Location -LiteralPath $projectRoot

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

try {
    Invoke-CheckedCommand -Title '检查 Rust 格式' -Command 'cargo' -Arguments @(
        'fmt', '--all', '--', '--check'
    )
    Invoke-CheckedCommand -Title '检查工作区联合索引差异空白字符' -Command 'git' -Arguments @(
        'diff', 'HEAD', '--check', '--',
        'src/frontend/components/index/ProjectCard.vue',
        'src/frontend/components/settings/ProjectIndexManager.vue',
        'src/frontend/components/tools/SouConfig.vue',
        'src/frontend/types/tauri.d.ts',
        'src/rust/mcp/tools/acemcp/commands.rs',
        'src/rust/mcp/tools/acemcp/mcp.rs',
        'src/rust/mcp/tools/acemcp/types.rs',
        'src/rust/mcp/tools/acemcp/watcher.rs',
        'src/rust/mcp/tools/sou/local.rs',
        'src/rust/mcp/tools/sou/semantic.rs',
        'src/rust/mcp/tools/workspace.rs',
        'scripts/test-sou-workspace-federation.ps1'
    )
    Invoke-CheckedCommand -Title '运行工作区解析与监听根测试' -Command 'cargo' -Arguments @(
        'test', '--lib', 'mcp::tools::workspace'
    )
    Invoke-CheckedCommand -Title '运行 ACE 工作区与 watcher 测试' -Command 'cargo' -Arguments @(
        'test', '--lib', 'mcp::tools::acemcp::'
    )
    Invoke-CheckedCommand -Title '运行 SOU Local 与联合搜索测试' -Command 'cargo' -Arguments @(
        'test', '--lib', 'mcp::tools::sou::'
    )
    Invoke-CheckedCommand -Title '检查工作区索引前端文件' -Command 'pnpm' -Arguments @(
        'exec', 'eslint',
        'src/frontend/components/index/ProjectCard.vue',
        'src/frontend/components/settings/ProjectIndexManager.vue',
        'src/frontend/components/tools/SouConfig.vue',
        'src/frontend/types/tauri.d.ts',
        '--rule', 'vue/custom-event-name-casing: off',
        '--rule', 'perfectionist/sort-imports: off'
    )

    if (-not $SkipFrontendBuild) {
        Invoke-CheckedCommand -Title '构建前端' -Command 'pnpm' -Arguments @('build')
    }
    if (-not $SkipRustBuild) {
        Invoke-CheckedCommand -Title '构建 Rust 后端' -Command 'cargo' -Arguments @('build')
    }

    Write-Host ""
    Write-Host 'SOU 工作区联合索引验证通过。' -ForegroundColor Green
}
finally {
    Pop-Location
}
