# Read-only production diagnostic: one fresh extraction and one source request per run.
param([Parameter(Mandatory)][string]$ProfileId,[Parameter(Mandatory)][string]$ConfigFile,[Parameter(Mandatory)][string]$AccountEmail)
$ErrorActionPreference='Stop'
$config=Get-Content -LiteralPath $ConfigFile -Raw | ConvertFrom-Json
$resources=(& "$PSScriptRoot/cloud-request.ps1" -AccountEmail $AccountEmail | ConvertFrom-Json).body
$source=$resources | Where-Object {$_.id -eq $ProfileId -and $_.data.type -eq 'source'}
if (!$source) { throw 'Subscription not found in the scoped account' }
$dns=ssh -o BatchMode=yes ym 'curl -4 -fsS --max-time 12 "https://dns.google/resolve?name=api.xiequ.cn&type=A&edns_client_subnet=223.5.5.0/24"' | ConvertFrom-Json
$address=($dns.Answer | Where-Object {$_.type -eq 1} | Select-Object -First 1).data
if ($address -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'No CDN IPv4' }
$uri=[uri]$source.data.url
$resolved=(ssh -o BatchMode=yes ym "getent ahostsv4 '$($uri.DnsSafeHost)'" | Select-Object -First 1).Split(' ',[StringSplitOptions]::RemoveEmptyEntries)[0]
if ($resolved -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'No target IPv4' }
$extractConfig='url = "'+$config.extract_url.Replace('\','\\').Replace('"','\"')+'"'
$response=$extractConfig | ssh -o BatchMode=yes ym "curl -4 --noproxy '*' -fsS --max-time 15 --resolve 'api.xiequ.cn:80:$address' --config -"
$extracted=$response | ConvertFrom-Json
if ($extracted.code -ne 0 -or $extracted.data.Count -ne 1) { throw 'Extraction failed' }
$ip=$extracted.data[0].IP
$port=$extracted.data[0].Port
if ($ip -notmatch '^\d+\.\d+\.\d+\.\d+$' -or "$port" -notmatch '^\d+$') { throw 'Invalid proxy' }
$fetchConfig=@(
    ('url = "'+$source.data.url.Replace('\','\\').Replace('"','\"')+'"'),
    ('proxy = "http://'+$ip+':'+$port+'"'),
    ('connect-to = "'+$uri.DnsSafeHost+':'+$uri.Port+':'+$resolved+':'+$uri.Port+'"')
) -join "`n"
Write-Host "Testing $($source.data.name); fresh proxy ${ip}:${port}; pinned target $resolved."
$fetchConfig | ssh -o BatchMode=yes ym 'curl -4 --noproxy "" -sS --connect-timeout 10 --max-time 25 -A "clash-verge/camofy-cloud" -o /dev/null -w "http=%{http_code} CONNECT=%{http_connect} bytes=%{size_download} remote=%{remote_ip} elapsed=%{time_total}\n" --config -'
