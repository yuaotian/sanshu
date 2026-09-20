param(
    [switch]$SkipCheck,
    [switch]$SkipFrontend,
    [switch]$RunModelE2E,
    [switch]$RetrievalOnly,
    [switch]$ModelConfig
)

$ErrorActionPreference = 'Stop'
# 中文说明：检索回归保持本地范围，模型下载 E2E 仍由单独的显式完整验证入口启用。
if ($RetrievalOnly -and $RunModelE2E) {
    throw '-RetrievalOnly 与 -RunModelE2E 互斥，请单独执行模型下载 E2E'
}
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
    if ($RetrievalOnly) {
        Write-Host '检索验证范围：UIUX 与 sou 单元测试、UIUX MCP 契约；跳过网络策略、更新器和模型下载。前端仅在 ModelConfig 模式验证。'
    }
    if ($ModelConfig) {
        Write-Host '追加验证：共享模型目录前置条件、只读状态和配置组件；仅临时目录与 mock，不操作真实模型。'
    }
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
        'src/rust/mcp/tools/uiux/model_manager.rs'
        'src/rust/mcp/tools/uiux/mod.rs'
        'src/rust/mcp/tools/uiux/semantic_search.rs'
        'src/rust/mcp/tools/uiux/structured_search.rs'
        'src/rust/mcp/tools/uiux/types.rs'
        'src/rust/mcp/tools/sou/mod.rs'
        'tests/uiux_mcp.rs'
    )
    # 中文说明：默认仍保留完整检查；定向检索入口仅检查本次检索链涉及的 Rust 文件。
    if (-not $RetrievalOnly) {
        $rustFiles += @(
            'src/rust/network/geo.rs'
            'src/rust/network/github_strategy.rs'
            'src/rust/network/mod.rs'
            'src/rust/ui/updater.rs'
        )
    }
    if ($ModelConfig) {
        $rustFiles += @(
            'src/rust/mcp/embedding/mod.rs'
            'src/rust/mcp/tools/acemcp/commands.rs'
        )
    }
    rustfmt --edition 2021 --check --config skip_children=true $rustFiles
    Assert-NativeSuccess 'Rust 格式检查' $LASTEXITCODE

    cargo test --lib uiux:: -j 1
    Assert-NativeSuccess 'UIUX 模块单元测试' $LASTEXITCODE

    if ($RunModelE2E) {
        cargo test --lib model_download_index_and_query_e2e -j 1 -- --ignored --nocapture
        Assert-NativeSuccess 'UIUX BGE 模型下载、索引与查询 E2E' $LASTEXITCODE
    }

    if (-not $RetrievalOnly) {
        cargo test --lib network::github_strategy::tests -j 1
        Assert-NativeSuccess 'GitHub 路由策略单元测试' $LASTEXITCODE

        cargo test --lib ui::updater::tests -j 1
        Assert-NativeSuccess '更新摘要校验单元测试' $LASTEXITCODE
    }

    cargo test --lib mcp::tools::sou::tests -j 1
    Assert-NativeSuccess 'sou 结构化片段单元测试' $LASTEXITCODE

    if ($ModelConfig) {
        cargo test --lib mcp::embedding::tests -j 1
        Assert-NativeSuccess '共享模型目录与运行时单元测试' $LASTEXITCODE
    }

    if ($env:OS -eq 'Windows_NT') {
        # 中文说明：集成测试编译也固定单任务，与检索单元测试保持同一资源预算。
        cargo test --test uiux_mcp --no-run -j 1
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
        cargo test --test uiux_mcp -j 1
        Assert-NativeSuccess 'UIUX MCP 集成测试' $LASTEXITCODE
    }

    if (-not $SkipCheck) {
        # 当前主机默认并行检查偶发 zlib-rs CTFE 尺寸异常；scoped 验证固定单任务以获得可重复结果。
        cargo check --lib -j 1
        Assert-NativeSuccess 'Rust library 编译检查' $LASTEXITCODE
    }

    if ($ModelConfig) {
        pnpm exec vitest run src/frontend/components/tools/__tests__/UiuxConfig.spec.ts src/frontend/components/tools/__tests__/SouConfig.spec.ts src/frontend/components/tabs/__tests__/McpToolsTab.spec.ts
        Assert-NativeSuccess '模型配置组件回归测试' $LASTEXITCODE
    }

    if (-not $SkipFrontend -and ((-not $RetrievalOnly) -or $ModelConfig)) {
        pnpm exec eslint src/frontend/components/tools/SouConfig.vue src/frontend/components/tools/UiuxConfig.vue src/frontend/components/tabs/McpToolsTab.vue
        Assert-NativeSuccess 'UIUX 配置界面 ESLint 检查' $LASTEXITCODE

        pnpm build
        Assert-NativeSuccess '前端生产构建' $LASTEXITCODE
    }
}
finally {
    Pop-Location
}
