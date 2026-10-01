# Run the vendored native ownership regression without upstream profiling tools.
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$probe = Join-Path $root '.bootstrap/libsql-regression'
New-Item -ItemType Directory -Path $probe -Force | Out-Null
$manifest = Get-Content -LiteralPath (Join-Path $root 'vendor/libsql/Cargo.toml') -Raw
$manifest = [regex]::Replace($manifest, '(?ms)^\[dev-dependencies\.(criterion|pprof)\]\r?\n.*?(?=^\[|\z)', '')
$manifest = $manifest.Replace('path = "src/lib.rs"', 'path = "../../vendor/libsql/src/lib.rs"')
$manifest += "`n[workspace]`n"
$manifest += "`n[profile.dev]`ndebug = 0`nincremental = false`n[profile.test]`ndebug = 0`nincremental = false`n"
$manifestPath = Join-Path $probe 'Cargo.toml'
[System.IO.File]::WriteAllText($manifestPath, $manifest, [System.Text.UTF8Encoding]::new($false))
& cargo test --manifest-path $manifestPath --target-dir (Join-Path $root 'target') --lib --no-default-features --features core disconnect_is_idempotent_and_clears_closed_handle -- --nocapture
exit $LASTEXITCODE
