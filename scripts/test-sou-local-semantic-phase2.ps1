param(
    [string]$JavaProject = 'E:\ProjectCode\CompanyCode\debt\debt-server',
    [string]$SecondaryProject = 'E:\ProjectCode\GoCode\go-codex-radar',
    [string]$ModelDir = '',
    [string]$RerankerDir = '',
    [ValidateSet('release', 'debug')]
    [string]$Profile = 'release',
    [ValidateRange(20000, 200000)]
    [int]$SyntheticChunks = 20000,
    [ValidateRange(1, 10000)]
    [int]$SyntheticMaxP95Ms = 120,
    [ValidateRange(500, 10000)]
    [int]$AccurateMaxP95Ms = 3000,
    [ValidateRange(3000, 30000)]
    [int]$AccurateObservationMs = 10000,
    [ValidateRange(512, 16384)]
    [int]$MaxPeakMiB = 4096,
    [ValidateRange(128, 4096)]
    [int]$MaxBalancedRssMiB = 500,
    [ValidateRange(10, 120)]
    [int]$ReleaseWaitSeconds = 30,
    [ValidateRange(5, 240)]
    [int]$TimeoutMinutes = 90,
    [string]$ReportPath = ''
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$OutputEncoding = [Console]::OutputEncoding
$projectRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$javaRoot = (Resolve-Path -LiteralPath $JavaProject).Path
$secondaryRoot = (Resolve-Path -LiteralPath $SecondaryProject).Path
if (-not $RerankerDir) {
    $RerankerDir = Join-Path $env:LOCALAPPDATA 'sanshu\models\bge-reranker-base'
}
$rerankerRoot = (Resolve-Path -LiteralPath $RerankerDir).Path

function Assert-NativeSuccess {
    param(
        [Parameter(Mandatory = $true)][string]$Step,
        [Parameter(Mandatory = $true)][int]$ExitCode
    )

    if ($ExitCode -ne 0) {
        throw "$Step 失败，退出码: $ExitCode"
    }
}

function Resolve-TestExecutable {
    param([string[]]$BuildOutput)

    $executables = foreach ($line in $BuildOutput) {
        try {
            $item = $line | ConvertFrom-Json -ErrorAction Stop
            if ($item.reason -eq 'compiler-artifact' -and
                $item.profile.test -eq $true -and
                $item.target.name -eq 'sanshu' -and
                $item.executable) {
                [string]$item.executable
            }
        }
        catch {
            continue
        }
    }
    $selected = $executables | Select-Object -Last 1
    if (-not $selected -or -not (Test-Path -LiteralPath $selected -PathType Leaf)) {
        throw 'cargo 未返回可执行的 sanshu library 测试二进制'
    }
    return (Resolve-Path -LiteralPath $selected).Path
}

$gateSuite = @{
    projects = @(
        @{
            name = 'java-debt-server'
            language = 'java'
            root = $javaRoot
            exclude_paths = @('.git', '.idea', 'target', 'logs', 'uploads', 'node_modules', 'dist', 'build')
            min_accurate_recall_at_5 = 0.75
            max_balanced_query_p95_ms = 500
            max_accurate_query_p95_ms = $AccurateMaxP95Ms
            queries = @(
                @{
                    id = 'java-plan-version'
                    kind = 'zh_intent'
                    query = '为每笔欠款生成新的协商版本并保留历史快照'
                    expected_paths = @('app/src/main/java/top/continew/admin/app/service/OrderMediationPlanService.java')
                },
                @{
                    id = 'java-progress-timing'
                    kind = 'zh_intent'
                    query = '工作流阶段切换时补全前序步骤的起止时刻'
                    expected_paths = @('app/src/main/java/top/continew/admin/app/service/OrderProgressTimingService.java')
                },
                @{
                    id = 'java-delivery-review'
                    kind = 'zh_intent'
                    query = '业务执行人员提交单项处理结果后由主管审核再复核'
                    expected_paths = @(
                        'app/src/main/java/top/continew/admin/app/controller/OrderDeliveryController.java',
                        'app/src/main/java/top/continew/admin/app/service/OrderDeliveryService.java'
                    )
                },
                @{
                    id = 'java-exact-service'
                    kind = 'exact_identifier'
                    query = 'OrderMediationPlanService'
                    expected_paths = @('app/src/main/java/top/continew/admin/app/service/OrderMediationPlanService.java')
                }
            )
        },
        @{
            name = 'go-codex-radar'
            language = 'go'
            root = $secondaryRoot
            exclude_paths = @('.git', 'target', 'node_modules', 'dist', 'build', 'data')
            min_accurate_recall_at_5 = 0.75
            max_balanced_query_p95_ms = 500
            max_accurate_query_p95_ms = $AccurateMaxP95Ms
            queries = @(
                @{
                    id = 'go-trusted-proxy-ip'
                    kind = 'zh_intent'
                    query = '由可信本机反向代理解析终端访问者地址'
                    expected_paths = @('client_ip.go')
                },
                @{
                    id = 'go-observation-store'
                    kind = 'zh_intent'
                    query = '把上游响应按内容指纹去重后写入事务记录'
                    expected_paths = @('storage_sqlite.go')
                },
                @{
                    id = 'go-health-route'
                    kind = 'zh_intent'
                    query = '注册存活探针以及模型评分的 HTTP 路由'
                    expected_paths = @('main.go')
                },
                @{
                    id = 'go-exact-client-ip'
                    kind = 'exact_identifier'
                    query = 'communityRatingClientIP'
                    expected_paths = @('client_ip.go')
                }
            )
        }
    )
}

$runRoot = Join-Path $projectRoot 'target\sou-semantic-phase2-gate'
New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
$stdoutPath = Join-Path $runRoot 'stdout.log'
$stderrPath = Join-Path $runRoot 'stderr.log'
$releaseMarkerPath = Join-Path $runRoot 'balanced-release.marker'
Remove-Item -LiteralPath $releaseMarkerPath -Force -ErrorAction SilentlyContinue
if (-not $ReportPath) {
    $ReportPath = Join-Path $runRoot 'report.json'
}
$reportFullPath = [System.IO.Path]::GetFullPath($ReportPath)
$reportParent = Split-Path -Parent $reportFullPath
New-Item -ItemType Directory -Path $reportParent -Force | Out-Null

Push-Location -LiteralPath $projectRoot
try {
    Write-Host "[1/3] 编译 $Profile library 测试二进制"
    $cargoArgs = @('test', '--lib', '--no-run', '--message-format=json')
    if ($Profile -eq 'release') {
        # 当前 Windows 主机并行优化编译会触发已知 CTFE 标量尺寸异常，门禁固定单 job 保证基线可复现。
        $cargoArgs = @('test', '--release', '--lib', '--no-run', '--message-format=json', '-j', '1')
    }
    $buildOutput = @(& cargo @cargoArgs 2>&1 | ForEach-Object { [string]$_ })
    $buildExitCode = $LASTEXITCODE
    foreach ($line in $buildOutput) {
        if (-not $line.StartsWith('{')) {
            Write-Host $line
        }
    }
    Assert-NativeSuccess 'phase2 门禁测试二进制编译' $buildExitCode
    $testExecutable = Resolve-TestExecutable -BuildOutput $buildOutput

    $env:SANSHU_SOU_REAL_GATE_JSON = $gateSuite | ConvertTo-Json -Depth 8 -Compress
    $env:SANSHU_SOU_20K_CHUNKS = [string]$SyntheticChunks
    $env:SANSHU_SOU_20K_MAX_P95_MS = [string]$SyntheticMaxP95Ms
    $env:SANSHU_SOU_GATE_INDEX_DIR = Join-Path $runRoot 'indexes'
    $env:SANSHU_SOU_GATE_RERANKER_DIR = $rerankerRoot
    # 仅测试二进制读取该覆盖值；门禁通过条件仍由 AccurateMaxP95Ms 控制。
    $env:SANSHU_SOU_GATE_ACCURATE_OBSERVATION_MS = [string]$AccurateObservationMs
    $env:SANSHU_SOU_GATE_RELEASE_MARKER = $releaseMarkerPath
    $env:SANSHU_SOU_GATE_RELEASE_WAIT_SECONDS = [string]$ReleaseWaitSeconds
    if ($ModelDir) {
        $env:SANSHU_SOU_GATE_MODEL_DIR = (Resolve-Path -LiteralPath $ModelDir).Path
    }
    else {
        Remove-Item Env:SANSHU_SOU_GATE_MODEL_DIR -ErrorAction SilentlyContinue
    }

    Write-Host '[2/3] 串行运行 Java、Go 与 20k chunks 门禁'
    $startedAt = [DateTimeOffset]::UtcNow
    $process = Start-Process `
        -FilePath $testExecutable `
        -ArgumentList @('phase2_gate_', '--ignored', '--nocapture', '--test-threads=1') `
        -PassThru `
        -WindowStyle Hidden `
        -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath
    $peakWorkingSetBytes = 0L
    $releaseObserved = $false
    $releaseMinimumWorkingSetBytes = [long]::MaxValue
    $releaseSampleCount = 0
    $releaseThresholdBytes = [long]$MaxBalancedRssMiB * 1MB
    $deadline = [DateTimeOffset]::UtcNow.AddMinutes($TimeoutMinutes)
    while (-not $process.HasExited) {
        $process.Refresh()
        if (-not $process.HasExited) {
            $peakWorkingSetBytes = [Math]::Max($peakWorkingSetBytes, $process.WorkingSet64)
            if (Test-Path -LiteralPath $releaseMarkerPath -PathType Leaf) {
                $releaseObserved = $true
                $releaseSampleCount++
                $releaseMinimumWorkingSetBytes = [Math]::Min(
                    $releaseMinimumWorkingSetBytes,
                    $process.WorkingSet64
                )
            }
        }
        if ([DateTimeOffset]::UtcNow -gt $deadline) {
            $process.Kill()
            throw "phase2 门禁超过 ${TimeoutMinutes} 分钟，已停止测试进程"
        }
        Start-Sleep -Milliseconds 200
    }
    $process.WaitForExit()
    $peakWorkingSetBytes = [Math]::Max($peakWorkingSetBytes, $process.PeakWorkingSet64)
    $elapsedMs = ([DateTimeOffset]::UtcNow - $startedAt).TotalMilliseconds
    $stdout = if (Test-Path -LiteralPath $stdoutPath) { @(Get-Content -LiteralPath $stdoutPath -Encoding UTF8) } else { @() }
    $stderr = if (Test-Path -LiteralPath $stderrPath) { @(Get-Content -LiteralPath $stderrPath -Encoding UTF8) } else { @() }
    $stdout | ForEach-Object { Write-Host $_ }
    $stderr | ForEach-Object { Write-Host $_ }

    Write-Host '[3/3] 汇总结构化报告'
    $results = foreach ($line in $stdout) {
        if ($line.StartsWith('SOU_PHASE2_GATE_RESULT=')) {
            $line.Substring('SOU_PHASE2_GATE_RESULT='.Length) | ConvertFrom-Json
        }
        elseif ($line.StartsWith('SOU_PHASE2_20K_RESULT=')) {
            $line.Substring('SOU_PHASE2_20K_RESULT='.Length) | ConvertFrom-Json
        }
        elseif ($line.StartsWith('SOU_PHASE2_RELEASE_RESULT=')) {
            $line.Substring('SOU_PHASE2_RELEASE_RESULT='.Length) | ConvertFrom-Json
        }
    }
    $peakThresholdBytes = [long]$MaxPeakMiB * 1MB
    $peakMemoryPassed = $peakWorkingSetBytes -le $peakThresholdBytes
    $releaseMemoryPassed = $releaseObserved -and
        $releaseSampleCount -gt 0 -and
        $releaseMinimumWorkingSetBytes -le $releaseThresholdBytes
    $report = [ordered]@{
        generated_at = [DateTimeOffset]::Now.ToString('o')
        profile = $Profile
        java_project = $javaRoot
        secondary_project = $secondaryRoot
        reranker_dir = $rerankerRoot
        test_executable = $testExecutable
        elapsed_ms = [Math]::Round($elapsedMs)
        peak_working_set_bytes = $peakWorkingSetBytes
        peak_working_set_mib = [Math]::Round($peakWorkingSetBytes / 1MB, 2)
        max_peak_mib = $MaxPeakMiB
        accurate_observation_ms = $AccurateObservationMs
        accurate_max_p95_ms = $AccurateMaxP95Ms
        peak_memory_passed = $peakMemoryPassed
        release_wait_seconds = $ReleaseWaitSeconds
        release_sample_count = $releaseSampleCount
        release_minimum_working_set_bytes = if ($releaseObserved) { $releaseMinimumWorkingSetBytes } else { $null }
        release_minimum_working_set_mib = if ($releaseObserved) { [Math]::Round($releaseMinimumWorkingSetBytes / 1MB, 2) } else { $null }
        max_balanced_rss_mib = $MaxBalancedRssMiB
        release_memory_passed = $releaseMemoryPassed
        test_exit_code = $process.ExitCode
        passed = $process.ExitCode -eq 0 -and $peakMemoryPassed -and $releaseMemoryPassed
        result_count = @($results).Count
        results = @($results)
    }
    $reportJson = $report | ConvertTo-Json -Depth 12
    [System.IO.File]::WriteAllText(
        $reportFullPath,
        $reportJson,
        [System.Text.UTF8Encoding]::new($false)
    )
    Write-Host "SOU_PHASE2_GATE_REPORT=$reportFullPath"
    Write-Host "SOU_PHASE2_GATE_PEAK_RSS_MIB=$($report.peak_working_set_mib)"
    Write-Host "SOU_PHASE2_GATE_BALANCED_MIN_RSS_MIB=$($report.release_minimum_working_set_mib)"
    if (@($results).Count -ne 4) {
        throw "期望 4 组结构化结果，实际获得 $(@($results).Count) 组"
    }
    Assert-NativeSuccess 'phase2 真实项目与 20k chunks 门禁' $process.ExitCode
    if (-not $peakMemoryPassed) {
        throw "测试进程峰值 $($report.peak_working_set_mib) MiB 超过门槛 $MaxPeakMiB MiB"
    }
    if (-not $releaseMemoryPassed) {
        throw "切回 Balanced 后 RSS 最低值 $($report.release_minimum_working_set_mib) MiB 超过门槛 $MaxBalancedRssMiB MiB"
    }
}
finally {
    Pop-Location
}
