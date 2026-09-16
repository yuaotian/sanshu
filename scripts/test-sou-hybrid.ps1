param(
    [switch]$SkipFrontend,
    [switch]$SkipBuild,
    [switch]$RunRealQueries,
    [string]$OmniProject = 'E:\ProjectCode\C++Code\omni-mouse-plus'
)

$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
Push-Location $projectRoot

function Assert-Success([string]$Step) {
    if ($LASTEXITCODE -ne 0) { throw "$Step 失败，退出码 $LASTEXITCODE" }
}

try {
    # 中文说明：回归门禁覆盖真实行为，独立记录热索引性能，避免把远端耗时混入 Local 指标。
    cargo test --lib mcp::tools::sou
    Assert-Success 'Sou 行为回归'
    cargo test --lib warm_fts5_query_p95_is_within_target_for_thousands_of_files -- --ignored --nocapture
    Assert-Success 'FTS5 热查询性能'

    if (-not $SkipFrontend) {
        pnpm exec eslint src/frontend/components/tools/SouConfig.vue
        Assert-Success 'SouConfig 规范检查'
        pnpm build
        Assert-Success '前端构建'
    }
    if (-not $SkipBuild) {
        cargo build --bins --features custom-protocol
        Assert-Success 'MCP 与桌面内嵌前端构建'
    }
    if ($RunRealQueries) {
        $cases = @(
            @{ name = 'explicit-document'; root = $OmniProject; query = 'find-mouse-theme-2.5d-plan-2026-09-16.md 后续主题 万剑归宗 三体水滴 星河飞船 流浪地球 实施计划与验收'; expected = 'docs/find-mouse-theme-2.5d-plan-2026-09-16.md' }
            @{ name = 'implementation'; root = $projectRoot; query = 'ProjectIndex ensure_watcher schedule_sync sync_index 文件变更过滤和索引同步生命周期'; expected = 'src/rust/mcp/tools/sou/local.rs' }
        )
        $suite = foreach ($case in $cases) {
            foreach ($backend in @('local', 'hybrid')) {
                $entry = $case.Clone()
                $entry.backend = $backend
                $entry
            }
        }
        $previousQueries = $env:SANSHU_SOU_HYBRID_QUERIES
        try {
            $env:SANSHU_SOU_HYBRID_QUERIES = ConvertTo-Json -InputObject @($suite) -Compress
            # 中文说明：直接运行新构建的 MCP 搜索入口，避开桌面会话中尚未重启的旧进程。
            $output = & cargo test --lib real_queries_from_env -- --ignored --nocapture 2>&1
            $code = $LASTEXITCODE
            $output | ForEach-Object { Write-Output $_ }
            $report = $output | ForEach-Object { "$PSItem" } | Where-Object { $_.StartsWith('SOU_HYBRID_REPORT=') } | Select-Object -Last 1
            if ($report) {
                $report.Substring('SOU_HYBRID_REPORT='.Length) | Set-Content -LiteralPath (Join-Path $projectRoot 'target/sou-hybrid-verification.json') -Encoding utf8
            }
            if ($code -ne 0) { throw "真实查询验收失败，退出码 $code" }
        }
        finally { $env:SANSHU_SOU_HYBRID_QUERIES = $previousQueries }
    }
}
finally { Pop-Location }
