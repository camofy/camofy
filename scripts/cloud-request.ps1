# Operator-only scoped API helper. Minted session expires in 10 minutes and is removed.
param([string]$Method='GET',[string]$Path='/resources',[string]$BodyFile,[object]$Body,[Parameter(Mandatory)][string]$AccountEmail,[string]$CloudOrigin='https://camofy.app')
$ErrorActionPreference='Stop'
if ($AccountEmail -notmatch '^[a-zA-Z0-9.@_+-]+$' -or !$Path.StartsWith('/')) { throw 'Invalid scope' }
$token=[Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32)).ToLowerInvariant()
$hash=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($token))).ToLowerInvariant()
$sql="INSERT INTO sessions(hash,user_id,expires_at) SELECT '$hash',id,now()+interval '10 minutes' FROM users WHERE email='$AccountEmail' RETURNING user_id;"
$uid=(($sql | ssh -o BatchMode=yes ym 'docker exec -i camofy-postgres psql -U camofy -d camofy -At -v ON_ERROR_STOP=1') | Where-Object {$_ -match '^[0-9a-f-]{36}$'})
if ($LASTEXITCODE -or !$uid) { throw 'Account/session unavailable' }
try {
    $args=@{Uri="$CloudOrigin/api$Path";Method=$Method;Headers=@{Authorization="Bearer $token";Origin=$CloudOrigin};TimeoutSec=60;SkipHttpErrorCheck=$true}
    if ($BodyFile) { $args.ContentType='application/json';$args.Body=Get-Content -LiteralPath $BodyFile -Raw }
    elseif ($null -ne $Body) { $args.ContentType='application/json';$args.Body=$Body | ConvertTo-Json -Depth 50 -Compress }
    $r=Invoke-WebRequest @args
    [pscustomobject]@{status=$r.StatusCode;body=($r.Content | ConvertFrom-Json -ErrorAction SilentlyContinue)} | ConvertTo-Json -Depth 50 -Compress
} finally {
    "DELETE FROM sessions WHERE hash='$hash';" | ssh -o BatchMode=yes ym 'docker exec -i camofy-postgres psql -U camofy -d camofy -At -v ON_ERROR_STOP=1' | Out-Null
}
