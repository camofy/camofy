# Explicit migration helper: read a local router backup, write only the named
# tenant through the cloud API. No router writes. SSH ym is used solely to mint
# a short-lived migration session for this authorized account, removed in finally.
param(
    [Parameter(Mandatory)][string]$BackupDir,
    [Parameter(Mandatory)][string]$AccountEmail,
    [Parameter(Mandatory)][string]$NewSubscriptionUrl,
    [string]$CloudOrigin = 'https://camofy.app'
)
$ErrorActionPreference = 'Stop'
if ($AccountEmail -notmatch '^[a-zA-Z0-9.@_+-]+$') { throw 'Unsupported account email characters' }
$configDir = Join-Path (Resolve-Path $BackupDir).Path 'camofy/config'
$old = Get-Content (Join-Path $configDir 'app.json') -Raw | ConvertFrom-Json
$sessionToken = [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32)).ToLowerInvariant()
$sessionHash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($sessionToken))).ToLowerInvariant()
$sql = "INSERT INTO sessions(hash,user_id,expires_at) SELECT '$sessionHash',id,now()+interval '30 minutes' FROM users WHERE email='$AccountEmail' RETURNING user_id;"
$userId = (($sql | ssh -o BatchMode=yes ym 'docker exec -i camofy-postgres psql -U camofy -d camofy -At -v ON_ERROR_STOP=1') | Where-Object { $_ -match '^[0-9a-f-]{36}$' })
if ($LASTEXITCODE -or !$userId) { throw 'Cannot create scoped session for the requested existing account' }
$headers = @{Authorization="Bearer $sessionToken";Origin=$CloudOrigin}
function Call-Api($method,$path,$body) {
    $args = @{Uri="$CloudOrigin/api$path";Method=$method;Headers=$headers;TimeoutSec=60}
    if ($null -ne $body) { $args.ContentType='application/json'; $args.Body=($body | ConvertTo-Json -Depth 30 -Compress) }
    try { Invoke-RestMethod @args } catch {
        $reason = 'details suppressed to protect configuration secrets'
        if ($body.kind -eq 'bundle') { $reason = ($_.ErrorDetails.Message | ConvertFrom-Json).error }
        throw "Migration API failed: $method $path, HTTP $([int]$_.Exception.Response.StatusCode), resource '$($body.data.name)': $reason"
    }
}
try {
    $loaded = Call-Api GET '/resources' $null
    $script:existing = @($loaded | ForEach-Object { $_ })
    function Ensure-Resource($kind,$key,$data) {
        $found = $script:existing | Where-Object { $_.kind -eq $kind -and $_.data.migration_key -eq $key } | Select-Object -First 1
        if ($found) { return $found }
        $data.migration_key = $key
        $r = Call-Api POST '/resources' @{kind=$kind;data=$data}
        $script:existing += $r
        return $r
    }
    $bindings = @()
    $imported = @()
    $defaults = Ensure-Resource 'profile' 'router-legacy-defaults' @{name='路由器旧默认参数';type='overlay';content=(Get-Content (Join-Path $configDir 'defaults.yaml') -Raw)}
    $bindings += @{profile_id=$defaults.id;enabled=$true}
    foreach ($p in $old.profiles) {
        $file = [IO.Path]::GetFullPath((Join-Path $configDir $p.path))
        if (!$file.StartsWith($configDir + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Backup profile path escapes config directory' }
        $content = Get-Content -LiteralPath $file -Raw
        if ([string]::IsNullOrWhiteSpace($content)) { $content='{}' }
        elseif (!(@($content -split "`n" | Where-Object { $_.Trim() -and !$_.Trim().StartsWith('#') }).Count)) { $content += "`n{}" }
        if ($p.profile_type -eq 'remote') {
            $source = Ensure-Resource 'profile' "router-source-$($p.id)" @{name=$p.name;type='source';url=$p.url;proxy_id=$null;auto_refresh=[bool]$old.subscription_auto_update.enabled;interval_seconds=86400}
            # Preserve exactly what was on the router even if its upstream has changed or expired.
            $snapshot = Ensure-Resource 'profile' "router-snapshot-$($p.id)" @{name="旧订阅快照 · $($p.name)";type='overlay';content=$content}
            $bindings += @{profile_id=$source.id;enabled=$false}
            $bindings += @{profile_id=$snapshot.id;enabled=($p.id -eq $old.active_subscription_id)}
            $imported += @{old_id=$p.id;source_id=$source.id;snapshot_id=$snapshot.id}
        } else {
            $independent = Ensure-Resource 'profile' "router-independent-$($p.id)" @{name=$p.name;type='overlay';content=$content}
            $bindings += @{profile_id=$independent.id;enabled=($p.id -eq $old.active_user_profile_id)}
            $imported += @{old_id=$p.id;profile_id=$independent.id}
        }
    }
    $legacy = Ensure-Resource 'bundle' 'router-legacy-identity' @{name='路由器旧配置（迁移留存）';profiles=$bindings;selections=@{};legacy_selections=$old.proxy_selections;legacy_schedule=$old.subscription_auto_update}
    if (!$legacy.data.published_revision) { throw 'Legacy identity failed to publish; do not replace router' }
    $testSource = Ensure-Resource 'profile' 'router-test-subscription' @{name='路由器联调订阅';type='source';url=$NewSubscriptionUrl;proxy_id=$null;auto_refresh=$true;interval_seconds=3600}
    $testYaml = @'
# A normal composable profile; no Agent special-case and no traffic interception.
tun:
  enable: false
mixed-port: 17890
port: 0
socks-port: 0
redir-port: 0
tproxy-port: 0
allow-lan: false
bind-address: 127.0.0.1
dns:
  listen: 127.0.0.1:15353
log-level: warning
'@
    $testParameters = Ensure-Resource 'profile' 'router-no-tun-parameters' @{name='测试运行参数 · 关闭 TUN';type='overlay';content=$testYaml}
    $testIdentity = Ensure-Resource 'bundle' 'router-test-identity' @{name='路由器联调（无 TUN）';profiles=@(@{profile_id=$testSource.id;enabled=$true},@{profile_id=$testParameters.id;enabled=$true});selections=@{}}
    $device = Ensure-Resource 'device' 'router-managed-device' @{name='router';bundle_id=$testIdentity.id}
    # Device URL is intentionally not issued here: the router will consume the canonical identity URL.
    # Reports/latency control remain opt-in via a separately issued device URL.
    [pscustomobject]@{user_id=$userId;imported=$imported;legacy_identity=$legacy.id;test_identity=$testIdentity.id;test_profile=$testSource.id;parameters_profile=$testParameters.id;device_id=$device.id;subscription_url=$testIdentity.data.subscription_url} | ConvertTo-Json -Depth 20 -Compress
} finally {
    "DELETE FROM sessions WHERE hash='$sessionHash' AND user_id='$userId';" | ssh -o BatchMode=yes ym 'docker exec -i camofy-postgres psql -U camofy -d camofy -At -v ON_ERROR_STOP=1' | Out-Null
    if ($LASTEXITCODE) { Write-Warning 'Migration session cleanup failed; it expires after 30 minutes.' }
}
