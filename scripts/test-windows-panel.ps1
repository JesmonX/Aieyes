# Interactive Windows regression against the built app. Isolated data, no real accounts.
param([string]$Binary = "$PSScriptRoot/../apps/desktop/src-tauri/target/release/aieyes-desktop.exe")
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class PanelProbe {
  [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; public Point(int x,int y){X=x;Y=y;} }
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
  public delegate bool EnumProc(IntPtr h,IntPtr p);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc proc,IntPtr p);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h,StringBuilder text,int size);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h,StringBuilder text,int size);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out Rect r);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h,uint flags);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint x,uint y,uint data,UIntPtr extra);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  public static IntPtr Find(uint pid,string name,bool byClass) {
    IntPtr found=IntPtr.Zero;
    EnumWindows((h,p)=>{uint owner;GetWindowThreadProcessId(h,out owner);if(owner!=pid)return true;
      var text=new StringBuilder(256);if(byClass)GetClassName(h,text,256);else GetWindowText(h,text,256);
      if(text.ToString()==name){found=h;return false;}return true;},IntPtr.Zero);return found;
  }
}
'@
$null = [PanelProbe]::SetProcessDPIAware()
$out = Join-Path $PSScriptRoot '../.local/windows-ui-sync/native'
New-Item -ItemType Directory -Force $out | Out-Null
$result = [ordered]@{status='running'; checks=@(); dpi=$null; reason=$null}
if (-not [Environment]::UserInteractive) {
    $result.status='not-run'; $result.reason='Windows runner has no interactive desktop'
    $result | ConvertTo-Json | Set-Content (Join-Path $out 'result.json')
    Write-Output $result.reason
    exit 0
}
function Pump([int]$Milliseconds=100) {
    $until=[DateTime]::UtcNow.AddMilliseconds($Milliseconds)
    do {[System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 10} while([DateTime]::UtcNow -lt $until)
}
function Wait-For([scriptblock]$Check,[string]$Label) {
    $until=[DateTime]::UtcNow.AddSeconds(15)
    do {if (& $Check) {return}; Pump 50} while([DateTime]::UtcNow -lt $until)
    throw "Timeout: $Label"
}
function Assert-That([bool]$Condition,[string]$Label) {
    if (-not $Condition) {throw $Label}; $result.checks += $Label
}
function Click-At([int]$X,[int]$Y,[bool]$Right=$false) {
    $null=[PanelProbe]::SetCursorPos($X,$Y); Pump 40
    [PanelProbe]::mouse_event($(if($Right){8}else{2}),0,0,0,[UIntPtr]::Zero)
    [PanelProbe]::mouse_event($(if($Right){16}else{4}),0,0,0,[UIntPtr]::Zero)
    Pump 120
}
function Bounds([IntPtr]$Handle) {
    $rect=New-Object PanelProbe+Rect; $null=[PanelProbe]::GetWindowRect($Handle,[ref]$rect);return $rect
}
function Capsule-Click([bool]$Right=$false) {
    $r=Bounds $script:capsule;Click-At ($r.Left+($r.Right-$r.Left)/2) ($r.Top+($r.Bottom-$r.Top)/2) $Right
}
function Panel-Visible { $script:panel=[PanelProbe]::Find($app.Id,'Aieyes 面板',$false);return $script:panel -ne [IntPtr]::Zero -and [PanelProbe]::IsWindowVisible($script:panel) }
function Screenshot([string]$Name) {
    $bounds=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bitmap=New-Object System.Drawing.Bitmap $bounds.Width,$bounds.Height
    $graphics=[System.Drawing.Graphics]::FromImage($bitmap)
    try {$graphics.CopyFromScreen($bounds.Location,[System.Drawing.Point]::Empty,$bounds.Size);$bitmap.Save((Join-Path $out "$Name.png"))}
    finally {$graphics.Dispose();$bitmap.Dispose()}
}
$oldData=$env:AIEYES_DATA_DIR
$testData=Join-Path ([IO.Path]::GetTempPath()) ('aieyes-panel-'+[guid]::NewGuid())
New-Item -ItemType Directory $testData | Out-Null
$env:AIEYES_DATA_DIR=$testData
$app=$null;$background=$null
try {
    $app=Start-Process -FilePath (Resolve-Path $Binary) -PassThru
    Wait-For { $script:capsule=[PanelProbe]::Find($app.Id,'AieyesNativeCapsule',$true);$script:capsule -ne [IntPtr]::Zero } 'native capsule startup'
    $result.dpi=[PanelProbe]::GetDpiForWindow($script:capsule)
    $background=New-Object System.Windows.Forms.Form
    $background.Text='Aieyes isolated background input target';$background.FormBorderStyle='None';$background.WindowState='Maximized';$background.BackColor=[Drawing.Color]::FromArgb(49,67,83)
    $background.Show();$background.Activate();Pump 300
    $null=[PanelProbe]::SetForegroundWindow($background.Handle)
    Capsule-Click
    Wait-For {Panel-Visible} 'open panel'
    Assert-That ([PanelProbe]::GetForegroundWindow() -eq $background.Handle) 'opening preserves background keyboard focus'
    Screenshot 'open'
    $former=Bounds $script:panel
    # Deliberate interaction can activate the panel (enabling text/keyboard input).
    Click-At ($former.Left+200) ($former.Top+90)
    Assert-That ([PanelProbe]::GetForegroundWindow() -eq $script:panel) 'panel interaction can acquire keyboard focus'
    Click-At 25 200
    Wait-For {-not (Panel-Visible)} 'outside click closes panel'
    Assert-That ([PanelProbe]::GetAncestor([PanelProbe]::WindowFromPoint([PanelProbe+Point]::new($former.Left+100,$former.Top+100)),2) -eq $background.Handle) 'closed panel does not intercept desktop hit testing'
    Click-At ($former.Left+100) ($former.Top+100) $true
    Assert-That (-not (Panel-Visible)) 'right click on former panel stays in background application'
    $null=[PanelProbe]::SetCursorPos(20,200);Pump 300
    Screenshot 'closed'
    # Check actual desktop pixels for a surviving frame/shadow, not only HWND flags.
    $bitmap=[Drawing.Bitmap]::FromFile((Join-Path $out 'closed.png'))
    try {
      $different=0;$total=0;$expected=$background.BackColor.ToArgb()
      for($x=$former.Left;$x -lt $former.Right;$x+=4) {for($y=$former.Top;$y -lt $former.Bottom;$y+=4) {
        if($x -ge 0 -and $y -ge 0 -and $x -lt $bitmap.Width -and $y -lt $bitmap.Height){$total++;if($bitmap.GetPixel($x,$y).ToArgb() -ne $expected){$different++}}
      }}
      Assert-That ($total -gt 0 -and $different/$total -lt .005) 'closed panel leaves no visible surface or frame'
    } finally {$bitmap.Dispose()}
    for($i=0;$i -lt 12;$i++) {Capsule-Click;Wait-For {Panel-Visible} 'repeat open';Capsule-Click;Wait-For {-not (Panel-Visible)} 'repeat close'}
    Assert-That (-not (Panel-Visible)) 'repeated capsule toggles end fully hidden'
    Capsule-Click;Wait-For {Panel-Visible} 'open before menu'
    Capsule-Click $true;[System.Windows.Forms.SendKeys]::SendWait('{ESC}');Pump 200
    Assert-That (Panel-Visible) 'canceling capsule menu preserves open panel'
    # Select the second item: pin; then prove outside clicks no longer dismiss.
    Capsule-Click $true;[System.Windows.Forms.SendKeys]::SendWait('{HOME}{DOWN}{ENTER}');Pump 200
    Click-At 25 200;Assert-That (Panel-Visible) 'pinned panel survives outside click'
    Capsule-Click $true;[System.Windows.Forms.SendKeys]::SendWait('{HOME}{DOWN}{ENTER}');Pump 200
    Click-At 25 200;Wait-For {-not (Panel-Visible)} 'unpin restores outside dismissal'
    Capsule-Click;Wait-For {Panel-Visible} 'open before renderer rebuild'
    $oldPanel=$script:panel
    Capsule-Click $true;[System.Windows.Forms.SendKeys]::SendWait('{HOME}{DOWN}{DOWN}{ENTER}')
    Wait-For {(Panel-Visible) -and $script:panel -ne $oldPanel} 'rebuild creates a new visible panel'
    Capsule-Click;Wait-For {-not (Panel-Visible)} 'rebuilt panel closes'
    $result.status='passed'
} catch {
    $result.status='failed';$result.reason=$_.ToString();Screenshot 'failure';throw
} finally {
    $result | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $out 'result.json')
    if($background){$background.Close();$background.Dispose()}
    if($app -and -not $app.HasExited){Stop-Process -Id $app.Id -Force}
    $env:AIEYES_DATA_DIR=$oldData
    Remove-Item -Recurse -Force $testData
}
