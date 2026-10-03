param([ValidateSet('debug', 'release')][string]$Mode = 'debug')
$ErrorActionPreference = 'Stop'
$ProjectDir = Split-Path -Parent $PSScriptRoot
function Invoke-Tauri {
    param([string[]]$CliArgs)
    if (Get-Command tauri -ErrorAction SilentlyContinue) { & tauri @CliArgs }
    else { & cargo tauri @CliArgs }
    if ($LASTEXITCODE -ne 0) { throw 'Tauri command failed. Install CLI: npm install --global @tauri-apps/cli@2' }
}
if ($env:AIEYES_OFFLINE -eq '1') { $env:CARGO_NET_OFFLINE = 'true' }
Push-Location (Join-Path $ProjectDir 'apps/desktop')
try {
    if ($Mode -eq 'debug') {
        & cargo build --manifest-path src-tauri/Cargo.toml --locked --features custom-protocol
        if ($LASTEXITCODE -ne 0) { throw 'Desktop build failed.' }
        Write-Output (Join-Path $ProjectDir 'apps/desktop/src-tauri/target/debug/aieyes-desktop.exe')
    } else {
        Invoke-Tauri -CliArgs @('--version')
        Invoke-Tauri -CliArgs @('build', '--ci', '--bundles', 'nsis', '--', '--locked')
        Write-Output (Join-Path $ProjectDir 'apps/desktop/src-tauri/target/release/bundle/nsis')
    }
} finally { Pop-Location }
