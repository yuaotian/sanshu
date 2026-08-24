$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot

function Assert-NativeSuccess {
    param(
        [string]$Step,
        [int]$ExitCode
    )

    if ($ExitCode -ne 0) {
        throw "$Step 失败，退出码: $ExitCode"
    }
}

try {
    rustfmt --edition 2021 --check 'src/rust/mcp/server.rs'
    Assert-NativeSuccess 'MCP 服务端格式检查' $LASTEXITCODE

    cargo test --lib mcp::server::tests
    Assert-NativeSuccess 'Antigravity MCP 握手兼容测试' $LASTEXITCODE

    cargo check --bin '三术'
    Assert-NativeSuccess '三术 MCP 二进制静态检查' $LASTEXITCODE
}
finally {
    Pop-Location
}
