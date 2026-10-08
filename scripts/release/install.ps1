# Verify the downloaded ZIP/executable; respect existing execution policy and preserve native argv.
$ErrorActionPreference = 'Stop'
$installArguments = @($args)
function Get-InstallOption([string] $Name) {
    for ($i = 0; $i -lt $installArguments.Count; $i++) {
        $value = [string]$installArguments[$i]
        if ($value -eq $Name -and $i + 1 -lt $installArguments.Count) { return [string]$installArguments[$i + 1] }
        if ($value.StartsWith($Name + '=')) { return $value.Substring($Name.Length + 1) }
    }
    return ''
}
$package = Get-InstallOption '--package'
$expected = Get-InstallOption '--sha256'
if (!$package -or $expected -cnotmatch '^[0-9a-f]{64}$') { throw 'Provide --package and the ZIP --sha256 from the trusted distribution entry.' }
if ((Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expected) { throw 'ZIP checksum mismatch.' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $package).ProviderPath)
try {
    $entry = $archive.GetEntry('manifest.json')
    if (!$entry -or $entry.Length -gt 16384) { throw 'Invalid package manifest.' }
    $reader = New-Object System.IO.StreamReader($entry.Open())
    try { $manifest = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
} finally { $archive.Dispose() }
$binary = Join-Path $PSScriptRoot 'dt-cli.exe'
if ((Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant() -cne $manifest.sha256) { throw 'Extracted binary does not match the verified ZIP.' }
function ConvertTo-NativeArgument([string] $Value) {
    if ($Value.IndexOf([char]0) -ge 0) { throw 'Native arguments cannot contain NUL.' }
    $escaped = [regex]::Replace($Value, '(\\*)"', '$1$1\"')
    $escaped = [regex]::Replace($escaped, '(\\+)$', '$1$1')
    return '"' + $escaped + '"'
}
$start = New-Object System.Diagnostics.ProcessStartInfo
$start.FileName = $binary
$start.UseShellExecute = $false
$start.WorkingDirectory = (Get-Location).ProviderPath
$start.Arguments = ((@('install') + $installArguments) | ForEach-Object { ConvertTo-NativeArgument ([string]$_) }) -join ' '
$process = [System.Diagnostics.Process]::Start($start)
$process.WaitForExit()
exit $process.ExitCode
