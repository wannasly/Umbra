$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
Push-Location -LiteralPath $taskRoot
try {
    $metadata = cargo metadata --locked --filter-platform x86_64-pc-windows-msvc --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve dependency licenses' }
    $nodes = @{}
    foreach ($node in $metadata.resolve.nodes) { $nodes[$node.id] = $node }
    $seen = [Collections.Generic.HashSet[string]]::new()
    $queue = [Collections.Generic.Queue[string]]::new()
    $queue.Enqueue(($metadata.packages | Where-Object name -eq umbra).id)
    while ($queue.Count) {
        $id = $queue.Dequeue()
        if (-not $seen.Add($id)) { continue }
        foreach ($dependency in $nodes[$id].deps) {
            if (@($dependency.dep_kinds | Where-Object { -not $_.kind }).Count) { $queue.Enqueue($dependency.pkg) }
        }
    }
    $notice = [Text.StringBuilder]::new()
    [void]$notice.AppendLine('Rust dependencies for the Windows build; versions are locked in Cargo.lock.')
    [void]$notice.AppendLine('Slint is used under the Slint Royalty-free 2.0 license included separately. Font, icon and sing-box notices are also included separately.')
    foreach ($package in ($metadata.packages | Where-Object { $_.source -and $seen.Contains($_.id) } | Sort-Object name,version)) {
        [void]$notice.AppendLine("`n============================================================")
        [void]$notice.AppendLine("$($package.name) $($package.version)")
        [void]$notice.AppendLine("Declared license: $($package.license)")
        [void]$notice.AppendLine("Source: $($package.repository)")
        $crateDir = Split-Path -Parent $package.manifest_path
        foreach ($license in (Get-ChildItem -LiteralPath $crateDir -File | Where-Object Name -Match '^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT)' | Sort-Object Name)) {
            [void]$notice.AppendLine("`n--- $($license.Name) ---")
            [void]$notice.AppendLine([IO.File]::ReadAllText($license.FullName))
        }
    }
    [IO.File]::WriteAllText((Join-Path $taskRoot 'licenses/Rust-dependencies.txt'),$notice.ToString(),[Text.UTF8Encoding]::new($false))
} finally { Pop-Location }
