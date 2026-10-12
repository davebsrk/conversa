<#
.SYNOPSIS
  Mide RAM y CPU de una app y de todo su árbol de procesos (incluidos los hijos de WebView2).

.DESCRIPTION
  Cada -Interval segundos recalcula el árbol de procesos (los procesos de WebView2 aparecen y
  desaparecen) y suma:
    - PrivateWS : working set privado (la columna "Memoria" del Administrador de tareas)
    - WS        : working set total (incluye páginas compartidas: sobreestima)
    - Commit    : bytes privados confirmados
  El uso de CPU se calcula a partir de los deltas de TotalProcessorTime y se expresa como % de
  la CPU total de la máquina (igual que el Administrador de tareas).

.EXAMPLE
  ./tools/measure.ps1 -Name WhatsApp.Root -Duration 120 -Label "oficial-activa"
.EXAMPLE
  ./tools/measure.ps1 -Name conversa -Duration 300 -Label "conversa-bandeja" -OutCsv results.csv
#>
param(
    [string]$Name,
    [int]$Id,
    [int]$Duration = 60,
    [int]$Interval = 2,
    [string]$Label = "",
    [string]$OutCsv
)

$ErrorActionPreference = 'Stop'

function Get-RootId {
    if ($Id) { return $Id }
    if (-not $Name) { throw "Usa -Name o -Id" }
    $n = $Name -replace '\.exe$', ''
    $procs = @(Get-Process -Name $n -ErrorAction SilentlyContinue)
    if ($procs.Count -eq 0) { throw "No hay ningún proceso llamado '$n'" }
    # Si hay varios, la raíz es el que no tiene como padre otro proceso con el mismo nombre.
    $ids = $procs.Id
    $cim = Get-CimInstance Win32_Process -Filter "Name='$n.exe'"
    $root = $cim | Where-Object { $ids -notcontains $_.ParentProcessId } | Select-Object -First 1
    return [int]$root.ProcessId
}

function Get-TreeIds([int]$rootId) {
    $all = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId
    $children = @{}
    foreach ($p in $all) {
        $pp = [int]$p.ParentProcessId
        if (-not $children.ContainsKey($pp)) { $children[$pp] = New-Object System.Collections.Generic.List[int] }
        $children[$pp].Add([int]$p.ProcessId)
    }
    $result = New-Object System.Collections.Generic.List[int]
    $stack = New-Object System.Collections.Generic.Stack[int]
    $stack.Push($rootId)
    while ($stack.Count -gt 0) {
        $cur = $stack.Pop()
        $result.Add($cur)
        if ($children.ContainsKey($cur)) { foreach ($c in $children[$cur]) { $stack.Push($c) } }
    }
    return $result
}

function Get-Sample([int]$rootId) {
    $ids = Get-TreeIds $rootId
    $perf = Get-CimInstance Win32_PerfRawData_PerfProc_Process -Property IDProcess, WorkingSetPrivate, WorkingSet, PrivateBytes |
        Where-Object { $ids -contains [int]$_.IDProcess }
    $cpu = 0.0
    foreach ($i in $ids) {
        $p = Get-Process -Id $i -ErrorAction SilentlyContinue
        if ($p) { $cpu += $p.TotalProcessorTime.TotalSeconds }
    }
    [pscustomobject]@{
        Time      = Get-Date
        Procs     = $ids.Count
        PrivateWS = ($perf | Measure-Object WorkingSetPrivate -Sum).Sum
        WS        = ($perf | Measure-Object WorkingSet -Sum).Sum
        Commit    = ($perf | Measure-Object PrivateBytes -Sum).Sum
        CpuSec    = $cpu
    }
}

$rootId = Get-RootId
$cores = [Environment]::ProcessorCount
$rootName = (Get-Process -Id $rootId).ProcessName
Write-Host ("Midiendo {0} [{1}] durante {2}s (cada {3}s)..." -f $rootName, $rootId, $Duration, $Interval)

$samples = New-Object System.Collections.Generic.List[object]
$prev = Get-Sample $rootId
$end = (Get-Date).AddSeconds($Duration)
while ((Get-Date) -lt $end) {
    Start-Sleep -Seconds $Interval
    $s = Get-Sample $rootId
    $wall = ($s.Time - $prev.Time).TotalSeconds
    # Si muere un proceso hijo su tiempo de CPU desaparece de la suma y el delta podría ser negativo.
    $cpuPct = [Math]::Max(0, ($s.CpuSec - $prev.CpuSec) / $wall / $cores * 100)
    $samples.Add([pscustomobject]@{
        Time        = $s.Time.ToString('s')
        Procs       = $s.Procs
        PrivateWSMB = [Math]::Round($s.PrivateWS / 1MB, 1)
        WSMB        = [Math]::Round($s.WS / 1MB, 1)
        CommitMB    = [Math]::Round($s.Commit / 1MB, 1)
        CpuPct      = [Math]::Round($cpuPct, 2)
    })
    $prev = $s
}

function Stat($prop) {
    $m = $samples | Measure-Object $prop -Average -Maximum -Minimum
    "{0,8:N1} {1,8:N1} {2,8:N1}" -f $m.Average, $m.Minimum, $m.Maximum
}

Write-Host ""
Write-Host ("Resultado {0}  ({1} muestras, {2} procesos, {3} núcleos)" -f $Label, $samples.Count, $samples[-1].Procs, $cores)
Write-Host ("{0,-16} {1,8} {2,8} {3,8}" -f "", "media", "mín", "máx")
Write-Host ("{0,-16} {1}" -f "Memoria (MB)", (Stat PrivateWSMB))
Write-Host ("{0,-16} {1}" -f "Working set (MB)", (Stat WSMB))
Write-Host ("{0,-16} {1}" -f "Commit (MB)", (Stat CommitMB))
Write-Host ("{0,-16} {1}" -f "CPU (%)", (Stat CpuPct))

if ($OutCsv) {
    $summary = [pscustomobject]@{
        Label        = $Label
        App          = $rootName
        Date         = (Get-Date).ToString('s')
        DurationSec  = $Duration
        Procs        = $samples[-1].Procs
        MemAvgMB     = [Math]::Round(($samples | Measure-Object PrivateWSMB -Average).Average, 1)
        MemMaxMB     = ($samples | Measure-Object PrivateWSMB -Maximum).Maximum
        CommitAvgMB  = [Math]::Round(($samples | Measure-Object CommitMB -Average).Average, 1)
        CpuAvgPct    = [Math]::Round(($samples | Measure-Object CpuPct -Average).Average, 2)
    }
    $summary | Export-Csv -Path $OutCsv -Append -NoTypeInformation -Encoding utf8
    Write-Host "Resumen añadido a $OutCsv"
}
