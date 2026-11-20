# Copyright © Kentaro Inomata
# SPDX-License-Identifier: MIT-0
#
# Builds an application written for upstream Slint three ways, and compares what each draws.
# The application's sources are not touched: it is copied out, and only the copy's Cargo.toml
# changes. The application needs a feature, `screenshot`, that gives it `--screenshot <file>`:
# it draws, saves a PNG and exits.
#   locked:   as the application ships, with the slint its Cargo.lock names.
#   upstream: `slint = "1.19"` from crates.io, which re-resolves the lock.
#   fork:     the same, plus `[patch.crates-io]` for `slint`, as the README writes it.
# upstream and fork differ only by the patch, so a difference between their screenshots is the
# fork's. locked against upstream is what changed between 1.18.1 and 1.19.0, not the fork's doing.
# The fork is this clone's crates/slint, or with -Git <tag>, that tag on GitHub.
#
# Each side builds in its own target directory: the package has the same name on every side, so
# a shared one would overwrite the executable.
#
# The work is in <target>/app-compat/<Label>/: <side>/<app>/ (the copy), <side>/build.log and
# <side>/report/, and shots/. report/ holds the diffs of the manifest and the lock, and
# `cargo tree` for slint. shots/ holds each side's screenshots, diff/<scene>.<newer>-vs-<older>.png
# (older | newer | their difference, as ui-check lays them out), and report.tsv with the share of
# pixels that differ by more than 16 and by more than 64 (of 255).
#
# Usage: tools/app-compat.ps1 -App <application directory> [-Sides locked,upstream,fork]
#        [-Git <tag>] [-Scenes <file>] [-Label msvc] [-SkipBuild]
# Needs ui-check built (`cargo build -p ui-check`). Run from PowerShell, where cargo finds the
# MSVC linker. -Label keeps the output of another toolchain apart from this one's.
# -SkipBuild reuses the executables of an earlier run and only takes the screenshots again.
param(
    [Parameter(Mandatory)] [string] $App,
    [ValidateSet('locked', 'upstream', 'fork')] [string[]] $Sides = @('locked', 'upstream', 'fork'),
    [string] $Git,
    [string] $Scenes = (Join-Path $PSScriptRoot 'app-compat/slokk.scenes'),
    [string] $Label = 'msvc',
    [switch] $SkipBuild
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$forkSource = if ($Git) {
    "git = `"https://github.com/matarillo/slint-wuc`", tag = `"$Git`""
} else {
    "path = `"$((Join-Path $repo 'crates/slint') -replace '\\', '/')`""
}
$timeoutMs = 60000

$appDir = (Get-Item -LiteralPath $App).FullName
if (-not (Test-Path (Join-Path $appDir 'Cargo.toml'))) { throw "No Cargo.toml in $appDir" }
# The slint that the application's own lock names, which the locked side must keep.
$appLock = Join-Path $appDir 'Cargo.lock'
$lockedVersion = if ((Test-Path $appLock) -and
    ((Get-Content -Raw $appLock) -match '(?m)^name = "slint"\r?\nversion = "([^"]+)"')) { $Matches[1] } else { $null }
if (-not $lockedVersion -and $Sides -contains 'locked') { throw "No slint in $appLock; leave out the locked side" }
# The copy keeps the folder's name, so paths in the build look as they do in the original.
$name = Split-Path -Leaf $appDir
$sceneList = foreach ($line in Get-Content $Scenes) {
    $line = $line.Trim()
    if ($line -eq '' -or $line.StartsWith('#')) { continue }
    $words = $line -split '\s+'
    [pscustomobject]@{ name = $words[0]; args = @($words | Select-Object -Skip 1) }
}

# Cargo reads .cargo/config.toml from the current directory, not the manifest's, so it runs in the
# repository: the same settings as the fork's own builds, wherever the script is started from.
function Invoke-InRepo([scriptblock] $Block) {
    Push-Location $repo
    try { & $Block } finally { Pop-Location }
}

# Next to the repository's own build output, wherever .cargo/config.toml puts it.
$target = [IO.Path]::GetFullPath((Invoke-InRepo {
    (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
}))
$uiCheck = Join-Path $target 'debug/ui-check.exe'
if (-not (Test-Path $uiCheck)) { throw "$uiCheck is missing; build it with: cargo build -p ui-check" }
$base = Join-Path $target "app-compat/$Label"
$shots = Join-Path $base 'shots'

# One string per line, LF only: Set-Content would write CRLF, and -NoNewline misbehaves here.
function Write-Lf([string] $Path, $Lines) {
    [IO.File]::WriteAllText($Path, ((@($Lines) -join "`n") + "`n"))
}

# Sets the requirement of dependency $Dep in $Toml, written as `dep = "x"` or `dep = { version = "x" }`.
function Set-Requirement([string] $Toml, [string] $Dep, [string] $Requirement) {
    $esc = [regex]::Escape($Dep)
    $plain = "(?m)^($esc\s*=\s*)`"[^`"]*`""
    $table = "(?m)^($esc\s*=\s*\{[^}\r\n]*?\bversion\s*=\s*)`"[^`"]*`""
    $pattern = if ($Toml -match $plain) { $plain } elseif ($Toml -match $table) { $table } else {
        throw "Cargo.toml has no `"$Dep`" requirement to replace"
    }
    $Toml -replace $pattern, "`${1}`"$Requirement`""
}

$failed = $false
$exes = @{}
$summary = foreach ($side in $Sides) {
    $sideDir = Join-Path $base $side
    $dir = Join-Path $sideDir $name
    $report = Join-Path $sideDir 'report'
    $targetDir = Join-Path $base "target-$side"
    $manifest = Join-Path $dir 'Cargo.toml'
    $log = Join-Path $sideDir 'build.log'
    $seconds = $null
    $ok = $true
    $versions = @()

    if (-not $SkipBuild) {
        if (Test-Path $sideDir) { Remove-Item -Recurse -Force $sideDir }
        New-Item -ItemType Directory -Force $dir, $report | Out-Null
        Get-ChildItem -Force -LiteralPath $appDir | Where-Object { $_.Name -notin 'target', '.git', '.local' } |
            Copy-Item -Recurse -Destination $dir
        # The originals, for the diffs below.
        Copy-Item (Join-Path $appDir 'Cargo.toml') (Join-Path $report 'orig.Cargo.toml')
        if (Test-Path (Join-Path $appDir 'Cargo.lock')) {
            Copy-Item (Join-Path $appDir 'Cargo.lock') (Join-Path $report 'orig.Cargo.lock')
        }

        if ($side -ne 'locked') {
            $toml = Get-Content -Raw $manifest
            $toml = Set-Requirement $toml 'slint' '1.19'
            $toml = Set-Requirement $toml 'slint-build' '=1.19.0'
            if ($side -eq 'fork') {
                if ($toml -match '\[patch\.crates-io\]') { throw "$name already has [patch.crates-io]" }
                $toml += "`n[patch.crates-io]`nslint = { $forkSource }`n"
            }
            [IO.File]::WriteAllText($manifest, $toml)
        }

        $watch = [Diagnostics.Stopwatch]::StartNew()
        Invoke-InRepo { cargo build --features screenshot --manifest-path $manifest --target-dir $targetDir *> $log }
        $ok = $LASTEXITCODE -eq 0
        $seconds = [int] $watch.Elapsed.TotalSeconds
    }

    # The executable is named after the package, which may not be the folder's name.
    $exe = ''
    if (Test-Path $manifest) {
        $package = (Invoke-InRepo { cargo metadata --format-version 1 --no-deps --manifest-path $manifest } |
            ConvertFrom-Json).packages[0]
        $bin = $package.targets | Where-Object { $_.kind -contains 'bin' } | Select-Object -First 1
        if ($bin) { $exe = Join-Path $targetDir "debug/$($bin.name).exe" }
    }
    $ok = $ok -and $exe -ne '' -and (Test-Path $exe)
    $exes[$side] = if ($ok) { $exe } else { $null }

    # Which slint the lock chose. A `slint` from crates.io has a registry source: for the fork,
    # that means the patch was not used.
    $lockFile = Join-Path $dir 'Cargo.lock'
    $lock = if (Test-Path $lockFile) { Get-Content -Raw $lockFile } else { '' }
    $slintPattern = switch ($side) {
        'locked' { '(?m)^name = "slint"\r?\nversion = "' + [regex]::Escape($lockedVersion) + '"\r?\nsource = "registry' }
        'upstream' { '(?m)^name = "slint"\r?\nversion = "1\.19\.0"\r?\nsource = "registry' }
        'fork' { '(?m)^name = "slint"\r?\nversion = "1\.19\.0"\r?\n(?!source = "registry)' }
    }
    $slintOk = $lock -match $slintPattern

    if (Test-Path $manifest) {
        $versions = [regex]::Matches($lock,
            '(?m)^name = "(slint|slint-build|i-slint-compiler|i-slint-core)"\r?\nversion = "([^"]*)"(?:\r?\nsource = "([^"]*)")?') |
            ForEach-Object {
                '{0} {1} {2}' -f $_.Groups[1].Value, $_.Groups[2].Value,
                    $(if ($_.Groups[3].Success) { $_.Groups[3].Value } else { '(path)' })
            }
        # Relative paths, so the diffs don't name where the work directory is.
        Push-Location $sideDir
        try {
            Write-Lf (Join-Path $report 'Cargo.toml.diff') (git diff --no-index --no-color -- report/orig.Cargo.toml "$name/Cargo.toml")
            $stat = if (Test-Path 'report/orig.Cargo.lock') {
                git diff --no-index --no-color --stat -- report/orig.Cargo.lock "$name/Cargo.lock"
            } else { '(the application has no Cargo.lock)' }
            Write-Lf (Join-Path $report 'Cargo.lock.stat') (@($stat) + '' + $versions)
        } finally { Pop-Location }
        $tree = Invoke-InRepo { cargo tree --manifest-path $manifest -i slint -e normal --depth 0 }
        Write-Lf (Join-Path $report 'tree.txt') $(if ($LASTEXITCODE -eq 0) { $tree } else { '(cargo tree failed)' })
    }

    if (-not $ok -or -not $slintOk) { $failed = $true }
    [pscustomobject]@{
        side = $side; built = $ok; seconds = $seconds; slintSourceOk = $slintOk
        slint = ($versions | Where-Object { $_ -like 'slint *' }) -join ''
        log = if ($ok) { '' } else { $log }
    }
}

# One scene at a time: each opens a window, and one that something else covers or resizes could
# be drawn differently.
$sceneNotes = @{}
foreach ($side in $Sides) {
    New-Item -ItemType Directory -Force (Join-Path $shots $side) | Out-Null
    foreach ($scene in $sceneList) {
        $png = Join-Path $shots "$side/$($scene.name).png"
        $stderrLog = Join-Path $shots "$side/$($scene.name).log"
        Remove-Item -Force -ErrorAction Ignore $png, $stderrLog
        $note = $null
        if (-not $exes[$side]) {
            $note = "$side`: no executable"
        } else {
            $psi = [Diagnostics.ProcessStartInfo]::new($exes[$side])
            foreach ($arg in @('--screenshot', $png) + $scene.args) { $psi.ArgumentList.Add($arg) }
            # As `cargo run` in the application's directory would run it, in case it reads files there.
            $psi.WorkingDirectory = Join-Path (Join-Path $base $side) $name
            $psi.UseShellExecute = $false
            $psi.RedirectStandardError = $true
            $psi.RedirectStandardOutput = $true
            # The same scale factor whatever the monitor's.
            $psi.Environment['SLINT_SCALE_FACTOR'] = '1'
            $process = [Diagnostics.Process]::Start($psi)
            $stderr = $process.StandardError.ReadToEndAsync()
            $stdout = $process.StandardOutput.ReadToEndAsync()
            if (-not $process.WaitForExit($timeoutMs)) {
                $process.Kill($true)
                $process.WaitForExit()
                $note = "$side`: timeout"
            }
            [IO.File]::WriteAllText($stderrLog, $stderr.Result + $stdout.Result)
            if (-not $note -and -not (Test-Path $png)) { $note = "$side`: missing shot" }
        }
        if ($note) { $sceneNotes[$scene.name] += @($note) }
    }
}

# Writes older | newer | difference, and returns the two percentages: over 16, and over 64.
function Compare-Shots([string] $Scene, [string] $Older, [string] $Newer) {
    $a = Join-Path $shots "$Older/$Scene.png"
    $b = Join-Path $shots "$Newer/$Scene.png"
    if (-not ((Test-Path $a) -and (Test-Path $b))) { return @('', '') }
    New-Item -ItemType Directory -Force (Join-Path $shots 'diff') | Out-Null
    $result = & $uiCheck compare $a $b (Join-Path $shots "diff/$Scene.$Newer-vs-$Older.png")
    if ($LASTEXITCODE -ne 0) { return @('', '') }
    $result -split "`t"
}

$rows = @("scene`tfork_vs_upstream_16`tfork_vs_upstream_64`tupstream_vs_locked_16`tupstream_vs_locked_64`tnotes")
foreach ($scene in $sceneList) {
    $forkVsUpstream = Compare-Shots $scene.name 'upstream' 'fork'
    $upstreamVsLocked = Compare-Shots $scene.name 'locked' 'upstream'
    $notes = @($sceneNotes[$scene.name]) -join '; '
    $rows += (@($scene.name) + $forkVsUpstream + $upstreamVsLocked + $notes) -join "`t"
}
Write-Lf (Join-Path $shots 'report.tsv') $rows

$summary | Format-Table -AutoSize
$rows | ConvertFrom-Csv -Delimiter "`t" | Format-Table -AutoSize
"Screenshots and diffs: $shots"
if ($failed) { exit 1 }
