# Copyright © Kentaro Inomata
# SPDX-License-Identifier: MIT-0
#
# Builds upstream's Rust examples and demos on the fork, with their sources unmodified, as an
# application written for upstream Slint would build after following the README. Then it starts
# every binary and captures its window, so that each app can be sorted into: works, doesn't work
# because of an API the fork removed, or a bug.
#
# Targets are directories relative to reference/slint-1.19.0 (a bare name means examples/<name>).
# Upstream's examples/, demos/, logo/ and internal/common/sharedfontique (a font that a demo
# imports) are mirrored under <target dir>/upstream-examples/<form>/, so that paths between them
# (`../../logo`, `../ui`) keep working, and a target's top-level folder is copied afresh before
# its build. Only its Cargo.toml changes, from upstream's workspace paths and inheritance to what
# an application outside the repository writes:
#   -Form patch: `slint = "1.19"` from crates.io, replaced by `[patch.crates-io]`.
#   -Form git:   `slint` from the fork directly.
# The fork is this clone's crates/slint, or with -Git <tag>, that tag on GitHub, as the README
# writes it. The script then checks in Cargo.lock that `slint` is the fork, not crates.io's.
# A target with a path dependency outside its own folder is skipped. Below -MinFreeGB of free
# space, building stops and the rest is skipped.
#
# Unless -NoRun, each binary then starts in its target's directory, its first visible window moves
# to the top right of the primary screen's working area, and only that window's rectangle is
# captured (nothing is clicked or typed); then the app is closed. Screenshots are in
# <work>/shots, one row per binary in <work>/results.tsv.
#
# Usage: tools/upstream-examples.ps1 [-Form patch,git] [-Examples examples/memory,demos/launcher]
#        [-Git <tag>] [-NoRun] [-WaitSeconds 15] [-SettleMs 2500] [-MinFreeGB 30]
# Run from PowerShell, where cargo finds the MSVC linker. Exits 1 if a target failed to build or
# an app showed no window.
param(
    [ValidateSet('patch', 'git')] [string[]] $Form = @('patch'),
    [string[]] $Examples = @(
        'examples/7guis', 'examples/async-io', 'examples/carousel/rust', 'examples/dnd-kanban',
        'examples/ffmpeg', 'examples/gallery', 'examples/gstreamer-player', 'examples/imagefilter/rust',
        'examples/maps', 'examples/memory', 'examples/native-gestures', 'examples/opengl_texture',
        'examples/opengl_underlay', 'examples/plotter', 'examples/runtime_key_bindings/rust',
        'examples/slide_puzzle', 'examples/speedometer/rust', 'examples/system-tray',
        'examples/todo/rust', 'examples/todo-mvc/rust', 'examples/virtual_keyboard/rust',
        'examples/wgpu_texture', 'demos/energy-monitor', 'demos/food-processor/rust',
        'demos/home-automation/rust', 'demos/launcher', 'demos/printerdemo/rust',
        'demos/usecases/rust', 'demos/weather-demo'
    ),
    [string] $Git,
    [switch] $Run = $true,
    [switch] $NoRun,
    [int] $WaitSeconds = 15,
    [int] $SettleMs = 2500,
    [double] $MinFreeGB = 30
)
$ErrorActionPreference = 'Stop'

# --- Cargo.toml: upstream's workspace to a standalone manifest -------------------------------

# Splits `a = 1, b = [2, 3]` at the commas outside quotes, brackets and braces.
function Split-TomlItems([string] $s) {
    $items = [System.Collections.Generic.List[string]]::new()
    $depth = 0; $quote = $null; $start = 0
    for ($i = 0; $i -lt $s.Length; $i++) {
        $c = $s[$i]
        if ($quote) {
            if ($c -eq '\' -and $quote -eq '"') { $i++ } elseif ($c -eq $quote) { $quote = $null }
        } elseif ($c -eq '"' -or $c -eq "'") { $quote = $c }
        elseif ($c -eq '[' -or $c -eq '{') { $depth++ }
        elseif ($c -eq ']' -or $c -eq '}') { $depth-- }
        elseif ($c -eq ',' -and $depth -eq 0) { $items.Add($s.Substring($start, $i - $start).Trim()); $start = $i + 1 }
    }
    $rest = $s.Substring($start).Trim()
    if ($rest) { $items.Add($rest) }
    $items
}

function Remove-TomlComment([string] $s) {
    $quote = $null
    for ($i = 0; $i -lt $s.Length; $i++) {
        $c = $s[$i]
        if ($quote) {
            if ($c -eq '\' -and $quote -eq '"') { $i++ } elseif ($c -eq $quote) { $quote = $null }
        } elseif ($c -eq '"' -or $c -eq "'") { $quote = $c }
        elseif ($c -eq '#') { return $s.Substring(0, $i).TrimEnd() }
    }
    $s.TrimEnd()
}

# A dependency's value, `{ a = 1, b = "x" }` or a bare version string, as key -> raw TOML value.
function ConvertFrom-TomlDep([string] $v) {
    $t = [ordered]@{}
    if ($v -match '^\{(.*)\}$') {
        foreach ($item in (Split-TomlItems $Matches[1])) {
            if ($item -notmatch '^([\w-]+)\s*=\s*(.+)$') { throw "Can't parse `"$item`" in $v" }
            $t[$Matches[1]] = $Matches[2].Trim()
        }
    } else {
        $t['version'] = $v
    }
    $t
}

function ConvertTo-TomlDep($t, [string[]] $first = @()) {
    $items = @($first) + @($t.Keys | ForEach-Object { "$_ = $($t[$_])" })
    '{ ' + ($items -join ', ') + ' }'
}

# The union of two TOML arrays of strings, in order.
function Merge-TomlArray([string] $a, [string] $b) {
    $items = @(Split-TomlItems ($a.Trim() -replace '^\[|\]$', '')) + @(Split-TomlItems ($b.Trim() -replace '^\[|\]$', ''))
    '[' + (($items | Select-Object -Unique) -join ', ') + ']'
}

# name -> raw value of the `[workspace.dependencies]` of a workspace's Cargo.toml.
function Read-WorkspaceDeps([string] $path) {
    $deps = @{}
    $in = $false
    foreach ($line in (Get-Content $path)) {
        if ($line -match '^\s*\[(.+?)\]\s*$') { $in = $Matches[1] -eq 'workspace.dependencies' }
        elseif ($in -and $line -match '^\s*([\w-]+)\s*=\s*(.+)$') { $deps[$Matches[1]] = Remove-TomlComment $Matches[2] }
    }
    $deps
}

# Returns the standalone manifest, why the target can't be built if it can't, and the lines that
# still inherit from the workspace. $Dir is the target's directory, $Top its top-level folder.
function Convert-Manifest([string] $Toml, [string] $Dir, [string] $Top, [hashtable] $WorkspaceDeps, [string] $Slint) {
    $lines = $Toml -split '\r?\n'
    $out = [System.Collections.Generic.List[string]]::new()
    $table = ''; $drop = $false; $skip = $null
    $removed = [System.Collections.Generic.List[string]]::new()
    $topPrefix = [System.IO.Path]::GetFullPath($Top).TrimEnd('\') + '\'
    for ($i = 0; $i -lt $lines.Count; $i++) {
        $line = $lines[$i]
        if ($line -match '^\s*\[\[?\s*(.+?)\s*\]\]?\s*(#.*)?$') {
            $table = $Matches[1]
            # Only binaries are built, and dev-dependencies may point at upstream's internal crates.
            $drop = $table -match '(^|\.)dev-dependencies(\.|$)'
            if ($table -eq 'lints') {
                $j = $i + 1
                while ($j -lt $lines.Count -and $lines[$j] -notmatch '^\s*\[') { $j++ }
                $body = if ($j -gt $i + 1) { @($lines[($i + 1)..($j - 1)] | Where-Object { $_ -notmatch '^\s*(#.*)?$' }) } else { @() }
                if ($body.Count -eq 1 -and $body[0] -match '^\s*workspace\s*=\s*true\s*(#.*)?$') { $i = $j - 1; continue }
            }
            if (-not $drop) { $out.Add($line) }
            continue
        }
        if ($drop) { continue }
        if ($table -eq 'package' -and $line -match '^\s*([\w-]+)(\.workspace\s*=\s*true|\s*=\s*\{\s*workspace\s*=\s*true\s*\})\s*(#.*)?$') {
            switch ($Matches[1]) {
                'edition' { $out.Add('edition = "2024"') }
                'version' { $out.Add('version = "1.19.0"') }
            }
            continue
        }
        if ($table -match '(^|\.)(build-)?dependencies$') {
            $code = Remove-TomlComment $line
            if ($code -match '^(\s*)([\w-]+)\.workspace\s*=\s*true$') { $code = "$($Matches[1])$($Matches[2]) = { workspace = true }" }
            if ($code -match '^(\s*)([\w-]+)\s*=\s*(\{.*\})$') {
                $indent = $Matches[1]; $name = $Matches[2]
                $dep = ConvertFrom-TomlDep $Matches[3]
                $text = $null
                if ($dep.Contains('workspace') -and $WorkspaceDeps.ContainsKey($name)) {
                    $base = ConvertFrom-TomlDep $WorkspaceDeps[$name]
                    foreach ($k in @($dep.Keys)) {
                        if ($k -eq 'workspace') { continue }
                        if ($k -eq 'features' -and $base.Contains('features')) { $base['features'] = Merge-TomlArray $base['features'] $dep['features'] }
                        else { $base[$k] = $dep[$k] }
                    }
                    $dep = $base
                    $text = ConvertTo-TomlDep $dep
                } elseif ($name -eq 'slint' -and $dep.Contains('path') -and $dep['path'] -match '^"(\.\./)+api/rs/slint"$') {
                    $dep.Remove('path')
                    $text = ConvertTo-TomlDep $dep -first $Slint
                } elseif ($name -eq 'slint-build' -and $dep.Contains('path') -and $dep['path'] -match '^"(\.\./)+api/rs/build"$') {
                    $dep.Remove('path')
                    $text = if ($dep.Count) { ConvertTo-TomlDep $dep -first 'version = "=1.19.0"' } else { '"=1.19.0"' }
                }
                # An optional mcu-board-support is only reached by a `simulator` feature, which isn't
                # enabled. But cargo resolves it anyway, and its own path dependencies on upstream's
                # crates/ aren't in the mirror, so the dependency and the features naming it go.
                if ($name -eq 'mcu-board-support' -and $dep['optional'] -eq 'true') { $removed.Add($name); continue }
                # A path dependency that leaves the target's top-level folder isn't in the mirror.
                if ($dep.Contains('path') -and -not $skip) {
                    $p = $dep['path'].Trim('"', "'")
                    $abs = [System.IO.Path]::GetFullPath((Join-Path $Dir $p)).TrimEnd('\') + '\'
                    if (-not $abs.StartsWith($topPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
                        $skip = "skipped: path dependency $name ($p)"
                    }
                }
                if ($text) { $line = "$indent$name = $text" }
            }
        }
        $out.Add($line)
    }
    if ($removed.Count) {
        $names = ($removed | ForEach-Object { [regex]::Escape($_) }) -join '|'
        $table = ''
        $kept = foreach ($line in $out) {
            if ($line -match '^\s*\[\[?\s*(.+?)\s*\]\]?\s*(#.*)?$') { $table = $Matches[1] }
            if ($table -eq 'features' -and $line -match "^\s*[\w-]+\s*=\s*\[.*`"(dep:)?($names)`"") { continue }
            $line
        }
        $out = [System.Collections.Generic.List[string]]::new([string[]]@($kept))
    }
    $left =@($out | Where-Object { $_ -notmatch '^\s*#' -and $_ -match 'workspace\s*=\s*true|\.workspace\b' })
    [pscustomobject]@{ Toml = ($out -join "`n"); Skip = $skip; Left = $left }
}

# --- Windows: finding, moving and capturing an app's window ---------------------------------

function Initialize-Windows {
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    if (-not ('UE' -as [type])) {
        Add-Type @'
using System; using System.Runtime.InteropServices; using System.Text;
public static class UE {
  [StructLayout(LayoutKind.Sequential)] public struct R { public int l,t,r,b; }
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern int GetWindowLongW(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out R r, int s);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern int GetWindowTextLengthW(IntPtr h);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  public static string Title(IntPtr h) {
    int n = GetWindowTextLengthW(h);
    var s = new StringBuilder(n + 1);
    GetWindowTextW(h, s, n + 1);
    return s.ToString();
  }
  // The largest visible top-level window of the process that isn't a tool window, preferring one
  // with a title.
  public static IntPtr Find(uint pid) {
    IntPtr best = IntPtr.Zero; long area = 0; bool titled = false;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      R r;
      if (p != pid || !IsWindowVisible(h) || (GetWindowLongW(h, -20) & 0x80) != 0 || !GetWindowRect(h, out r)) return true;
      if (r.r - r.l <= 0 || r.b - r.t <= 0) return true;
      long a = (long)(r.r - r.l) * (r.b - r.t);
      bool t = GetWindowTextLengthW(h) > 0;
      if ((t && !titled) || (t == titled && a > area)) { best = h; area = a; titled = t; }
      return true;
    }, IntPtr.Zero);
    return best;
  }
}
'@
    }
    [UE]::SetProcessDpiAwarenessContext([IntPtr]-4) | Out-Null  # per-monitor v2: physical pixels
}

function Read-SharedText([string] $path) {
    if (-not (Test-Path -LiteralPath $path)) { return '' }
    $fs = [System.IO.File]::Open($path, 'Open', 'Read', 'ReadWrite')
    try { [System.IO.StreamReader]::new($fs).ReadToEnd() } finally { $fs.Dispose() }
}

# Starts the app, waits for its window, captures it and closes it.
function Invoke-App([string] $Exe, [string] $Dir, [string] $Bin, [string] $Shot) {
    $r = [ordered]@{ window = 'no'; title = ''; size = ''; seconds = ''; exitCode = ''; panicked = ''; stderrHead = ''; shot = '' }
    if (-not (Test-Path $Exe)) { $r.stderrHead = "harness: no $Exe"; return $r }
    $outFile = Join-Path $Dir "$Bin.stdout.txt"
    $errFile = Join-Path $Dir "$Bin.stderr.txt"
    $scale = $env:SLINT_SCALE_FACTOR
    $env:SLINT_SCALE_FACTOR = $null  # the monitor's own scale
    try {
        $p = Start-Process -FilePath $Exe -WorkingDirectory $Dir -NoNewWindow -PassThru `
            -RedirectStandardOutput $outFile -RedirectStandardError $errFile
    } finally { $env:SLINT_SCALE_FACTOR = $scale }
    $null = $p.Handle  # keeps the exit code readable
    try {
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $h = [IntPtr]::Zero
        while ($sw.Elapsed.TotalSeconds -lt $WaitSeconds) {
            $h = [UE]::Find([uint32]$p.Id)
            if ($h -ne [IntPtr]::Zero -or $p.HasExited) { break }
            Start-Sleep -Milliseconds 100
        }
        if ($h -ne [IntPtr]::Zero) {
            $r.window = 'yes'
            $r.seconds = [math]::Round($sw.Elapsed.TotalSeconds, 1)
            $r.title = [UE]::Title($h)
            # Visible bounds, without the invisible resize border.
            $wr = New-Object UE+R; $fr = New-Object UE+R
            [UE]::GetWindowRect($h, [ref]$wr) | Out-Null
            if ([UE]::DwmGetWindowAttribute($h, 9, [ref]$fr, 16) -ne 0) { $fr = $wr }  # DWMWA_EXTENDED_FRAME_BOUNDS
            $area = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
            $x = [math]::Max($area.Left, $area.Right - 16 - ($fr.r - $fr.l))
            # SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE. The window then goes above the others
            # (HWND_TOPMOST, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE), still unactivated, so that
            # the capture doesn't show whatever sits at the screen's corner. The app closes right after.
            [UE]::SetWindowPos($h, [IntPtr]::Zero, $wr.l + $x - $fr.l, $wr.t + $area.Top - $fr.t, 0, 0, 0x15) | Out-Null
            [UE]::SetWindowPos($h, [IntPtr]-1, 0, 0, 0, 0, 0x13) | Out-Null
            Start-Sleep -Milliseconds $SettleMs
            if ($p.HasExited) {
                $r.exitCode = $p.ExitCode  # it ended while settling, so there is nothing to capture
            } else {
                [UE]::DwmGetWindowAttribute($h, 9, [ref]$fr, 16) | Out-Null
                $w = $fr.r - $fr.l; $ht = $fr.b - $fr.t
                $r.size = "${w}x${ht}"
                $bmp = [System.Drawing.Bitmap]::new($w, $ht)
                $g = [System.Drawing.Graphics]::FromImage($bmp)
                try {
                    $g.CopyFromScreen($fr.l, $fr.t, 0, 0, $bmp.Size)
                    $bmp.Save($Shot, [System.Drawing.Imaging.ImageFormat]::Png)
                    $r.shot = $Shot
                } finally { $g.Dispose(); $bmp.Dispose() }
                [UE]::PostMessageW($h, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null  # WM_CLOSE
                $p.WaitForExit(3000) | Out-Null
            }
        } elseif ($p.HasExited) {
            $r.exitCode = $p.ExitCode
        }
    } catch {
        $r.stderrHead = "harness: $($_.Exception.Message)"
    } finally {
        if (-not $p.HasExited) { try { $p.Kill($true) } catch { } }
        $p.WaitForExit(3000) | Out-Null
    }
    $err = Read-SharedText $errFile
    $r.panicked = [bool]($err -match 'panicked')
    $head = @($err -split '\r?\n' | Where-Object { $_ } | Select-Object -First 3 | ForEach-Object { $_.Substring(0, [math]::Min($_.Length, 200)) })
    if ($head) { $r.stderrHead = ($head -join ' | ') }
    $r
}

# --- Main ---------------------------------------------------------------------------------

$repo = Split-Path -Parent $PSScriptRoot
$fork = if ($Git) {
    "git = `"https://github.com/matarillo/slint-wuc`", tag = `"$Git`""
} else {
    "path = `"$((Join-Path $repo 'crates/slint') -replace '\\', '/')`""
}
$upstream = Join-Path $repo 'reference/slint-1.19.0'
if (-not (Test-Path (Join-Path $upstream 'examples'))) {
    throw "Upstream's sources aren't in $upstream (reference/ isn't tracked). Get them with: " +
        "git clone --depth 1 --branch v1.19.0 https://github.com/slint-ui/slint reference/slint-1.19.0"
}
# Next to the repository's own build output, wherever .cargo/config.toml puts it.
$target = (cargo metadata --format-version 1 --no-deps --manifest-path (Join-Path $repo 'Cargo.toml') |
    ConvertFrom-Json).target_directory
$work = Join-Path $target 'upstream-examples'
$shots = Join-Path $work 'shots'
New-Item -ItemType Directory -Force $shots | Out-Null
$drive = [System.IO.DriveInfo]::new([System.IO.Path]::GetPathRoot($work))
function Get-FreeGB { [math]::Round($drive.AvailableFreeSpace / 1GB, 1) }
$doRun = $Run -and -not $NoRun
if ($doRun) { Initialize-Windows }
$workspaceDeps = @{}
$targets = $Examples | ForEach-Object {
    $t = ($_ -replace '\\', '/').Trim('/')
    if ($t -notmatch '/') { "examples/$t" } else { $t }
}

$results = [System.Collections.Generic.List[object]]::new()
function New-Result($f, $t, $bin = '') {
    [pscustomobject]@{
        form = $f; target = $t; bin = $bin; built = $false; slintIsFork = $false; skipped = ''
        window = ''; title = ''; size = ''; secondsToWindow = ''; exitCode = ''; panicked = ''
        stderrHead = ''; shot = ''; buildLog = ''
    }
}

$stop = $false
foreach ($f in $Form) {
    # Upstream's trees side by side, so that paths between them keep working.
    # demos/food-processor imports a font of upstream's own, from internal/common/sharedfontique.
    foreach ($tree in 'examples', 'demos', 'logo', 'internal/common/sharedfontique') {
        $dst = Join-Path $work "$f/$tree"
        New-Item -ItemType Directory -Force $dst | Out-Null
        foreach ($child in Get-ChildItem -LiteralPath (Join-Path $upstream $tree)) {
            if (-not (Test-Path -LiteralPath (Join-Path $dst $child.Name))) {
                Copy-Item -Recurse -LiteralPath $child.FullName -Destination (Join-Path $dst $child.Name)
            }
        }
    }
    foreach ($t in $targets) {
        $row = New-Result $f $t
        $results.Add($row)
        if ($stop) { $row.skipped = 'skipped: disk'; continue }
        $parts = $t -split '/'
        $topRel = "$($parts[0])/$($parts[1])"
        if (-not (Test-Path (Join-Path $upstream "$t/Cargo.toml"))) { $row.skipped = 'failed: no Cargo.toml'; continue }
        Write-Host "[$f] $t"
        $topDir = [System.IO.Path]::GetFullPath((Join-Path $work "$f/$topRel"))
        if (Test-Path $topDir) { Remove-Item -Recurse -Force $topDir }
        Copy-Item -Recurse (Join-Path $upstream $topRel) $topDir
        $dir = [System.IO.Path]::GetFullPath((Join-Path $work "$f/$t"))
        $manifest = Join-Path $dir 'Cargo.toml'

        if (-not $workspaceDeps.ContainsKey($parts[0])) {
            $workspaceDeps[$parts[0]] = Read-WorkspaceDeps (Join-Path $upstream "$($parts[0])/Cargo.toml")
        }
        $slint = if ($f -eq 'patch') { 'version = "1.19"' } else { $fork }
        $conv = Convert-Manifest -Toml (Get-Content -Raw $manifest) -Dir $dir -Top $topDir `
            -WorkspaceDeps $workspaceDeps[$parts[0]] -Slint $slint
        if ($conv.Skip) { $row.skipped = $conv.Skip; continue }
        if ($conv.Left) { $row.skipped = "failed: manifest still inherits from the workspace: $($conv.Left[0].Trim())"; continue }
        $toml = $conv.Toml.TrimEnd() + "`n`n[workspace]`n"
        if ($f -eq 'patch') { $toml += "`n[patch.crates-io]`nslint = { $fork }`n" }
        Set-Content -NoNewline -Path $manifest -Value $toml

        if ((Get-FreeGB) -lt $MinFreeGB) { $stop = $true; $row.skipped = 'skipped: disk'; continue }
        $log = Join-Path $dir 'build.log'
        $row.buildLog = $log
        $saved = 'CARGO_PROFILE_DEV_DEBUG', 'CARGO_INCREMENTAL' | ForEach-Object { , @($_, [Environment]::GetEnvironmentVariable($_)) }
        $env:CARGO_PROFILE_DEV_DEBUG = '0'; $env:CARGO_INCREMENTAL = '0'
        try {
            cargo build --manifest-path $manifest --target-dir (Join-Path $work 'target') *> $log
            $row.built = $LASTEXITCODE -eq 0
        } finally {
            foreach ($s in $saved) { [Environment]::SetEnvironmentVariable($s[0], $s[1]) }
        }
        # A `slint` from crates.io has a registry source: the patch was not used.
        $lock = if (Test-Path (Join-Path $dir 'Cargo.lock')) { Get-Content -Raw (Join-Path $dir 'Cargo.lock') } else { '' }
        $row.slintIsFork = [bool]($lock -match '(?m)^name = "slint"\r?\nversion = "1\.19\.0"\r?\n(?!source = "registry)')
        if (-not $row.built -or -not $doRun) { continue }

        $meta = cargo metadata --no-deps --format-version 1 --manifest-path $manifest 2>$null | ConvertFrom-Json
        $bins = @($meta.packages.targets | Where-Object { $_.kind -contains 'bin' } | ForEach-Object { $_.name })
        if (-not $bins) { $row.window = 'no'; $row.stderrHead = 'harness: no binary target'; continue }
        $results.Remove($row) | Out-Null
        foreach ($bin in $bins) {
            Write-Host "[$f] $t $bin"
            $b = New-Result $f $t $bin
            $b.built = $true; $b.slintIsFork = $row.slintIsFork; $b.buildLog = $log
            $name = ($t -replace '/', '-') + "-$bin.png"
            if ($Form.Count -gt 1) { $name = "$f-$name" }
            $app = Invoke-App (Join-Path $work "target/debug/$bin.exe") $dir $bin (Join-Path $shots $name)
            $b.window = $app.window; $b.title = $app.title; $b.size = $app.size; $b.secondsToWindow = $app.seconds
            $b.exitCode = $app.exitCode; $b.panicked = $app.panicked; $b.stderrHead = $app.stderrHead; $b.shot = $app.shot
            $results.Add($b)
        }
    }
}

$results | Format-Table form, target, bin, built, slintIsFork, skipped, window, title, size, secondsToWindow, exitCode, panicked -AutoSize
$columns = 'form', 'target', 'bin', 'built', 'slintIsFork', 'skipped', 'window', 'title', 'size', 'secondsToWindow',
    'exitCode', 'panicked', 'stderrHead', 'shot', 'buildLog'
$tsv = @($columns -join "`t") + @($results | ForEach-Object {
    $row = $_
    ($columns | ForEach-Object { "$($row.$_)" -replace '[\t\r\n]+', ' ' }) -join "`t"
})
Set-Content -Path (Join-Path $work 'results.tsv') -Value $tsv
$targetBytes = (Get-ChildItem -Recurse -File (Join-Path $work 'target') -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
Write-Host ("Results: {0}`nFree on {1}: {2} GB; {3}: {4:N1} GB" -f (Join-Path $work 'results.tsv'), $drive.Name, (Get-FreeGB),
    (Join-Path $work 'target'), ($targetBytes / 1GB))

$failed = $results | Where-Object {
    $_.skipped -eq 'skipped: disk' -or $_.skipped -like 'failed:*' -or
    (-not $_.skipped -and (-not $_.built -or -not $_.slintIsFork)) -or
    ($doRun -and $_.built -and $_.window -ne 'yes')
}
if ($failed) { exit 1 }
