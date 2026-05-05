# GitHub 代理站可用性测试脚本
#
# 中文说明：用于区分代理站是否能访问 raw.githubusercontent.com 与 GitHub Release 下载链路。

param(
    [string]$RawUrl = "https://raw.githubusercontent.com/yuaotian/sanshu/refs/heads/main/announcements/latest.json",
    [string]$ReleaseUrl = "https://github.com/yuaotian/sanshu/releases/latest",
    [int]$TimeoutSeconds = 5
)

$ErrorActionPreference = "Stop"
$ProxyPrefixes = @(
    "https://wget.la/",
    "https://rapidgit.jjda.de5.net/",
    "https://fastgit.cc/",
    "https://gitproxy.mrhjx.cn/",
    "https://github.boki.moe/",
    "https://github.ednovas.xyz/"
)
$UserAgent = "sanshu-proxy-tester/1.0"
$script:InsecureCertificateEnabled = $false

function Enable-InsecureCertificate {
    if (-not $script:InsecureCertificateEnabled) {
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

function Test-ProxiedUrl {
    param(
        [string]$Prefix,
        [string]$TargetUrl
    )

    $url = Join-ProxyUrl -Prefix $Prefix -Url $TargetUrl
    $params = @{
        Uri = $url
        Method = "GET"
        UseBasicParsing = $true
        TimeoutSec = $TimeoutSeconds
        Headers = @{
            "User-Agent" = $UserAgent
        }
    }

    $command = Get-Command Invoke-WebRequest
    if ($command.Parameters.ContainsKey("SkipCertificateCheck")) {
        $params["SkipCertificateCheck"] = $true
    } else {
        Enable-InsecureCertificate
    }

    $started = Get-Date
    try {
        $response = Invoke-WebRequest @params
        $elapsed = [int]((Get-Date) - $started).TotalMilliseconds
        return [pscustomobject]@{
            Ok = $response.StatusCode -ge 200 -and $response.StatusCode -lt 400
            Ms = $elapsed
            Error = ""
        }
    } catch {
        $elapsed = [int]((Get-Date) - $started).TotalMilliseconds
        return [pscustomobject]@{
            Ok = $false
            Ms = $elapsed
            Error = $_.Exception.Message
        }
    }
}

$results = foreach ($prefix in $ProxyPrefixes) {
    Write-Host "测试代理站: $prefix" -ForegroundColor Cyan
    $raw = Test-ProxiedUrl -Prefix $prefix -TargetUrl $RawUrl
    $release = Test-ProxiedUrl -Prefix $prefix -TargetUrl $ReleaseUrl

    [pscustomobject]@{
        Proxy = $prefix
        RawOk = $raw.Ok
        RawMs = $raw.Ms
        ReleaseOk = $release.Ok
        ReleaseMs = $release.Ms
        Error = (($raw.Error, $release.Error) | Where-Object { $_ } | Select-Object -First 1)
    }
}

$results | Sort-Object @{ Expression = { -not ($_.RawOk -or $_.ReleaseOk) } }, RawMs, ReleaseMs | Format-Table -AutoSize
