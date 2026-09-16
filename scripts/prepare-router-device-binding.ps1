# One-time operator migration. Outputs are private, ignored deployment artifacts.
param([string]$BackupDirectory='output/router-device-control-20260916',[Parameter(Mandatory)][string]$AccountEmail,[Parameter(Mandatory)][string]$DeviceId,[Parameter(Mandatory)][string]$IdentityId)
$ErrorActionPreference='Stop'
$settings=Get-Content (Join-Path $BackupDirectory 'agent.before.json') -Raw | ConvertFrom-Json -AsHashtable
$records=(& "$PSScriptRoot/cloud-request.ps1" -AccountEmail $AccountEmail | ConvertFrom-Json).body
$device=$records | Where-Object { $_.kind -eq 'device' -and $_.id -eq $DeviceId }
if (!$device -or $device.data.bundle_id -ne $IdentityId) {throw 'Router identity changed; inspect before migration'}
$identity=$records | Where-Object { $_.id -eq $device.data.bundle_id }
$grant=& "$PSScriptRoot/cloud-request.ps1" -AccountEmail $AccountEmail -Method POST -Path /tokens -Body @{bundle_id=$device.data.bundle_id;device_id=$device.id;label='router device control'} | ConvertFrom-Json
if ($grant.status -ne 200 -or !$grant.body.token) {throw 'Cannot provision device credential'}
$settings.Remove('subscription_url')
$settings.cloud_url='https://camofy.app'
$settings.device_token=$grant.body.token
$settings.device_id=$device.id
$settings.identity_name=$identity.data.name
$settings.web_listen='192.168.50.1:3000'
if ($settings.dns_redirect -ne $false) {throw 'Unexpected DNS interception setting'}
[IO.File]::WriteAllText((Join-Path (Resolve-Path $BackupDirectory) 'agent.device.json'),($settings | ConvertTo-Json -Depth 30),[Text.UTF8Encoding]::new($false))
Write-Output 'Device-scoped credential prepared privately; identity and DNS policy retained.'
