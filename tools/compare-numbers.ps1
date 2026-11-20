# Copyright © Kentaro Inomata
# SPDX-License-Identifier: MIT-0
#
# Compares the fork with upstream Slint 1.19.0 from outside the process: startup time, memory and
# CPU while idle, for the same app built on each. The apps are upstream's gallery (its first page
# animates an indeterminate ProgressIndicator and Spinner, so its "idle" draws every frame) and
# upstream's custom-titlebar (static). Upstream runs as an application gets it by default
# (winit + FemtoVG on OpenGL) and with the software renderer (SLINT_BACKEND=winit-software).
#
# Per run: start the exe (without a console; the probes are console programs), time until it
# has a visible top-level window and until that window answers a WM_NULL (its thread pumps
# messages again), wait -Settle seconds, read memory, then read the CPU time used over -Idle
# seconds. The first run of each is a warm-up and isn't counted.
#
# Build first (release, as a user ships it):
#   cargo build --release -p slint-fork-probe --bin probe-gallery --bin probe-titlebar
#   cd tools/upstream-gallery; cargo build --release --bin upstream-gallery --bin upstream-titlebar
# Usage: tools/compare-numbers.ps1 [-Runs 5] [-Settle 3] [-Idle 10] [-Apps gallery,titlebar]
# Windows open in the foreground while it runs; leave the desktop alone.
param(
    [int] $Runs = 5,
    [double] $Settle = 3,
    [double] $Idle = 10,
    [ValidateSet('gallery', 'titlebar')] [string[]] $Apps = @('gallery', 'titlebar'),
    [string] $Out
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$target = (cargo metadata --format-version 1 --no-deps --manifest-path (Join-Path $repo 'Cargo.toml') |
    ConvertFrom-Json).target_directory
$release = Join-Path $target 'release'

Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
public static class NumbersProbe {
    delegate bool EnumProc(IntPtr hwnd, IntPtr lParam);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr lParam);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr hwnd, uint cmd);
    [DllImport("user32.dll")] static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint msg, IntPtr w, IntPtr l, uint flags, uint timeoutMs, out IntPtr result);
    static IntPtr VisibleWindow(int pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            // GW_OWNER: top-level windows without an owner, as the main window is.
            if (p == pid && IsWindowVisible(h) && GetWindow(h, 4) == IntPtr.Zero) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    // Milliseconds from `start` until `pid` shows a window, then until that window answers a
    // WM_NULL; -1 on a timeout.
    public static double[] WaitReady(int pid, Stopwatch start, int timeoutMs) {
        while (start.ElapsedMilliseconds < timeoutMs) {
            IntPtr h = VisibleWindow(pid);
            if (h != IntPtr.Zero) {
                double visible = start.Elapsed.TotalMilliseconds;
                IntPtr r;
                // SMTO_NORMAL
                if (SendMessageTimeout(h, 0, IntPtr.Zero, IntPtr.Zero, 0, (uint)timeoutMs, out r) == IntPtr.Zero) return new[] { visible, -1.0 };
                return new[] { visible, start.Elapsed.TotalMilliseconds };
            }
            System.Threading.Thread.Sleep(1);
        }
        return new[] { -1.0, -1.0 };
    }
}
'@

$sides = @(
    @{ Side = 'fork'; Prefix = 'probe'; Env = @{} },
    @{ Side = 'upstream'; Prefix = 'upstream'; Env = @{} },
    @{ Side = 'upstream-sw'; Prefix = 'upstream'; Env = @{ SLINT_BACKEND = 'winit-software' } }
)

function Measure-Once($exe, $envVars) {
    $saved = @{}
    foreach ($k in $envVars.Keys) { $saved[$k] = [Environment]::GetEnvironmentVariable($k); [Environment]::SetEnvironmentVariable($k, $envVars[$k]) }
    try {
        $info = [Diagnostics.ProcessStartInfo]::new($exe)
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $true
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $p = [Diagnostics.Process]::Start($info)
    } finally {
        foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k]) }
    }
    try {
        $visible, $answered = [NumbersProbe]::WaitReady($p.Id, $clock, 20000)
        if ($answered -lt 0) { throw "$exe showed no window, or it didn't answer, in 20 s" }
        Start-Sleep -Milliseconds ([int]($Settle * 1000))
        $p.Refresh()
        $ws = $p.WorkingSet64; $private = $p.PrivateMemorySize64; $threads = $p.Threads.Count
        $gpu = try {
            $c = Get-Counter "\GPU Process Memory(pid_$($p.Id)_*)\Dedicated Usage", "\GPU Process Memory(pid_$($p.Id)_*)\Shared Usage" -ErrorAction Stop
            $d = ($c.CounterSamples | Where-Object Path -like '*dedicated usage' | Measure-Object CookedValue -Sum).Sum
            $s = ($c.CounterSamples | Where-Object Path -like '*shared usage' | Measure-Object CookedValue -Sum).Sum
            @($d, $s)
        } catch { @($null, $null) }
        $p.Refresh(); $cpu0 = $p.TotalProcessorTime; $t0 = $clock.Elapsed
        Start-Sleep -Milliseconds ([int]($Idle * 1000))
        $p.Refresh(); $cpu1 = $p.TotalProcessorTime; $t1 = $clock.Elapsed
        [pscustomobject]@{
            VisibleMs = [math]::Round($visible, 1)
            AnsweredMs = [math]::Round($answered, 1)
            WorkingSetMB = [math]::Round($ws / 1MB, 1)
            PrivateMB = [math]::Round($private / 1MB, 1)
            GpuDedicatedMB = if ($null -ne $gpu[0]) { [math]::Round($gpu[0] / 1MB, 1) }
            GpuSharedMB = if ($null -ne $gpu[1]) { [math]::Round($gpu[1] / 1MB, 1) }
            Threads = $threads
            # Percent of one core.
            IdleCpuPct = [math]::Round(100 * ($cpu1 - $cpu0).TotalMilliseconds / ($t1 - $t0).TotalMilliseconds, 2)
        }
    } finally {
        if (-not $p.HasExited) { $p.Kill(); $p.WaitForExit(5000) | Out-Null }
    }
}

$rows = foreach ($app in $Apps) {
    foreach ($s in $sides) {
        $exe = Join-Path $release "$($s.Prefix)-$app.exe"
        if (-not (Test-Path $exe)) { throw "$exe isn't built (see the build commands at the top)" }
        Write-Host "$app / $($s.Side): warm-up + $Runs runs"
        $null = Measure-Once $exe $s.Env
        for ($i = 1; $i -le $Runs; $i++) {
            $r = Measure-Once $exe $s.Env
            $r | Add-Member -NotePropertyName App -NotePropertyValue $app -PassThru |
                Add-Member -NotePropertyName Side -NotePropertyValue $s.Side -PassThru |
                Add-Member -NotePropertyName Run -NotePropertyValue $i -PassThru
        }
    }
}

if (-not $Out) { $Out = Join-Path $repo 'manual-checks/numbers' }
New-Item -ItemType Directory -Force $Out | Out-Null
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$rows | Select-Object App, Side, Run, VisibleMs, AnsweredMs, WorkingSetMB, PrivateMB, GpuDedicatedMB, GpuSharedMB, Threads, IdleCpuPct |
    Export-Csv (Join-Path $Out "runs-$stamp.csv")

function Get-Median($xs) {
    $v = @($xs | Where-Object { $null -ne $_ } | Sort-Object)
    if ($v.Count -eq 0) { return $null }
    if ($v.Count % 2) { $v[[int][math]::Floor($v.Count / 2)] } else { ($v[$v.Count / 2 - 1] + $v[$v.Count / 2]) / 2 }
}
$fields = 'VisibleMs', 'AnsweredMs', 'WorkingSetMB', 'PrivateMB', 'GpuDedicatedMB', 'GpuSharedMB', 'Threads', 'IdleCpuPct'
$summary = $rows | Group-Object App, Side | ForEach-Object {
    $o = [ordered]@{ App = $_.Group[0].App; Side = $_.Group[0].Side }
    foreach ($f in $fields) { $o[$f] = Get-Median ($_.Group | ForEach-Object $f) }
    [pscustomobject]$o
}
$summary | Export-Csv (Join-Path $Out "median-$stamp.csv")
Write-Host "Medians of $Runs runs (runs-$stamp.csv, median-$stamp.csv in $Out):"
$summary | Format-Table -AutoSize | Out-String -Width 200 | Write-Host
