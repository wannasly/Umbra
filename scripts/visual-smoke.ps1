param([switch]$NoBuild, [switch]$DebugBuild, [string]$Size = '1280x720', [ValidateSet('dashboard','servers','subscriptions','routing','logs','settings')][string[]]$Pages = @('dashboard','servers','subscriptions','routing','logs','settings'), [ValidateSet('','import','subscription','rule','edit-rule','rename','about')][string]$Action = '', [switch]$Lifecycle)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
Push-Location -LiteralPath $taskRoot
$savedEnvironment = @{}
foreach ($key in @('UMBRA_DATA_DIR','UMBRA_SMOKE_CAPTURE','UMBRA_SMOKE_PAGE','UMBRA_SMOKE_SIZE','UMBRA_SMOKE_ACTION','UMBRA_SMOKE_LIFECYCLE','SLINT_SCALE_FACTOR')) {
    $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
}
try {
    if (-not $NoBuild) {
        cargo build --release --features smoke-test --locked
        if ($LASTEXITCODE -ne 0) { throw 'Native build failed' }
    }
    $outputDir = Join-Path $taskRoot "artifacts/native/slint-$Size"
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
        $captureName = $page + $(if ($Action) { "-$Action" }) + $(if ($Lifecycle) { '-lifecycle' })
        $env:UMBRA_SMOKE_CAPTURE = Join-Path $outputDir "$captureName.png"
        $env:UMBRA_SMOKE_PAGE = $page
        $env:UMBRA_SMOKE_SIZE = $Size
        $env:SLINT_SCALE_FACTOR = '1'
        if ($Action) { $env:UMBRA_SMOKE_ACTION = $Action } else { Remove-Item Env:UMBRA_SMOKE_ACTION -ErrorAction SilentlyContinue }
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
