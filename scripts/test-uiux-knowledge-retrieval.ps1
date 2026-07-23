param(
    [switch]$SkipCheck,
    [switch]$SkipFrontend
)

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
    $rustFiles = @(
        'src/rust/config/settings.rs'
        'src/rust/mcp/tools/acemcp/commands.rs'
        'src/rust/mcp/tools/sou/mod.rs'
        'src/rust/mcp/tools/uiux/knowledge_base.rs'
        'src/rust/mcp/tools/uiux/markdown_search.rs'
        'src/rust/mcp/tools/uiux/mcp.rs'
        'src/rust/mcp/tools/uiux/mod.rs'
        'src/rust/mcp/tools/uiux/types.rs'
        'tests/uiux_mcp.rs'
    )
    rustfmt --edition 2021 --check --config skip_children=true $rustFiles
    Assert-NativeSuccess 'Rust 格式检查' $LASTEXITCODE

    cargo test --lib uiux::
    Assert-NativeSuccess 'UIUX 模块单元测试' $LASTEXITCODE

    cargo test --lib mcp::tools::sou::tests
    Assert-NativeSuccess 'sou 结构化片段单元测试' $LASTEXITCODE

    cargo test --test uiux_mcp
    Assert-NativeSuccess 'UIUX MCP 集成测试' $LASTEXITCODE

    if (-not $SkipCheck) {
        cargo check --lib
        Assert-NativeSuccess 'Rust library 编译检查' $LASTEXITCODE
    }

    if (-not $SkipFrontend) {
        pnpm exec eslint src/frontend/components/tools/SouConfig.vue
        Assert-NativeSuccess 'SouConfig ESLint 检查' $LASTEXITCODE

        pnpm build
        Assert-NativeSuccess '前端生产构建' $LASTEXITCODE
    }
}
finally {
    Pop-Location
}
