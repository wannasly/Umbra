param([switch]$VerifyOnly)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
Push-Location -LiteralPath $taskRoot
try {
    $metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
    $version = ($metadata.packages | Where-Object name -eq umbra).version
    $archive = Join-Path $taskRoot "release/Umbra_${version}_windows_x64_portable.zip"
    $installer = Join-Path $taskRoot "release/Umbra_${version}_x64-setup.exe"
    if (-not (Test-Path -LiteralPath $installer)) { throw 'Installer missing' }
    $zip = [IO.Compression.ZipFile]::OpenRead($archive)
    try {
        foreach ($name in @('Umbra.exe','resources/sing-box.exe','LICENSE.txt','THIRD_PARTY_NOTICES.md','licenses/Slint-Royalty-free-2.0.md','licenses/inter-OFL.txt','licenses/manrope-OFL.txt','licenses/jetbrainsmono-OFL.txt','licenses/Lucide-LICENSE.txt','licenses/flag-icons-LICENSE.txt','licenses/sing-box-GPL-3.0.txt')) {
            if (-not $zip.GetEntry($name)) { throw "Portable archive is missing $name" }
        }
        foreach ($pair in @(@('Umbra.exe','target/release/umbra.exe'),@('resources/sing-box.exe','src-tauri/resources/sing-box.exe'))) {
            $stream = $zip.GetEntry($pair[0]).Open()
            $sha = [Security.Cryptography.SHA256]::Create()
            try { $actual = [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-','') }
            finally { $stream.Dispose(); $sha.Dispose() }
            if ($actual -ne (Get-FileHash -LiteralPath $pair[1] -Algorithm SHA256).Hash) { throw "Portable checksum mismatch: $($pair[0])" }
        }
    } finally { $zip.Dispose() }
    Write-Output 'Portable manifest and binary hashes verified.'
    if ($VerifyOnly) { return }
    # A clean hosted runner has no user's existing Umbra registration to overwrite.
    if ($env:GITHUB_ACTIONS -ne 'true' -or -not $env:RUNNER_TEMP) { throw 'Installation smoke test requires a clean GitHub Actions runner; use -VerifyOnly locally.' }
    if (Test-Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Umbra') { throw 'Existing Umbra installation must be preserved.' }
    $installDir = Join-Path $env:RUNNER_TEMP 'UmbraInstallerSmoke'
    if (Test-Path -LiteralPath $installDir) { throw 'Installer smoke destination already exists.' }
    $defaultDataDir = Join-Path $env:APPDATA 'com.umbra.proxy'
    New-Item -ItemType Directory -Path $defaultDataDir -Force | Out-Null
    $sentinel = Join-Path $defaultDataDir ('installer-smoke-' + [guid]::NewGuid().ToString('N') + '.txt')
    Set-Content -LiteralPath $sentinel -Value 'preserve user data' -Encoding ascii
    $process = Start-Process -FilePath $installer -ArgumentList "/S /D=$installDir" -PassThru -WindowStyle Hidden
    $null = $process.Handle
    if (-not $process.WaitForExit(60000) -or $process.ExitCode -ne 0) { throw 'Silent installation failed.' }
    foreach ($relative in @('Umbra.exe','resources/sing-box.exe','licenses/Slint-Royalty-free-2.0.md','Uninstall.exe')) {
        if (-not (Test-Path -LiteralPath (Join-Path $installDir $relative))) { throw "Installed file missing: $relative" }
    }
    if ((Get-FileHash -LiteralPath (Join-Path $installDir 'Umbra.exe')).Hash -ne (Get-FileHash target/release/umbra.exe).Hash) { throw 'Installed application differs from the build.' }
    $savedData = $env:UMBRA_DATA_DIR
    $smokeProcess = $null
    try {
        $env:UMBRA_DATA_DIR = Join-Path $env:RUNNER_TEMP 'UmbraAppSmokeData'
        $stderr = Join-Path $env:RUNNER_TEMP 'umbra-app-smoke.stderr.log'
        $smokeProcess = Start-Process -FilePath (Join-Path $installDir 'Umbra.exe') -PassThru -WindowStyle Hidden -RedirectStandardError $stderr
        $null = $smokeProcess.Handle
        Start-Sleep -Seconds 3
        $startupLog = Get-Content -LiteralPath $stderr -Raw
        if ($smokeProcess.HasExited) { throw "Installed application exited during startup.`n$startupLog" }
        if ($startupLog -match '(?m)^Umbra:') { throw "Installed application reported a startup error.`n$startupLog" }
        if (-not (Test-Path -LiteralPath (Join-Path $env:UMBRA_DATA_DIR 'bin/sing-box.exe'))) { throw 'Installed application did not discover its bundled core.' }
    } finally {
        if ($smokeProcess -and -not $smokeProcess.HasExited) { Stop-Process -Id $smokeProcess.Id; $null = $smokeProcess.WaitForExit(10000) }
        $env:UMBRA_DATA_DIR = $savedData
    }
    $uninstaller = Start-Process -FilePath (Join-Path $installDir 'Uninstall.exe') -ArgumentList '/S' -PassThru -WindowStyle Hidden
    $null = $uninstaller.Handle
    if (-not $uninstaller.WaitForExit(30000)) { throw 'Uninstaller timed out.' }
    for ($attempt = 0; $attempt -lt 40 -and (Test-Path -LiteralPath (Join-Path $installDir 'Umbra.exe')); $attempt++) { Start-Sleep -Milliseconds 250 }
    if (Test-Path -LiteralPath (Join-Path $installDir 'Umbra.exe')) { throw 'Uninstaller left the application installed.' }
    if (-not (Test-Path -LiteralPath (Join-Path $env:RUNNER_TEMP 'UmbraAppSmokeData/settings.json'))) { throw 'Uninstaller removed application data.' }
    if ((Get-Content -LiteralPath $sentinel -Raw).Trim() -ne 'preserve user data') { throw 'Uninstaller changed the default application data directory.' }
    Remove-Item -LiteralPath $sentinel
    Write-Output 'Silent install, application startup, bundled core, uninstall, and data preservation verified.'
} finally { Pop-Location }
