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
    $assetRoot = Join-Path $projectRoot 'src/rust/assets/resources/ui-ux-pro-max-v2.15.0'
    $upstream = Get-Content (Join-Path $assetRoot 'UPSTREAM.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    $catalog = Get-Content (Join-Path $assetRoot 'data/catalog-summary.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($upstream.tag -ne 'v2.15.0' -or $catalog.verifiedAt -ne '2026-08-13') {
        throw 'UI/UX Pro Max 上游版本或目录核验日期不符合固定基线'
    }
    if ((Get-ChildItem (Join-Path $assetRoot 'data/stacks') -Filter '*.csv').Count -ne 22) {
        throw 'UI/UX Pro Max 技术栈数据应为 22 份'
    }

    $rustFiles = @(
        'src/rust/mcp/tools/uiux/knowledge_base.rs'
        'src/rust/mcp/tools/uiux/lexicon.rs'
        'src/rust/mcp/tools/uiux/mcp.rs'
        'src/rust/mcp/tools/uiux/mod.rs'
        'src/rust/mcp/tools/uiux/structured_search.rs'
        'src/rust/mcp/tools/uiux/types.rs'
        'tests/uiux_mcp.rs'
    )
    rustfmt --edition 2021 --check --config skip_children=true $rustFiles
    Assert-NativeSuccess 'Rust 格式检查' $LASTEXITCODE

    cargo test --lib uiux::
    Assert-NativeSuccess 'UIUX 模块单元测试' $LASTEXITCODE

    cargo test --lib mcp::tools::sou::tests
    Assert-NativeSuccess 'sou 结构化片段单元测试' $LASTEXITCODE

    if ($env:OS -eq 'Windows_NT') {
        cargo test --test uiux_mcp --no-run
        Assert-NativeSuccess 'UIUX MCP 集成测试编译' $LASTEXITCODE

        $testBinary = Get-ChildItem (Join-Path $projectRoot 'target/debug/deps/uiux_mcp-*.exe') |
            Sort-Object LastWriteTime -Descending |
            Select-Object -First 1 -ExpandProperty FullName
        $mtCommand = Get-Command mt.exe -ErrorAction SilentlyContinue
        $mtPath = if ($mtCommand) {
            $mtCommand.Source
        }
        else {
            Get-ChildItem "${env:ProgramFiles(x86)}/Windows Kits/10/bin" -Filter mt.exe -Recurse -ErrorAction SilentlyContinue |
                Where-Object { $_.FullName -match '[\\/]x64[\\/]mt\.exe$' } |
                Sort-Object FullName -Descending |
                Select-Object -First 1 -ExpandProperty FullName
        }
        if (-not $mtPath) {
            throw '未找到 Windows Manifest Tool (mt.exe)，不能激活 Common-Controls v6 测试依赖'
        }

        & $mtPath -nologo -manifest 'tests/windows-common-controls.manifest' "-outputresource:$testBinary;#1"
        Assert-NativeSuccess 'UIUX MCP 测试清单嵌入' $LASTEXITCODE
        & $testBinary
        Assert-NativeSuccess 'UIUX MCP 集成测试' $LASTEXITCODE
    }
    else {
        cargo test --test uiux_mcp
        Assert-NativeSuccess 'UIUX MCP 集成测试' $LASTEXITCODE
    }

    if (-not $SkipCheck) {
        # 当前主机默认并行检查偶发 zlib-rs CTFE 尺寸异常；scoped 验证固定单任务以获得可重复结果。
        cargo check --lib -j 1
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
