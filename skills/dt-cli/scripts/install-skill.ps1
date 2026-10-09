param([Parameter(Mandatory=$true)][string]$Directory, [switch]$Check, [switch]$Rollback)
$ErrorActionPreference = 'Stop'
$prepared = (& "$PSScriptRoot/bootstrap.ps1" | ConvertFrom-Json)
if ($LASTEXITCODE -ne 0 -or -not $prepared.ok) { throw 'BOOTSTRAP_FAILED' }
$arguments = @('skill','install','--directory',$Directory)
if ($Check) { $arguments += '--check' }
if ($Rollback) { $arguments += '--rollback' }
& $prepared.data.launcher @arguments
exit $LASTEXITCODE
