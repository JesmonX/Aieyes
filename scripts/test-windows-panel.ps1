# Interactive Windows regression against the built app. Isolated data, no real accounts.
param([string]$Binary = "$PSScriptRoot/../apps/desktop/src-tauri/target/release/aieyes-desktop.exe", [switch]$VerifyMaterials)
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
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h,uint message,IntPtr w,IntPtr l);
  [DllImport("user32.dll")] public static extern bool GetMenuItemRect(IntPtr owner,IntPtr menu,uint item,out Rect rect);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetMenuString(IntPtr menu,uint item,StringBuilder text,int size,uint flags);
  public static IntPtr FindMenu(uint pid) {
    IntPtr found=IntPtr.Zero;
    EnumWindows((h,p)=>{uint owner;GetWindowThreadProcessId(h,out owner);if(owner!=pid||!IsWindowVisible(h))return true;
      var text=new StringBuilder(256);GetClassName(h,text,256);if(text.ToString()=="#32768"){found=h;return false;}return true;},IntPtr.Zero);return found;
  }
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
function Wait-For([scriptblock]$Check,[string]$Label,[int]$TimeoutSeconds=15) {
    $until=[DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if ($app -and $app.HasExited) {throw "Application exited during ${Label}: exit code $($app.ExitCode); see app-stderr.log"}
        if (& $Check) {return}
        Pump 50
    } while([DateTime]::UtcNow -lt $until)
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
function Open-CapsuleMenu {
    Capsule-Click $true
    Wait-For { [PanelProbe]::FindMenu($app.Id) -ne [IntPtr]::Zero } 'capsule menu appears'
}
function Choose-MenuItem([uint32]$Index,[string]$Expected) {
    Open-CapsuleMenu
    $popup=[PanelProbe]::FindMenu($app.Id)
    # MN_GETHMENU obtains the actual popup menu; do not assume keyboard activation
    # of a WS_EX_NOACTIVATE capsule or a fixed menu row height on different DPI.
    $menu=[PanelProbe]::SendMessage($popup,0x1e1,[IntPtr]::Zero,[IntPtr]::Zero)
    $label=New-Object Text.StringBuilder 256
    $null=[PanelProbe]::GetMenuString($menu,$Index,$label,256,0x400)
    Assert-That ($label.ToString() -eq $Expected) "native menu item: $Expected"
    $rect=New-Object PanelProbe+Rect
    Assert-That ([PanelProbe]::GetMenuItemRect([IntPtr]::Zero,$menu,$Index,[ref]$rect)) "locate native menu item: $Expected"
    Click-At (($rect.Left+$rect.Right)/2) (($rect.Top+$rect.Bottom)/2)
    Wait-For { [PanelProbe]::FindMenu($app.Id) -eq [IntPtr]::Zero } 'menu selection completes'
    Pump 200
}
function Panel-Visible { $script:panel=[PanelProbe]::Find($app.Id,'Aieyes 面板',$false);return $script:panel -ne [IntPtr]::Zero -and [PanelProbe]::IsWindowVisible($script:panel) }
function Screenshot([string]$Name) {
    $bounds=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bitmap=New-Object System.Drawing.Bitmap $bounds.Width,$bounds.Height
    $graphics=[System.Drawing.Graphics]::FromImage($bitmap)
    try {$graphics.CopyFromScreen($bounds.Location,[System.Drawing.Point]::Empty,$bounds.Size);$bitmap.Save((Join-Path $out "$Name.png"))}
    finally {$graphics.Dispose();$bitmap.Dispose()}
}
# Opt-in on an interactive Windows desktop with transparency enabled. This checks
# actual changing desktop pixels, which WebView screenshots cannot validate.
function Verify-Backdrop([IntPtr]$Handle,[string]$Name) {
    $rect=Bounds $Handle
    $width=$rect.Right-$rect.Left;$height=$rect.Bottom-$rect.Top
    $captures=@();$original=$background.BackColor
    try {
        foreach($color in @([Drawing.Color]::FromArgb(218,105,100),[Drawing.Color]::FromArgb(65,118,222))) {
            $background.BackColor=$color;$background.Refresh();Pump 800
            $bitmap=New-Object Drawing.Bitmap $width,$height
            $graphics=[Drawing.Graphics]::FromImage($bitmap)
            try {$graphics.CopyFromScreen($rect.Left,$rect.Top,0,0,[Drawing.Size]::new($width,$height))}
            finally {$graphics.Dispose()}
            $captures+=,$bitmap
            $bitmap.Save((Join-Path $out "$Name-backdrop-$($captures.Count).png"))
        }
        $changed=0;$total=0
        # Empty strips inside the window avoid text, counters and animated cards.
        $inset=[Math]::Max(6,[int](6*[PanelProbe]::GetDpiForWindow($Handle)/96))
        foreach($x in @($inset,($width-$inset-1))) {
            for($y=80;$y -lt $height-80;$y+=9) {
                $a=$captures[0].GetPixel($x,$y);$b=$captures[1].GetPixel($x,$y)
                if(([Math]::Abs([int]$a.R-[int]$b.R)+[Math]::Abs([int]$a.G-[int]$b.G)+[Math]::Abs([int]$a.B-[int]$b.B)) -gt 9){$changed++}
                $total++
            }
        }
        $result["$Name-backdrop"]=@{samples=$total;changed=$changed}
        Assert-That ($total -gt 0 -and $changed/$total -gt .25) "$Name visibly responds to desktop background changes"
    } finally {
        foreach($capture in $captures){$capture.Dispose()}
        $background.BackColor=$original;$background.Refresh();Pump 200
    }
}
$oldData=$env:AIEYES_DATA_DIR
$oldWebviewData=$env:WEBVIEW2_USER_DATA_FOLDER
$testData=Join-Path ([IO.Path]::GetTempPath()) ('aieyes-panel-'+[guid]::NewGuid())
New-Item -ItemType Directory $testData | Out-Null
$env:AIEYES_DATA_DIR=$testData
$env:WEBVIEW2_USER_DATA_FOLDER=Join-Path $testData 'webview2'
$app=$null;$background=$null
try {
    $startup=[Diagnostics.Stopwatch]::StartNew()
    $app=Start-Process -FilePath (Resolve-Path $Binary) -PassThru -RedirectStandardOutput (Join-Path $out 'app-stdout.log') -RedirectStandardError (Join-Path $out 'app-stderr.log')
    # A fresh runner initializes both WebViews before creating the capsule.
    # Give cold startup its own budget; keep interaction deadlines at 15 seconds.
    Wait-For { $script:capsule=[PanelProbe]::Find($app.Id,'AieyesNativeCapsule',$true);$script:capsule -ne [IntPtr]::Zero -and [PanelProbe]::IsWindowVisible($script:capsule) } 'native capsule startup' 60
    $result.startupMilliseconds=$startup.ElapsedMilliseconds
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
      $different=0;$total=0;$expected=$background.BackColor.ToArgb();$capsuleRect=Bounds $script:capsule
      for($x=$former.Left;$x -lt $former.Right;$x+=4) {for($y=$former.Top;$y -lt $former.Bottom;$y+=4) {
        # The capsule deliberately remains visible, including its own shadow.
        if($x -ge $capsuleRect.Left-8 -and $x -le $capsuleRect.Right+8 -and $y -ge $capsuleRect.Top-8 -and $y -le $capsuleRect.Bottom+8){continue}
        if($x -ge 0 -and $y -ge 0 -and $x -lt $bitmap.Width -and $y -lt $bitmap.Height){$total++;if($bitmap.GetPixel($x,$y).ToArgb() -ne $expected){$different++}}
      }}
      $result.pixelSamples=$total;$result.differentPixels=$different
      Assert-That ($total -gt 0 -and $different/$total -lt .005) 'closed panel leaves no visible surface or frame'
    } finally {$bitmap.Dispose()}
    for($i=0;$i -lt 12;$i++) {Capsule-Click;Wait-For {Panel-Visible} 'repeat open';Capsule-Click;Wait-For {-not (Panel-Visible)} 'repeat close'}
    Assert-That (-not (Panel-Visible)) 'repeated capsule toggles end fully hidden'
    Capsule-Click;Wait-For {Panel-Visible} 'open before menu'
    Open-CapsuleMenu;Click-At 25 200;Wait-For { [PanelProbe]::FindMenu($app.Id) -eq [IntPtr]::Zero } 'outside click cancels menu';Pump 200
    Assert-That (Panel-Visible) 'canceling capsule menu preserves open panel'
    # Select the second item: pin; then prove outside clicks no longer dismiss.
    Choose-MenuItem 1 '固定面板'
    Click-At 25 200;Assert-That (Panel-Visible) 'pinned panel survives outside click'
    Choose-MenuItem 1 '取消固定面板'
    Click-At 25 200;Wait-For {-not (Panel-Visible)} 'unpin restores outside dismissal'
    Capsule-Click;Wait-For {Panel-Visible} 'open before renderer rebuild'
    $oldPanel=$script:panel
    Choose-MenuItem 2 '重建面板（用于界面无响应）'
    Wait-For {(Panel-Visible) -and $script:panel -ne $oldPanel} 'rebuild creates a new visible panel'
    Capsule-Click;Wait-For {-not (Panel-Visible)} 'rebuilt panel closes'
    if($VerifyMaterials) {
        Capsule-Click;Wait-For {Panel-Visible} 'open for native material check'
        Verify-Backdrop $script:panel 'floating'
        Capsule-Click;Wait-For {-not (Panel-Visible)} 'close after native material check'
        Choose-MenuItem 3 '打开主窗口'
        Wait-For { $script:main=[PanelProbe]::Find($app.Id,'Aieyes',$false);$script:main -ne [IntPtr]::Zero -and [PanelProbe]::IsWindowVisible($script:main) } 'main window appears'
        Verify-Backdrop $script:main 'main'
    }
    $result.status='passed'
} catch {
    $result.status='failed';$result.reason=$_.ToString()
    if($app){$result.processExited=$app.HasExited;if($app.HasExited){$result.exitCode=$app.ExitCode}}
    Screenshot 'failure';throw
} finally {
    if($background){$background.Close();$background.Dispose()}
    if($app -and -not $app.HasExited){
        & taskkill /PID $app.Id /T /F 2>&1 | Out-Null
        $null=$app.WaitForExit(5000)
    }
    $env:AIEYES_DATA_DIR=$oldData
    $env:WEBVIEW2_USER_DATA_FOLDER=$oldWebviewData
    for($attempt=0;$attempt -lt 20;$attempt++) {
        try {Remove-Item -Recurse -Force $testData;break}
        catch {if($attempt -eq 19){$result.cleanupWarning=$_.ToString();Write-Warning 'Temporary test directory is still locked'}else{Pump 100}}
    }
    $result | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $out 'result.json')
}
