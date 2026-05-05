# 三术在线安装脚本（Windows）
#
# 中文说明：该脚本不依赖本地 Rust/Node 构建环境，会从 GitHub Release 下载最新 CLI 包。

param(
    [string]$InstallDir = "",
    [switch]$NoPath = $false,
    [switch]$DryRun = $false,
    [int]$TimeoutSeconds = 8
)

$ErrorActionPreference = "Stop"

$ReleaseApiUrl = "https://api.github.com/repos/yuaotian/sanshu/releases/latest"
$ProxyPrefixes = @(
    "https://wget.la/",
    "https://rapidgit.jjda.de5.net/",
    "https://fastgit.cc/",
    "https://gitproxy.mrhjx.cn/",
    "https://github.boki.moe/",
    "https://github.ednovas.xyz/"
)
$LocalProxyPorts = @(7890, 7891, 7892, 10808, 8080)
$UserAgent = "sanshu-online-installer/1.0"
$script:InsecureCertificateEnabled = $false

function Enable-InsecureCertificate {
    if (-not $script:InsecureCertificateEnabled) {
        # 中文说明：仅代理站请求启用证书忽略，用于兼容证书过期的 GitHub 代理站。
        [System.Net.ServicePointManager]::ServerCertificateValidationCallback = { $true }
        $script:InsecureCertificateEnabled = $true
    }
}

function Join-ProxyUrl {
    param(
        [string]$Prefix,
        [string]$Url
    )
    return "$($Prefix.TrimEnd('/'))/$Url"
}

function Invoke-SanshuWebRequest {
    param(
        [string]$Url,
        [string]$OutFile = "",
        [string]$ProxyUrl = "",
        [switch]$IgnoreCertificate = $false,
        [int]$Timeout = $TimeoutSeconds
    )

    $params = @{
        Uri = $Url
        Method = "GET"
        UseBasicParsing = $true
        TimeoutSec = $Timeout
        Headers = @{
            "User-Agent" = $UserAgent
            "Accept" = "application/vnd.github+json"
        }
    }

    if ($OutFile) {
        $params["OutFile"] = $OutFile
    }
    if ($ProxyUrl) {
        $params["Proxy"] = $ProxyUrl
    }
    if ($IgnoreCertificate) {
        $command = Get-Command Invoke-WebRequest
        if ($command.Parameters.ContainsKey("SkipCertificateCheck")) {
            $params["SkipCertificateCheck"] = $true
        } else {
            Enable-InsecureCertificate
        }
    }

    return Invoke-WebRequest @params
}

function Get-CountryCode {
    try {
        $response = Invoke-SanshuWebRequest -Url "https://ipinfo.io/json" -Timeout 3
        $geo = $response.Content | ConvertFrom-Json
        if ($geo.country) {
            return [string]$geo.country
        }
    } catch {
        Write-Host "⚠️  IP 区域检测失败，按 UNKNOWN 处理: $($_.Exception.Message)" -ForegroundColor Yellow
    }
    return "UNKNOWN"
}

function Test-TcpPort {
    param(
        [string]$HostName,
        [int]$Port,
        [int]$TimeoutMs = 300
    )

    try {
        $client = New-Object System.Net.Sockets.TcpClient
        $async = $client.BeginConnect($HostName, $Port, $null, $null)
        $ok = $async.AsyncWaitHandle.WaitOne($TimeoutMs, $false)
        if (-not $ok) {
            $client.Close()
            return $false
        }
        $client.EndConnect($async)
        $client.Close()
        return $true
    } catch {
        return $false
    }
}

function Get-RequestCandidates {
    param([string]$Url)

    $country = Get-CountryCode
    Write-Host "🌍 检测到区域: $country" -ForegroundColor Cyan

    $candidates = New-Object System.Collections.Generic.List[object]
    $directTimeout = if ($country -eq "CN" -or $country -eq "UNKNOWN") { 3 } else { $TimeoutSeconds }
    $candidates.Add([pscustomobject]@{
        Label = "github-direct-$country"
        Url = $Url
        Proxy = ""
        IgnoreCertificate = $false
        Timeout = $directTimeout
    })

    foreach ($prefix in $ProxyPrefixes) {
        $candidates.Add([pscustomobject]@{
            Label = "github-proxy:$($prefix.TrimEnd('/'))"
            Url = Join-ProxyUrl -Prefix $prefix -Url $Url
            Proxy = ""
            IgnoreCertificate = $true
            Timeout = 4
        })
    }

    foreach ($port in $LocalProxyPorts) {
        if (Test-TcpPort -HostName "127.0.0.1" -Port $port) {
            $candidates.Add([pscustomobject]@{
                Label = "local-proxy:127.0.0.1:$port"
                Url = $Url
                Proxy = "http://127.0.0.1:$port"
                IgnoreCertificate = $false
                Timeout = $TimeoutSeconds
            })
        }
    }

    return $candidates
}

function Invoke-JsonWithStrategy {
    param([string]$Url)

    $errors = @()
    foreach ($candidate in Get-RequestCandidates -Url $Url) {
        try {
            Write-Host "🔎 尝试获取 JSON: $($candidate.Label)" -ForegroundColor Gray
            $response = Invoke-SanshuWebRequest `
                -Url $candidate.Url `
                -ProxyUrl $candidate.Proxy `
                -IgnoreCertificate:([bool]$candidate.IgnoreCertificate) `
                -Timeout $candidate.Timeout
            $json = $response.Content | ConvertFrom-Json
            return [pscustomobject]@{
                Json = $json
                Route = $candidate
            }
        } catch {
            $errors += "$($candidate.Label): $($_.Exception.Message)"
        }
    }

    throw "所有 GitHub JSON 路由均失败：$($errors -join ' | ')"
}

function Save-FileWithStrategy {
    param(
        [string]$Url,
        [string]$OutFile
    )

    $errors = @()
    foreach ($candidate in Get-RequestCandidates -Url $Url) {
        try {
            Write-Host "⬇️  尝试下载: $($candidate.Label)" -ForegroundColor Gray
            Invoke-SanshuWebRequest `
                -Url $candidate.Url `
                -OutFile $OutFile `
                -ProxyUrl $candidate.Proxy `
                -IgnoreCertificate:([bool]$candidate.IgnoreCertificate) `
                -Timeout 60 | Out-Null
            return $candidate
        } catch {
            Remove-Item -LiteralPath $OutFile -Force -ErrorAction SilentlyContinue
            $errors += "$($candidate.Label): $($_.Exception.Message)"
        }
    }

    throw "所有下载路由均失败：$($errors -join ' | ')"
}

function Resolve-SanshuInstallDir {
    if ($InstallDir) {
        return [System.IO.Path]::GetFullPath($InstallDir)
    }

    $systemDrive = ($env:SystemDrive.TrimEnd("\")).ToUpperInvariant()
    foreach ($drive in Get-PSDrive -PSProvider FileSystem) {
        if (-not $drive.Root) {
            continue
        }
        $driveRoot = $drive.Root.TrimEnd("\").ToUpperInvariant()
        if ($driveRoot -eq $systemDrive) {
            continue
        }
        return (Join-Path $drive.Root "mcp_server\sanshu")
    }

    if ($env:USERPROFILE) {
        return (Join-Path $env:USERPROFILE "mcp_server\sanshu")
    }
    return (Join-Path $env:LOCALAPPDATA "mcp_server\sanshu")
}

function Add-UserPath {
    param([string]$PathToAdd)

    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($currentPath -like "*$PathToAdd*") {
        return $true
    }

    $newPath = if ($currentPath) { "$currentPath;$PathToAdd" } else { $PathToAdd }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    return $true
}

function Copy-ReleaseFile {
    param(
        [string]$ExtractDir,
        [string]$FileName,
        [string]$TargetDir
    )

    $source = Get-ChildItem -LiteralPath $ExtractDir -Recurse -File -Filter $FileName | Select-Object -First 1
    if (-not $source) {
        throw "Release 包中缺少文件: $FileName"
    }
    Copy-Item -LiteralPath $source.FullName -Destination (Join-Path $TargetDir $FileName) -Force
}

Write-Host "🚀 三术在线安装开始" -ForegroundColor Green

$releaseResult = Invoke-JsonWithStrategy -Url $ReleaseApiUrl
$release = $releaseResult.Json
$tagName = [string]$release.tag_name
Write-Host "✅ 最新版本: $tagName via $($releaseResult.Route.Label)" -ForegroundColor Green

$asset = $release.assets | Where-Object { $_.name -like "*windows-x86_64*.zip" } | Select-Object -First 1
if (-not $asset) {
    throw "未找到 Windows x86_64 zip Release 资产"
}

$resolvedInstallDir = Resolve-SanshuInstallDir
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) "sanshu-online-install"
$archivePath = Join-Path $tempRoot $asset.name
$extractDir = Join-Path $tempRoot "extract"

Write-Host "📦 Release 资产: $($asset.name)" -ForegroundColor Cyan
Write-Host "📁 安装目录: $resolvedInstallDir" -ForegroundColor Cyan
Write-Host "🧪 DryRun: $DryRun" -ForegroundColor Cyan

if ($DryRun) {
    Write-Host "✅ DryRun 完成：未下载、未解压、未修改 PATH。" -ForegroundColor Green
    exit 0
}

New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null
Remove-Item -LiteralPath $extractDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $extractDir -Force | Out-Null
New-Item -ItemType Directory -Path $resolvedInstallDir -Force | Out-Null

$downloadRoute = Save-FileWithStrategy -Url ([string]$asset.browser_download_url) -OutFile $archivePath
Write-Host "✅ 下载完成: $archivePath via $($downloadRoute.Label)" -ForegroundColor Green

Expand-Archive -Path $archivePath -DestinationPath $extractDir -Force
foreach ($name in @("等一下.exe", "三术.exe", "sanshu.exe", "README.md", "sanshu_prompt_word.md", "sanshu_prompt_word_cli.md")) {
    Copy-ReleaseFile -ExtractDir $extractDir -FileName $name -TargetDir $resolvedInstallDir
}

$pathAdded = $false
if (-not $NoPath) {
    $answer = Read-Host "是否自动添加安装目录到用户 PATH？默认 Y，输入 n 跳过"
    if ([string]::IsNullOrWhiteSpace($answer) -or $answer.Trim().ToLowerInvariant() -ne "n") {
        $pathAdded = Add-UserPath -PathToAdd $resolvedInstallDir
        Write-Host "✅ 已添加到用户 PATH，请重启终端或编辑器生效" -ForegroundColor Green
    }
}

$commandValue = if ($pathAdded) { "sanshu" } else { (Join-Path $resolvedInstallDir "sanshu.exe") }

Write-Host ""
Write-Host "📝 MCP 配置（新建配置可直接使用）：" -ForegroundColor Cyan
$McpConfig = @"
{
  "mcpServers": {
    "sanshu": {
      "command": "$($commandValue.Replace('\', '\\'))"
    }
  }
}
"@
Write-Host $McpConfig -ForegroundColor Gray

Write-Host ""
Write-Host "🧩 如果已有 MCP 配置，请只把下面这一段插入既有 mcpServers 对象中，不要新建第二个 mcpServers：" -ForegroundColor Cyan
$McpServerEntry = @"
"sanshu": {
  "command": "$($commandValue.Replace('\', '\\'))"
}
"@
Write-Host $McpServerEntry -ForegroundColor Gray

Write-Host ""
Write-Host "📁 安装目录已打开: $resolvedInstallDir" -ForegroundColor Cyan
Start-Process explorer.exe $resolvedInstallDir
Write-Host "🎉 三术在线安装完成" -ForegroundColor Green
