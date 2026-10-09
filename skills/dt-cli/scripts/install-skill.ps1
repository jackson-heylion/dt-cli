param([Parameter(Mandatory=$true)][string]$Directory, [switch]$Check, [switch]$Rollback, [string]$InstallationDirectory)
$ErrorActionPreference = 'Stop'
$bootstrapArguments = @{}
if ($InstallationDirectory) { $bootstrapArguments.Directory = $InstallationDirectory }
$prepared = (& "$PSScriptRoot/bootstrap.ps1" @bootstrapArguments | ConvertFrom-Json)
if ($prepared.ok -ne $true -or -not $prepared.data.launcher -or -not [IO.Path]::IsPathRooted($prepared.data.launcher) -or -not (Test-Path -LiteralPath $prepared.data.launcher -PathType Leaf)) {
    $reason = if ($prepared.error.message) { $prepared.error.message } else { 'Invalid bootstrap response or missing launcher' }
    throw "BOOTSTRAP_FAILED: $reason"
}
$arguments = @('skill','install','--directory',$Directory)
if ($Check) { $arguments += '--check' }
if ($Rollback) { $arguments += '--rollback' }
& $prepared.data.launcher @arguments
exit $LASTEXITCODE
