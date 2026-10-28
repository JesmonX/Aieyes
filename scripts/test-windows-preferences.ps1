# Run in a normal (non-elevated) interactive session; WebView2 150+ ignores debug environment flags when elevated.
# All temporary user settings are restored in finally.
param([string]$Binary = "$PSScriptRoot/../apps/desktop/src-tauri/target/debug/aieyes-desktop.exe", [string]$PlaywrightPath)
$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class PreferenceProbe {
  [DllImport("user32.dll",CharSet=CharSet.Unicode,SetLastError=true)]
  public static extern IntPtr SendMessageTimeout(IntPtr h,uint msg,IntPtr w,string l,uint flags,uint timeout,out IntPtr result);
}
'@
$clock = 'HKCU:\Control Panel\International'
$personalize = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'
$saved = @()
foreach ($pair in @(@($clock,'iTime'), @($clock,'sShortTime'), @($clock,'sTimeFormat'), @($personalize,'EnableTransparency'))) {
    $key = Get-Item $pair[0]
    $exists = $key.GetValueNames() -contains $pair[1]
    $saved += [pscustomobject]@{Path=$pair[0]; Name=$pair[1]; Exists=$exists; Value=$key.GetValue($pair[1]); Kind=$(if($exists){$key.GetValueKind($pair[1])}else{'String'})}
}
function Broadcast {
    $result = [IntPtr]::Zero
    $null = [PreferenceProbe]::SendMessageTimeout([IntPtr]0xffff,0x1a,[IntPtr]::Zero,'intl',2,1000,[ref]$result)
}
$root = Join-Path $env:TEMP ('aieyes-preferences-' + [guid]::NewGuid().ToString('N'))
$out = Join-Path $PSScriptRoot '../.local/windows-ui-sync/preferences'
New-Item -ItemType Directory -Force $root,$out | Out-Null
$envNames = @('AIEYES_DATA_DIR','WEBVIEW2_USER_DATA_FOLDER','WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS','AIEYES_PLAYWRIGHT_PATH')
$oldEnv = @{}; foreach($name in $envNames){$oldEnv[$name]=[Environment]::GetEnvironmentVariable($name)}
$app = $null
try {
    $env:AIEYES_DATA_DIR = $root
    $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $root 'webview'
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9334'
    if ($PlaywrightPath) {$env:AIEYES_PLAYWRIGHT_PATH=$PlaywrightPath}
    foreach ($test in @(@('0','h:mm tt','h:mm:ss tt','h12','materials',1), @('1','HH:mm','HH:mm:ss','h23','persisted',1), @('1','HH:mm','HH:mm:ss','h23','opaque',0))) {
        Set-ItemProperty $clock iTime $test[0]
        Set-ItemProperty $clock sShortTime $test[1]
        Set-ItemProperty $clock sTimeFormat $test[2]
        Set-ItemProperty $personalize EnableTransparency ([int]$test[5])
        Broadcast
        $app = Start-Process -FilePath (Resolve-Path $Binary) -WorkingDirectory (Split-Path (Resolve-Path $Binary)) -PassThru -RedirectStandardOutput (Join-Path $out ($test[4]+'.stdout.log')) -RedirectStandardError (Join-Path $out ($test[4]+'.stderr.log'))
        Start-Sleep -Seconds 2
        if ($app.HasExited) {throw "Test app exited before WebView2 started: $($app.ExitCode)"}
        $ErrorActionPreference = 'Continue'
        & node "$PSScriptRoot/test-windows-preferences.cjs" $test[3] $test[4] (Join-Path $out 'appearance.png') 2>&1 | ForEach-Object {"$_"}
        $nodeExit = $LASTEXITCODE
        $ErrorActionPreference = 'Stop'
        if ($nodeExit -ne 0) {throw "Native preferences case failed: $($test[4])"}
        if (-not $app.HasExited) {Stop-Process -Id $app.Id -Force}
        $app.WaitForExit(); $app = $null
        Start-Sleep -Milliseconds 800
    }
    Write-Output 'PASS native material preferences, invalid value rollback, persistence, transparency fallback, 12/24-hour user overrides and both interface languages'
} finally {
    if ($app -and -not $app.HasExited) {Stop-Process -Id $app.Id -Force}
    foreach ($value in $saved) {
        if ($value.Exists) {New-ItemProperty $value.Path -Name $value.Name -Value $value.Value -PropertyType $value.Kind -Force | Out-Null}
        else {Remove-ItemProperty $value.Path $value.Name -ErrorAction SilentlyContinue}
    }
    Broadcast
    foreach($name in $envNames){[Environment]::SetEnvironmentVariable($name,$oldEnv[$name])}
    Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
    Write-Output 'Original Windows user settings restored'
}
