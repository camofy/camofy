# Operator workflow: no stored login password; exact account and egress IP are required.
param([Parameter(Mandatory)][string]$ConfigFile,[Parameter(Mandatory)][string]$ExpectedIP,[Parameter(Mandatory)][string]$AccountEmail)
$ErrorActionPreference='Stop'
$config=Get-Content -LiteralPath $ConfigFile -Raw | ConvertFrom-Json -AsHashtable
function Call-Cloud([string]$Method,[string]$Path,[object]$Data) {
    $r=& "$PSScriptRoot/cloud-request.ps1" -Method $Method -Path $Path -Body $Data -AccountEmail $AccountEmail | ConvertFrom-Json
    if ($r.status -notin @(200,202,204)) { throw "Cloud request failed ($($r.status)): $($r.body.error)" }
    return $r.body
}
$resources=Call-Cloud GET /resources $null
$existing=@($resources | Where-Object {$_.kind -eq 'proxy' -and $_.data.name -eq $config.name})
if ($existing.Count -gt 1) { throw 'Ambiguous proxy name; resolve duplicates first' }
$preview=Call-Cloud POST /proxies/egress-preview @{}
if ($preview.ip -ne $ExpectedIP) { throw "Server egress changed: $($preview.ip); expected $ExpectedIP. No whitelist was changed." }
Write-Host "Verified server egress: $($preview.ip). Adding only this whitelist entry."
$config.egress_preview=$preview.proof
$payload=@{kind='proxy';data=$config}
if ($existing.Count) {
    $payload.version=$existing[0].version
    $proxy=Call-Cloud PUT "/resources/$($existing[0].id)" $payload
} else { $proxy=Call-Cloud POST /resources $payload }
Write-Host "Proxy saved: $($proxy.id); whitelist $($proxy.data.whitelist_ip). Credentials not displayed."
$resources=Call-Cloud GET /resources $null
foreach ($source in @($resources | Where-Object {$_.kind -eq 'profile' -and $_.data.type -eq 'source'})) {
    $source.data.proxy_id=$proxy.id
    $saved=Call-Cloud PUT "/resources/$($source.id)" @{kind='profile';version=$source.version;data=$source.data}
    # Saving auto-refresh profiles queues immediately; others need an explicit refresh.
    if ($saved.data.auto_refresh -eq $false) { $null=Call-Cloud POST "/profiles/$($saved.id)/refresh" @{} }
    Write-Host "Queued via Xiequ: $($saved.data.name) ($($saved.id))"
}
