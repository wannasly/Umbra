param(
    [switch]$NoBuild,
    [ValidateSet('dashboard','pages','resize')][string]$Scene = 'dashboard',
    [ValidateRange(4,90)][int]$Seconds = 12,
    [string]$Size = '1100x720',
    [ValidateRange(0.75,3)][double]$Scale = 1
)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
Push-Location -LiteralPath $taskRoot
$savedEnvironment = @{}
foreach ($key in @('UMBRA_DATA_DIR','UMBRA_BENCH_SIZE','UMBRA_BENCH_SCENE','UMBRA_BENCH_SECONDS','UMBRA_SMOKE_CAPTURE','UMBRA_SMOKE_STATUS','SLINT_SCALE_FACTOR')) {
    $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
}
try {
    if (-not $NoBuild) {
        cargo build --release --features smoke-test --locked
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    }
    $outputDir = Join-Path $taskRoot ("artifacts/native/motion/$Scene-$Size-scale$Scale-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path (Join-Path $outputDir 'bin') | Out-Null
    $fixtureSettings = Get-Content -LiteralPath tests/fixtures/visual/settings.fixture.json -Raw | ConvertFrom-Json
    $fixtureSettings.reduceMotion = $false
    [System.IO.File]::WriteAllText((Join-Path $outputDir 'settings.json'), ($fixtureSettings | ConvertTo-Json -Depth 30), [System.Text.UTF8Encoding]::new($false))
    Copy-Item -LiteralPath tests/fixtures/visual/profiles.fixture.json -Destination (Join-Path $outputDir 'profiles.json')
    New-Item -ItemType HardLink -Path (Join-Path $outputDir 'bin/sing-box.exe') -Target (Join-Path $taskRoot 'src-tauri/resources/sing-box.exe') | Out-Null
    $env:UMBRA_DATA_DIR = $outputDir
    $env:UMBRA_BENCH_SIZE = $Size
    $env:UMBRA_BENCH_SCENE = $Scene
    $env:UMBRA_BENCH_SECONDS = "$Seconds"
    $env:UMBRA_SMOKE_CAPTURE = Join-Path $outputDir 'unused.png'
    $env:UMBRA_SMOKE_STATUS = 'connected'
    $env:SLINT_SCALE_FACTOR = $Scale.ToString([Globalization.CultureInfo]::InvariantCulture)
    $taskProcess = Start-Process -FilePath (Join-Path $taskRoot 'target/release/umbra.exe') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $outputDir 'stderr.log')
    $null = $taskProcess.Handle
    if (-not $taskProcess.WaitForExit(($Seconds + 10)*1000)) {
        $taskProcess.Kill()
        throw 'Motion benchmark timed out'
    }
    if ($taskProcess.ExitCode -ne 0) { throw "Motion benchmark failed: $outputDir" }
    $report = Get-Content -LiteralPath (Join-Path $outputDir 'frame-times.json') -Raw | ConvertFrom-Json
    $report | Select-Object scene,build,frames,fps,median_ms,p95_ms,p99_ms,over_25ms
    Write-Output "Report: $(Join-Path $outputDir 'frame-times.json')"
    Remove-Item -LiteralPath (Join-Path $outputDir 'bin/sing-box.exe')
} finally {
    foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key], 'Process') }
    Pop-Location
}
