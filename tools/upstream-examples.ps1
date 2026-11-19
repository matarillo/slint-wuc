# Copyright © Kentaro Inomata
# SPDX-License-Identifier: MIT-0
#
# Builds upstream's examples on the fork, with their sources unmodified, as an application
# written for upstream Slint would build after following the README.
#
# Each example is copied out of reference/slint-1.19.0/examples, at the same depth next to a copy
# of upstream's logo/, which some examples load through `../../logo`. Only its Cargo.toml changes,
# from upstream's workspace paths to what an application outside the repository writes:
#   -Form patch: `slint = "1.19"` from crates.io, replaced by `[patch.crates-io]`.
#   -Form git:   `slint` from the fork directly.
# The fork is this clone's crates/slint, or with -Git <tag>, that tag on GitHub, as the README
# writes it. The script then checks in Cargo.lock that `slint` is the fork, not crates.io's.
#
# Usage: tools/upstream-examples.ps1 [-Form patch,git] [-Examples memory,slide_puzzle,system-tray] [-Git <tag>]
# Run from PowerShell, where cargo finds the MSVC linker.
param(
    [ValidateSet('patch', 'git')] [string[]] $Form = @('patch', 'git'),
    [string[]] $Examples = @('memory', 'slide_puzzle', 'system-tray'),
    [string] $Git
)
$ErrorActionPreference = 'Stop'
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

$results = foreach ($f in $Form) {
    foreach ($e in $Examples) {
        $dir = Join-Path $work "$f/examples/$e"
        if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
        New-Item -ItemType Directory -Force (Split-Path $dir) | Out-Null
        Copy-Item -Recurse (Join-Path $upstream "examples/$e") $dir
        if (-not (Test-Path (Join-Path $work "$f/logo"))) {
            Copy-Item -Recurse (Join-Path $upstream 'logo') (Join-Path $work "$f/logo")
        }

        $manifest = Join-Path $dir 'Cargo.toml'
        $toml = Get-Content -Raw $manifest
        $slint = if ($f -eq 'patch') { 'version = "1.19"' } else { $fork }
        $toml = $toml -replace 'slint = \{ path = "\.\./\.\./api/rs/slint"', "slint = { $slint"
        $toml = $toml -replace 'slint-build = \{ path = "\.\./\.\./api/rs/build" \}', 'slint-build = "=1.19.0"'
        $toml = $toml -replace 'edition\.workspace = true', 'edition = "2024"'
        $toml = $toml -replace '(?m)^rust-version\.workspace = true\r?\n', ''
        if ($toml -match '\.workspace = true|workspace = true|\.\./\.\./') {
            throw "$e`: Cargo.toml still refers to upstream's workspace"
        }
        $toml += "`n[workspace]`n"
        if ($f -eq 'patch') { $toml += "`n[patch.crates-io]`nslint = { $fork }`n" }
        Set-Content -NoNewline -Path $manifest -Value $toml

        $log = Join-Path $dir 'build.log'
        cargo build --manifest-path $manifest --target-dir (Join-Path $work 'target') *> $log
        $built = $LASTEXITCODE -eq 0
        # A `slint` from crates.io has a registry source: the patch was not used.
        $lock = if (Test-Path (Join-Path $dir 'Cargo.lock')) { Get-Content -Raw (Join-Path $dir 'Cargo.lock') } else { '' }
        $fromFork = $lock -match '(?m)^name = "slint"\r?\nversion = "1\.19\.0"\r?\n(?!source = "registry)'
        [pscustomobject]@{
            form = $f; example = $e; built = $built; slintIsFork = $fromFork
            log = if ($built) { '' } else { $log }
        }
    }
}
$results | Format-Table -AutoSize
if ($results | Where-Object { -not $_.built -or -not $_.slintIsFork }) { exit 1 }
