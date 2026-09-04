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
    Invoke-CheckedCommand -Title '检查 sou Rust 格式' -Command 'rustfmt' -Arguments @(
        '--edition', '2021', '--check', '--config', 'skip_children=true',
        'src/rust/mcp/tools/sou/mod.rs'
    )
    Invoke-CheckedCommand -Title '检查 ACE/sou 变更空白字符' -Command 'git' -Arguments @(
        'diff', '--check', '--',
        'src/rust/mcp/tools/acemcp/mcp.rs',
        'src/rust/mcp/tools/sou/mod.rs',
        'scripts/test-ace-sou-resilience.ps1'
    )
    Invoke-CheckedCommand -Title '运行 sou 响应解析单元测试' -Command 'cargo' -Arguments @(
        'test', '--lib', 'mcp::tools::sou::tests'
    )
    Invoke-CheckedCommand -Title '运行 ACE 网络重试分类单元测试' -Command 'cargo' -Arguments @(
        'test', '--lib', 'mcp::tools::acemcp::mcp::retry_tests'
    )
    Invoke-CheckedCommand -Title '执行 Rust library 编译检查' -Command 'cargo' -Arguments @(
        'check', '--lib', '-j', '1'
    )

    Write-Host ""
    Write-Host 'ACE/sou 定向验证通过。' -ForegroundColor Green
}
finally {
    Pop-Location
}
