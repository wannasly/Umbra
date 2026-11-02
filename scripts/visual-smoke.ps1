param([switch]$NoBuild, [switch]$DebugBuild, [string]$Size = '1280x720', [ValidateSet('dashboard','servers','subscriptions','routing','logs','settings')][string[]]$Pages = @('dashboard','servers','subscriptions','routing','logs','settings'), [ValidateSet('','import','subscription','rule','edit-rule','rename','about')][string]$Action = '', [switch]$Lifecycle, [ValidateSet('','disconnected','connecting','connected')][string]$Status = '', [switch]$Traffic, [switch]$Scroll, [ValidateRange(0.75,3)][double]$Scale = 1)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
Push-Location -LiteralPath $taskRoot
$savedEnvironment = @{}
foreach ($key in @('UMBRA_DATA_DIR','UMBRA_SMOKE_CAPTURE','UMBRA_SMOKE_PAGE','UMBRA_SMOKE_SIZE','UMBRA_SMOKE_ACTION','UMBRA_SMOKE_LIFECYCLE','UMBRA_SMOKE_STATUS','UMBRA_SMOKE_TRAFFIC','UMBRA_SMOKE_SCROLL','SLINT_SCALE_FACTOR')) {
    $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
}
try {
    if (-not $NoBuild) {
        cargo build --release --features smoke-test --locked
        if ($LASTEXITCODE -ne 0) { throw 'Native build failed' }
    }
    $outputDir = Join-Path $taskRoot ("artifacts/native/slint-$Size" + $(if ($Scale -ne 1) { "-scale$Scale" }))
    New-Item -ItemType Directory -Force -Path $outputDir | Out-Null
    foreach ($page in $Pages) {
        $dataDir = Join-Path $outputDir ("data-$page-" + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $dataDir | Out-Null
        Copy-Item -LiteralPath 'tests/fixtures/visual/settings.fixture.json' -Destination (Join-Path $dataDir 'settings.json')
        Copy-Item -LiteralPath 'tests/fixtures/visual/profiles.fixture.json' -Destination (Join-Path $dataDir 'profiles.json')
        # Each isolated capture needs the core version, not another 43 MiB copy.
        New-Item -ItemType Directory -Path (Join-Path $dataDir 'bin') | Out-Null
        New-Item -ItemType HardLink -Path (Join-Path $dataDir 'bin/sing-box.exe') -Target (Join-Path $taskRoot 'src-tauri/resources/sing-box.exe') | Out-Null
        $env:UMBRA_DATA_DIR = $dataDir
        $captureName = $page + $(if ($Traffic) { "-traffic" }) + $(if ($Scroll) { "-scrolled" }) + $(if ($Status) { "-$Status" }) + $(if ($Action) { "-$Action" }) + $(if ($Lifecycle) { '-lifecycle' })
        $env:UMBRA_SMOKE_CAPTURE = Join-Path $outputDir "$captureName.png"
        $env:UMBRA_SMOKE_PAGE = $page
        $env:UMBRA_SMOKE_SIZE = $Size
        $env:SLINT_SCALE_FACTOR = $Scale.ToString([Globalization.CultureInfo]::InvariantCulture)
        if ($Action) { $env:UMBRA_SMOKE_ACTION = $Action } else { Remove-Item Env:UMBRA_SMOKE_ACTION -ErrorAction SilentlyContinue }
        if ($Traffic) { $env:UMBRA_SMOKE_TRAFFIC='1' } else { Remove-Item Env:UMBRA_SMOKE_TRAFFIC -ErrorAction SilentlyContinue }
        if ($Scroll) { $env:UMBRA_SMOKE_SCROLL='1' } else { Remove-Item Env:UMBRA_SMOKE_SCROLL -ErrorAction SilentlyContinue }
        if ($Status) { $env:UMBRA_SMOKE_STATUS = $Status } else { Remove-Item Env:UMBRA_SMOKE_STATUS -ErrorAction SilentlyContinue }
        if ($Lifecycle) { $env:UMBRA_SMOKE_LIFECYCLE = '1' } else { Remove-Item Env:UMBRA_SMOKE_LIFECYCLE -ErrorAction SilentlyContinue }
        $binaryPath = if ($DebugBuild) { 'target/debug/umbra.exe' } else { 'target/release/umbra.exe' }
        $taskCaptureProcess = Start-Process -FilePath (Join-Path $taskRoot $binaryPath) -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $outputDir "$captureName.stderr.log")
        # Keep the process handle open so Windows PowerShell can read the exit code after exit.
        $null = $taskCaptureProcess.Handle
        if ($Lifecycle) {
            Start-Sleep -Milliseconds 350
            $taskSecondProcess = Start-Process -FilePath (Join-Path $taskRoot $binaryPath) -WindowStyle Hidden -PassThru
            $null = $taskSecondProcess.Handle
            if (-not $taskSecondProcess.WaitForExit(5000) -or $taskSecondProcess.ExitCode -ne 0) { throw 'Second launch failed' }
        }
        if (-not $taskCaptureProcess.WaitForExit(20000)) {
            $taskCaptureProcess.Kill()
            throw "Capture timed out: $page"
        }
        if ($taskCaptureProcess.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $env:UMBRA_SMOKE_CAPTURE)) { throw "Capture failed: $page (exit $($taskCaptureProcess.ExitCode)); see $captureName.stderr.log" }
        Remove-Item -LiteralPath (Join-Path $dataDir 'bin/sing-box.exe')
        Write-Output "$page : $env:UMBRA_SMOKE_CAPTURE"
    }
} finally {
    foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key], 'Process') }
    Pop-Location
}
