param([string]$MakeNsis = "${env:ProgramFiles(x86)}\NSIS\makensis.exe")
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
Push-Location -LiteralPath $taskRoot
try {
    $metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
    $version = ($metadata.packages | Where-Object name -eq 'umbra').version
    if (-not $version) { throw 'Umbra package version was not found' }
    & (Join-Path $PSScriptRoot 'dependency-notices.ps1')
    $releaseDir = New-Item -ItemType Directory -Force -Path (Join-Path $taskRoot 'release')
    $stagingDir = Join-Path ([IO.Path]::GetTempPath()) ('Umbra-package-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path (Join-Path $stagingDir 'resources') -Force | Out-Null
    Copy-Item -LiteralPath 'target/release/umbra.exe' -Destination (Join-Path $stagingDir 'Umbra.exe')
    Copy-Item -LiteralPath 'src-tauri/resources/sing-box.exe' -Destination (Join-Path $stagingDir 'resources/sing-box.exe')
    Copy-Item -LiteralPath 'LICENSE' -Destination (Join-Path $stagingDir 'LICENSE.txt')
    Copy-Item -LiteralPath 'THIRD_PARTY_NOTICES.md' -Destination $stagingDir
    Copy-Item -LiteralPath 'licenses' -Destination $stagingDir -Recurse
    $portablePath = Join-Path $releaseDir.FullName "Umbra_${version}_windows_x64_portable.zip"
    Compress-Archive -Path (Join-Path $stagingDir '*') -DestinationPath $portablePath -Force
    if (-not (Test-Path -LiteralPath $MakeNsis)) { throw "NSIS compiler not found: $MakeNsis" }
    & $MakeNsis "/DUMBRA_VERSION=$version" (Join-Path $taskRoot 'installer/umbra.nsi')
    if ($LASTEXITCODE -ne 0) { throw 'NSIS build failed' }
    $installerPath = Join-Path $releaseDir.FullName "Umbra_${version}_x64-setup.exe"
    Copy-Item -LiteralPath 'THIRD_PARTY_NOTICES.md' -Destination $releaseDir.FullName
    @($portablePath, $installerPath, (Join-Path $releaseDir.FullName 'THIRD_PARTY_NOTICES.md')) | ForEach-Object {
        '{0}  {1}' -f (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant(), [IO.Path]::GetFileName($_)
    } | Set-Content -Encoding ascii -LiteralPath (Join-Path $releaseDir.FullName 'SHA256SUMS.txt')
    Write-Output "Release files: $($releaseDir.FullName)"
} finally {
    Pop-Location
}
