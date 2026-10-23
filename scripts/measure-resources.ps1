param([Parameter(Mandatory)][int]$RootProcessId, [int]$Seconds = 30, [string]$Output = 'artifacts/native/resources.json')
$ErrorActionPreference = 'Stop'
$allProcesses = Get-CimInstance Win32_Process
$processIds = [Collections.Generic.HashSet[int]]::new()
[void]$processIds.Add($RootProcessId)
do {
    $changed = $false
    foreach ($process in $allProcesses) {
        if ($processIds.Contains([int]$process.ParentProcessId) -and $processIds.Add([int]$process.ProcessId)) { $changed = $true }
    }
} while ($changed)
$cpuStart = @{}
foreach ($processId in $processIds) {
    $process = Get-Process -Id $processId -ErrorAction SilentlyContinue
    if ($process) { $cpuStart[$processId] = $process.TotalProcessorTime.TotalSeconds }
}
$samples = [Collections.Generic.List[object]]::new()
$stopwatch = [Diagnostics.Stopwatch]::StartNew()
while ($stopwatch.Elapsed.TotalSeconds -lt $Seconds) {
    $memory = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | Where-Object { $processIds.Contains([int]$_.IDProcess) })
    $gpu = @(Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '^pid_(\d+)_' -and $processIds.Contains([int]$Matches[1]) })
    $gpuMemory = @(Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUProcessMemory -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '^pid_(\d+)_' -and $processIds.Contains([int]$Matches[1]) })
    $samples.Add([pscustomobject]@{
        elapsed = [math]::Round($stopwatch.Elapsed.TotalSeconds, 2)
        privateWorkingSetMiB = [math]::Round(($memory | Measure-Object WorkingSetPrivate -Sum).Sum / 1MB, 2)
        workingSetMiB = [math]::Round(($memory | Measure-Object WorkingSet -Sum).Sum / 1MB, 2)
        privateCommitMiB = [math]::Round(($memory | Measure-Object PrivateBytes -Sum).Sum / 1MB, 2)
        gpuEnginePercent = ($gpu | Measure-Object UtilizationPercentage -Sum).Sum
        gpuDedicatedMiB = [math]::Round(($gpuMemory | Measure-Object DedicatedUsage -Sum).Sum / 1MB, 2)
        gpuSharedMiB = [math]::Round(($gpuMemory | Measure-Object SharedUsage -Sum).Sum / 1MB, 2)
    })
    Start-Sleep -Milliseconds 800
}
$cpuSeconds = 0.0
foreach ($processId in $cpuStart.Keys) {
    $process = Get-Process -Id $processId -ErrorAction SilentlyContinue
    if ($process) { $cpuSeconds += $process.TotalProcessorTime.TotalSeconds - $cpuStart[$processId] }
}
$result = [pscustomobject]@{
    rootProcessId = $RootProcessId
    processIds = @($processIds)
    durationSeconds = [math]::Round($stopwatch.Elapsed.TotalSeconds,2)
    cpuPercentOfOneCore = [math]::Round($cpuSeconds / $stopwatch.Elapsed.TotalSeconds * 100, 3)
    cpuPercentOfMachine = [math]::Round($cpuSeconds / $stopwatch.Elapsed.TotalSeconds / [Environment]::ProcessorCount * 100, 3)
    samples = $samples
}
$taskOutputPath = [IO.Path]::GetFullPath($Output)
New-Item -ItemType Directory -Force -Path (Split-Path $taskOutputPath -Parent) | Out-Null
$result | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 -LiteralPath $taskOutputPath
$result | Select-Object rootProcessId,durationSeconds,cpuPercentOfOneCore,cpuPercentOfMachine
Write-Output "Measurements: $taskOutputPath"
