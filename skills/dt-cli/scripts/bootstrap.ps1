param([string]$Directory, [switch]$Refresh)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
if (!$Directory) { $Directory = Join-Path $env:LOCALAPPDATA 'datousoft\dt-cli\data\installation' }
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or [Environment]::GetEnvironmentVariable('PROCESSOR_ARCHITEW6432') -eq 'ARM64' -or $env:PROCESSOR_ARCHITECTURE -ne 'AMD64') { throw 'PLATFORM_MISMATCH' }
$Directory = [IO.Path]::GetFullPath($Directory)
$cursor = $Directory
while ($cursor) {
    if ((Test-Path -LiteralPath $cursor) -and ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'BOOTSTRAP_DIRECTORY_INVALID' }
    $cursor = [IO.Path]::GetDirectoryName($cursor)
}
$config = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'distribution.json') -Raw | ConvertFrom-Json
$launcher = Join-Path $Directory 'bin\dt-cli.exe'
$temp = Join-Path ([IO.Path]::GetTempPath()) ('dt-cli-bootstrap-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($temp) | Out-Null
function Require($Value) { if (!$Value) { throw 'BOOTSTRAP_METADATA_INVALID' } }
function FixedVersion([string]$Value) {
    Require ($Value -cmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$')
    return [version]$Value
}
function HashFile($Path) {
    $sha = [Security.Cryptography.SHA256]::Create()
    $stream = [IO.File]::OpenRead($Path)
    try { return ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
    finally { $stream.Dispose(); $sha.Dispose() }
}
function Download($Key, $Path, [long]$Limit) {
    Require ($Key -cmatch '^(channels/stable\.json|releases/[a-zA-Z0-9./_-]+)$' -and !($Key.Split('/') | Where-Object { $_ -in @('', '.', '..') }))
    $request = [Net.HttpWebRequest]::Create($config.publicBaseUrl + $Key)
    $request.AllowAutoRedirect = $false
    $request.Timeout = 90000
    $request.ReadWriteTimeout = 90000
    $request.Headers['Cache-Control'] = 'no-cache'
    $response = $request.GetResponse()
    try {
        Require ([int]$response.StatusCode -eq 200 -and $response.ContentLength -le $Limit)
        $inputStream = $response.GetResponseStream()
        $outputStream = [IO.File]::Create($Path)
        try {
            $buffer = New-Object byte[] 65536
            $total = 0L
            while (($count = $inputStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                $total += $count
                Require ($total -le $Limit)
                $outputStream.Write($buffer, 0, $count)
            }
        } finally { $outputStream.Dispose(); $inputStream.Dispose() }
    } finally { $response.Dispose() }
}
function QuoteNative([string]$Value) {
    Require ($Value.IndexOf([char]0) -lt 0)
    $escaped = [regex]::Replace($Value, '(\\*)"', '$1$1\"')
    $escaped = [regex]::Replace($escaped, '(\\+)$', '$1$1')
    return '"' + $escaped + '"'
}
function Native($Binary, [string[]]$Arguments) {
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $Binary
    $start.Arguments = (($Arguments | ForEach-Object { QuoteNative $_ }) -join ' ')
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.StandardOutputEncoding = New-Object Text.UTF8Encoding($false)
    $start.CreateNoWindow = $true
    $process = [Diagnostics.Process]::Start($start)
    try {
        $text = $process.StandardOutput.ReadToEnd()
        Require ($process.WaitForExit(150000))
        if ($process.ExitCode -ne 0) { throw 'NATIVE_COMMAND_FAILED' }
        $body = $text | ConvertFrom-Json
        Require ($body.ok -eq $true)
        return $body.data
    } finally { $process.Dispose() }
}
try {
    Require ($config.schemaVersion -eq 1 -and $config.bootstrapSchema -eq 1 -and $config.channelKey -ceq 'channels/stable.json')
    $minimum = FixedVersion $config.minimumCliVersion
    $action = 'existing'; $update = 'not-checked'
    $probe = $null
    if (Test-Path -LiteralPath $launcher) { $probe = Native $launcher @('version') }
    if ($probe -and (FixedVersion $probe.cliVersion) -lt $minimum) {
        try {
            Native $launcher @('upgrade', '--online', '--minimum-version', $config.minimumCliVersion, '--directory', $Directory) | Out-Null
            $probe = Native $launcher @('version')
        } catch { }
    }
    if (!$probe -or (FixedVersion $probe.cliVersion) -lt $minimum -or $probe.cliVersion -eq '0.4.0') {
        if ($probe) { $action = 'upgrade' } else { $action = 'install' }
        Require ($config.publicBaseUrl -cmatch '^https://[a-zA-Z0-9.-]+(:[0-9]+)?/([a-zA-Z0-9_-]+/)*$')
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        $stablePath = Join-Path $temp 'stable.json'
        Download 'channels/stable.json' $stablePath 16384
        $stable = Get-Content -LiteralPath $stablePath -Raw | ConvertFrom-Json
        Require ($stable.schemaVersion -eq 1 -and $stable.sequence -gt 0 -and $stable.sequence -le 9007199254740991 -and $stable.sequence -eq [Math]::Floor($stable.sequence))
        Require ($stable.releaseKey -ceq ('releases/' + $stable.version + '/release.json') -and $stable.releaseSha256 -cmatch '^[0-9a-f]{64}$')
        $releasePath = Join-Path $temp 'release.json'
        Download $stable.releaseKey $releasePath 65536
        Require ((HashFile $releasePath) -ceq $stable.releaseSha256)
        $release = Get-Content -LiteralPath $releasePath -Raw | ConvertFrom-Json
        Require ($release.schemaVersion -eq 1 -and $release.version -ceq $stable.version -and (FixedVersion $release.version) -ge $minimum)
        Require ($release.buildCommit -cmatch '^[0-9a-f]{40}$' -and $release.catalogDigest -cmatch '^[0-9a-f]{64}$')
        $compatibility = $release.compatibility
        foreach ($field in @('bootstrapSchema', 'profileFormat', 'credentialFormat', 'installerSchema', 'launcherSchema')) { Require ($compatibility.$field -eq 1) }
        Require ((FixedVersion $config.skillVersion) -ge (FixedVersion $compatibility.minimumSkillVersion) -and (FixedVersion $config.skillVersion) -lt (FixedVersion $compatibility.maximumSkillVersionExclusive))
        Require ($release.packages.Count -eq 2)
        foreach ($expected in @(@('aarch64-apple-darwin', 'Darwin', 'arm64'), @('x86_64-pc-windows-msvc', 'Windows', 'x86_64'))) {
            $matches = @($release.packages | Where-Object { $_.target -ceq $expected[0] })
            Require ($matches.Count -eq 1)
            $p = $matches[0]
            Require ($p.os -ceq $expected[1] -and $p.architecture -ceq $expected[2] -and $p.key.StartsWith('releases/' + $release.version + '/'))
            Require ($p.sha256 -cmatch '^[0-9a-f]{64}$' -and $p.binarySha256 -cmatch '^[0-9a-f]{64}$' -and $p.bytes -gt 0 -and $p.bytes -le 268435456)
            if ($p.os -ceq 'Windows') { $selected = $p }
        }
        $archivePath = Join-Path $temp 'package.zip'
        Download $selected.key $archivePath $selected.bytes
        Require ((Get-Item -LiteralPath $archivePath).Length -eq $selected.bytes -and (HashFile $archivePath) -ceq $selected.sha256)
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $archive = [IO.Compression.ZipFile]::OpenRead($archivePath)
        $binary = Join-Path $temp 'dt-cli.exe'
        try {
            Require ($archive.Entries.Count -eq 4)
            foreach ($entryName in @('dt-cli.exe', 'manifest.json', 'install.sh', 'install.ps1')) {
                Require (@($archive.Entries | Where-Object { $_.FullName -ceq $entryName }).Count -eq 1)
            }
            $entry = $archive.GetEntry('manifest.json'); Require ($entry.Length -le 16384)
            $reader = New-Object IO.StreamReader($entry.Open())
            try { $manifest = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
            Require ($manifest.manifestSchemaVersion -eq 1 -and $manifest.releaseType -ceq 'release' -and $manifest.localDevelopment -eq $false -and $manifest.nativeProbe -ceq 'performed')
            Require ($manifest.os -ceq 'Windows' -and $manifest.architecture -ceq 'x86_64' -and $manifest.buildTarget -ceq 'x86_64-pc-windows-msvc' -and $manifest.binary -ceq 'dt-cli.exe')
            foreach ($field in @('profileFormat', 'credentialFormat', 'minimumInstallerSchema', 'minimumLauncherSchema')) { Require ($manifest.$field -eq 1) }
            Require ($manifest.version -ceq $release.version -and $manifest.buildCommit -ceq $release.buildCommit -and $manifest.catalogDigest -ceq $release.catalogDigest -and $manifest.sha256 -ceq $selected.binarySha256)
            $entry = $archive.GetEntry('dt-cli.exe'); Require ($entry.Length -le 268435456)
            $inputStream = $entry.Open(); $outputStream = [IO.File]::Create($binary)
            try { $inputStream.CopyTo($outputStream) } finally { $outputStream.Dispose(); $inputStream.Dispose() }
        } finally { $archive.Dispose() }
        Require ((HashFile $binary) -ceq $manifest.sha256)
        Native $binary @($action, '--package', $archivePath, '--sha256', $selected.sha256, '--directory', $Directory) | Out-Null
        $update = 'online'
    }
    $arguments = @('upgrade', '--online', '--minimum-version', $config.minimumCliVersion, '--directory', $Directory)
    if (!$Refresh) { $arguments += '--cached' }
    try {
        $updated = Native $launcher $arguments
        $update = $updated.updateCheck
        if ($updated.changed) { $action = 'upgrade' }
    } catch { $update = 'failed-compatible-existing' }
    $probe = Native $launcher @('version')
    Require ((FixedVersion $probe.cliVersion) -ge $minimum -and $probe.buildTarget -ceq 'x86_64-pc-windows-msvc' -and $probe.localDevelopment -eq $false)
    @{ok=$true; data=@{launcher=$launcher; version=$probe.cliVersion; action=$action; updateCheck=$update}; error=$null} | ConvertTo-Json -Compress
} catch {
    @{ok=$false; data=$null; error=@{code='BOOTSTRAP_FAILED'; message=$_.Exception.Message}} | ConvertTo-Json -Compress
    exit 1
} finally { Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue }
