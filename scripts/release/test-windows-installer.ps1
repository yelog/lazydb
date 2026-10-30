param([switch] $AllowUserPathMutation)

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'These behavioral fixtures require Windows.' }
if (-not $AllowUserPathMutation) { throw 'Use -AllowUserPathMutation on an isolated Windows account; fixtures temporarily change user PATH.' }

function Assert-True([bool] $Condition, [string] $Message) {
    if (-not $Condition) { throw "Assertion failed: $Message" }
}
function Write-Utf8([string] $Path, [string] $Text) {
    [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($false))
}
function Invoke-TestHost([string] $Executable, [string] $ScriptPath) {
    $process = Start-Process -FilePath $Executable -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$ScriptPath`"" -NoNewWindow -PassThru -RedirectStandardOutput (Join-Path $temp 'stdout.log') -RedirectStandardError (Join-Path $temp 'stderr.log')
    try {
        $null = $process.Handle
        if (-not $process.WaitForExit(60000)) { throw 'Fixture host timed out' }
        $process.WaitForExit()
        return $process.ExitCode
    } finally {
        if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
        $process.Dispose()
    }
}

$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$hostExe = (Get-Process -Id $PID).Path
$windowsPowerShell = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
$temp = Join-Path ([IO.Path]::GetTempPath()) ('lazydb fixtures ' + [IO.Path]::GetRandomFileName())
$oldPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$savedEnvironment = @{}
$names = @('HOME', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'TEMP', 'TMP', 'LAZYDB_INSTALL_DIR', 'LAZYDB_CHANNEL', 'LAZYDB_CHANNEL_BASE_URL', 'LAZYDB_MCP_SETUP', 'LAZYDB_FIXTURE_ROOT', 'LAZYDB_BOOTSTRAP_FAILURE')
foreach ($name in $names) { $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }

try {
    [IO.Directory]::CreateDirectory($temp) | Out-Null
    $env:LAZYDB_FIXTURE_ROOT = $temp
    # Only HTTP is replaced. Unknown URLs fail rather than silently returning a fixture.
    $downloadMock = @'
function Invoke-WebRequest {
    param([string] $Uri, [string] $OutFile, [switch] $UseBasicParsing, [int] $TimeoutSec)
    $root = $env:LAZYDB_FIXTURE_ROOT
    $source = switch -Exact ($Uri) {
        'https://lazydb.yelog.org/channels/stable.json' { 'manifest.json' }
        'https://lazydb.yelog.org/channels/beta.json' { 'manifest.json' }
        'https://github.com/yelog/lazydb/releases/download/v1.2.3/lazydb_1.2.3_x86_64-pc-windows-msvc.zip' { 'release.zip' }
        'https://github.com/yelog/lazydb/releases/download/v1.2.3-beta.1/lazydb_1.2.3-beta.1_x86_64-pc-windows-msvc.zip' { 'release.zip' }
        'https://lazydb.yelog.org/install.ps1' { 'downloaded-installer.ps1' }
        default { throw "Unexpected fixture URL: $Uri" }
    }
    [IO.File]::AppendAllText((Join-Path $root 'requests.log'), "$Uri`n")
    Copy-Item -LiteralPath (Join-Path $root $source) -Destination $OutFile -Force
}
'@
    $textDownloadMock = @'
function Invoke-RestMethod {
    [CmdletBinding()]
    param([string] $Uri)
    if ($Uri -ne 'https://lazydb.yelog.org/install.ps1.txt') { throw "Unexpected fixture URL: $Uri" }
    [IO.File]::AppendAllText((Join-Path $env:LAZYDB_FIXTURE_ROOT 'requests.log'), "$Uri`n")
    switch ($env:LAZYDB_BOOTSTRAP_FAILURE) {
        'network' { throw 'fixture download failed' }
        'empty' { return '' }
        'invalid' { return 'if (' }
        default { Get-Content -LiteralPath (Join-Path $env:LAZYDB_FIXTURE_ROOT 'downloaded-installer.ps1') -Raw }
    }
}
'@
    $installer = Get-Content -LiteralPath (Join-Path $root 'pages/install.ps1') -Raw
    Write-Utf8 (Join-Path $temp 'downloaded-installer.ps1') ($downloadMock + "`n" + $installer)
    Write-Utf8 (Join-Path $temp 'run.ps1') ('$ErrorActionPreference = ''Stop''' + "`n" + $downloadMock + "`n& '" + (Join-Path $root 'pages/install.ps1').Replace("'", "''") + "'`n")
    $readme = Get-Content -LiteralPath (Join-Path $root 'README.md') -Raw
    $bootstrap = [regex]::Match($readme, '(?s)```powershell\r?\n(.*?)\r?\n```')
    Assert-True $bootstrap.Success 'README contains a PowerShell bootstrap'
    $command = [regex]::Match($bootstrap.Groups[1].Value, '^powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "([^"]+)"$')
    Assert-True $command.Success 'README uses an isolated PowerShell command'
    # Execute the exact command payload in each test host, with only HTTP mocked.
    Write-Utf8 (Join-Path $temp 'bootstrap.ps1') ($textDownloadMock + "`n" + $command.Groups[1].Value + "`n" + 'if (-not $?) { exit 1 }')

    foreach ($failure in @('network', 'empty', 'invalid')) {
        $env:LAZYDB_BOOTSTRAP_FAILURE = $failure
        Assert-True ((Invoke-TestHost $hostExe (Join-Path $temp 'bootstrap.ps1')) -ne 0) "$failure bootstrap rejected"
    }
    Remove-Item Env:LAZYDB_BOOTSTRAP_FAILURE
    $installation = Get-Content -LiteralPath (Join-Path $root 'docs/installation.md') -Raw
    $fallback = [regex]::Match($installation, '(?s)<!-- windows-installer-file-start -->\s*```powershell\r?\n(.*?)\r?\n```\s*<!-- windows-installer-file-end -->')
    Assert-True $fallback.Success 'installation guide contains the file-based Windows installer'
    Write-Utf8 (Join-Path $temp 'fallback.ps1') ($downloadMock + "`n" + $fallback.Groups[1].Value)

    # Windows PowerShell emits a runnable .NET Framework EXE; pwsh 7 cannot emit
    # ConsoleApplication assemblies. Both test hosts execute the same real EXE.
    $buildScript = @'
$ErrorActionPreference = 'Stop'
Add-Type -OutputAssembly (Join-Path $env:LAZYDB_FIXTURE_ROOT 'lazydb.exe') -OutputType ConsoleApplication -TypeDefinition @"
using System;
public class Fixture {
    public static int Main(string[] args) {
        if (args.Length != 2 || args[0] != "version" || args[1] != "--json") return 2;
        string version = Environment.GetEnvironmentVariable("LAZYDB_CHANNEL") == "beta" ? "1.2.3-beta.1" : "1.2.3";
        Console.WriteLine("{\"version\":\"" + version + "\"}");
        return 0;
    }
}
"@
'@
    Write-Utf8 (Join-Path $temp 'build.ps1') $buildScript
    Assert-True ((Invoke-TestHost $windowsPowerShell (Join-Path $temp 'build.ps1')) -eq 0) 'fixture EXE compiles'
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $payload = Join-Path $temp 'payload'
    [IO.Directory]::CreateDirectory((Join-Path $payload 'nested [binary]')) | Out-Null
    Copy-Item -LiteralPath (Join-Path $temp 'lazydb.exe') -Destination (Join-Path $payload 'nested [binary]/lazydb.exe')
    [IO.Compression.ZipFile]::CreateFromDirectory($payload, (Join-Path $temp 'good.zip'))
    $emptyPayload = Join-Path $temp 'empty'
    [IO.Directory]::CreateDirectory($emptyPayload) | Out-Null
    Write-Utf8 (Join-Path $emptyPayload 'README.txt') 'No binary here'
    [IO.Compression.ZipFile]::CreateFromDirectory($emptyPayload, (Join-Path $temp 'missing.zip'))

    foreach ($scenario in @('success', 'beta', 'repeat', 'empty-path', 'bad-hash', 'invalid-json', 'invalid-manifest', 'invalid-url', 'missing-binary', 'version-mismatch', 'readme', 'fallback')) {
        $caseDir = Join-Path $temp $scenario
        $scratch = Join-Path $caseDir 'temp [scratch]'
        $env:HOME = Join-Path $caseDir 'home'
        $env:USERPROFILE = $env:HOME
        $env:APPDATA = Join-Path $caseDir 'appdata [config]'
        $env:LOCALAPPDATA = Join-Path $caseDir 'local'
        $env:TEMP = $scratch
        $env:TMP = $scratch
        $env:LAZYDB_INSTALL_DIR = Join-Path $caseDir 'bin [install]'
        $env:LAZYDB_CHANNEL = if ($scenario -eq 'beta') { 'beta' } else { 'stable' }
        $env:LAZYDB_CHANNEL_BASE_URL = 'https://lazydb.yelog.org/channels'
        $env:LAZYDB_MCP_SETUP = 'skip'
        foreach ($dir in @($scratch, $env:HOME, $env:APPDATA, $env:LOCALAPPDATA, $env:LAZYDB_INSTALL_DIR)) { [IO.Directory]::CreateDirectory($dir) | Out-Null }
        $statePath = Join-Path $env:APPDATA 'lazydb/install.json'
        [IO.Directory]::CreateDirectory((Split-Path $statePath)) | Out-Null
        $binaryPath = Join-Path $env:LAZYDB_INSTALL_DIR 'lazydb.exe'
        Write-Utf8 $binaryPath 'old executable'
        Write-Utf8 $statePath 'old state'
        $initialPath = if ($scenario -eq 'empty-path') { $null } elseif ($scenario -eq 'repeat') { "C:\fixture;;$env:LAZYDB_INSTALL_DIR;$env:LAZYDB_INSTALL_DIR;" } else { 'C:\fixture;;C:\other;' }
        [Environment]::SetEnvironmentVariable('Path', $initialPath, 'User')
        $initialPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $zip = if ($scenario -eq 'missing-binary') { 'missing.zip' } else { 'good.zip' }
        Copy-Item -LiteralPath (Join-Path $temp $zip) -Destination (Join-Path $temp 'release.zip') -Force
        $hash = (Get-FileHash -LiteralPath (Join-Path $temp 'release.zip') -Algorithm SHA256).Hash.ToLowerInvariant()
        $manifest = @{
            schema = 1; product = 'lazydb'; channel = $env:LAZYDB_CHANNEL; version = '1.2.3'
            assets = @{ 'x86_64-pc-windows-msvc' = @{ url = 'https://github.com/yelog/lazydb/releases/download/v1.2.3/lazydb_1.2.3_x86_64-pc-windows-msvc.zip'; sha256 = $hash } }
        }
        $expectedVersion = if ($scenario -eq 'beta') { '1.2.3-beta.1' } else { '1.2.3' }
        $manifest.version = $expectedVersion
        $manifest.assets.'x86_64-pc-windows-msvc'.url = "https://github.com/yelog/lazydb/releases/download/v$expectedVersion/lazydb_$($expectedVersion)_x86_64-pc-windows-msvc.zip"
        switch ($scenario) {
            'bad-hash' { $manifest.assets.'x86_64-pc-windows-msvc'.sha256 = '0' * 64 }
            'invalid-manifest' { $manifest.product = 'other' }
            'invalid-url' { $manifest.assets.'x86_64-pc-windows-msvc'.url = 'https://example.com/release.zip' }
            'version-mismatch' { $manifest.version = '9.9.9' }
        }
        $json = if ($scenario -eq 'invalid-json') { '{broken' } else { $manifest | ConvertTo-Json -Depth 5 }
        Write-Utf8 (Join-Path $temp 'manifest.json') $json
        Write-Utf8 (Join-Path $temp 'requests.log') ''
        $scriptName = if ($scenario -eq 'readme') { 'bootstrap.ps1' } elseif ($scenario -eq 'fallback') { 'fallback.ps1' } else { 'run.ps1' }
        $result = Invoke-TestHost $hostExe (Join-Path $temp $scriptName)
        $failure = $scenario -in @('bad-hash', 'invalid-json', 'invalid-manifest', 'invalid-url', 'missing-binary', 'version-mismatch')
        if ($failure) {
            Assert-True ($result -ne 0) "$scenario rejected"
            $expectedError = switch ($scenario) {
                'bad-hash' { 'checksum mismatch' }
                'invalid-json' { 'ConvertFrom-Json' }
                'invalid-manifest' { 'manifest identity mismatch' }
                'invalid-url' { 'invalid Windows release asset' }
                'missing-binary' { 'archive does not contain lazydb.exe' }
                'version-mismatch' { 'staged binary failed version check' }
            }
            Assert-True ((Get-Content -LiteralPath (Join-Path $temp 'stderr.log') -Raw) -match [regex]::Escape($expectedError)) "$scenario fails for the expected reason"
            Assert-True (([IO.File]::ReadAllText($binaryPath)) -ceq 'old executable') "$scenario preserves old binary"
            Assert-True (([IO.File]::ReadAllText($statePath)) -ceq 'old state') "$scenario preserves old state"
            Assert-True ([object]::Equals([Environment]::GetEnvironmentVariable('Path', 'User'), $initialPath)) "$scenario preserves user PATH"
        } else {
            Assert-True ($result -eq 0) "$scenario installs successfully"
            Assert-True ((Get-FileHash -LiteralPath $binaryPath).Hash -eq (Get-FileHash -LiteralPath (Join-Path $temp 'lazydb.exe')).Hash) "$scenario installs exact binary"
            $version = & $binaryPath version --json
            Assert-True ($LASTEXITCODE -eq 0 -and ($version | ConvertFrom-Json).version -eq $expectedVersion) "$scenario runs installed binary"
            $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
            Assert-True ($state.schema -eq 1 -and $state.product -eq 'lazydb' -and $state.manager -eq 'native' -and $state.version -eq $expectedVersion -and $state.channel -eq $env:LAZYDB_CHANNEL -and $state.target -eq 'x86_64-pc-windows-msvc' -and $state.path -eq $binaryPath) "$scenario records installation identity"
            $bytes = [IO.File]::ReadAllBytes($statePath)
            Assert-True (-not ($bytes.Length -ge 3 -and $bytes[0] -eq 239 -and $bytes[1] -eq 187 -and $bytes[2] -eq 191)) "$scenario state has no BOM"
            $expectedPath = if ($scenario -eq 'repeat') { $initialPath } else { (([string]$initialPath).TrimEnd(';') + ';' + $env:LAZYDB_INSTALL_DIR).Trim(';') }
            Assert-True ([Environment]::GetEnvironmentVariable('Path', 'User') -ceq $expectedPath) "$scenario user PATH"
            Assert-True ((Invoke-TestHost $hostExe (Join-Path $temp $scriptName)) -eq 0) "$scenario reinstall succeeds"
            Assert-True ([Environment]::GetEnvironmentVariable('Path', 'User') -ceq $expectedPath) "$scenario reinstall leaves PATH unchanged"
        }
        Assert-True (@(Get-ChildItem -LiteralPath $scratch -Force).Count -eq 0) "$scenario cleans all temporary downloads/extraction/bootstrap files"
        $requests = @(Get-Content -LiteralPath (Join-Path $temp 'requests.log'))
        Assert-True ($requests -contains "https://lazydb.yelog.org/channels/$env:LAZYDB_CHANNEL.json") "$scenario requests selected channel"
        $expectedCount = if ($failure) { if ($scenario -in @('invalid-json', 'invalid-manifest', 'invalid-url')) { 1 } else { 2 } } elseif ($scenario -in @('readme', 'fallback')) { 6 } else { 4 }
        Assert-True ($requests.Count -eq $expectedCount) "$scenario requests only expected URLs"
        Write-Host "Passed: $scenario ($hostExe)"
    }
} finally {
    try { [Environment]::SetEnvironmentVariable('Path', $oldPath, 'User') } finally {
        foreach ($name in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process') }
        Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
    }
}
Write-Host 'Windows installer behavioral fixtures passed.'
