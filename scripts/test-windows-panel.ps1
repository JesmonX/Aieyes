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
  [DllImport("user32.dll",EntryPoint="GetWindowLongW")] public static extern int GetWindowLong(IntPtr h,int index);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h,uint command);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int width,int height,uint flags);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("shell32.dll")] public static extern int SHQueryUserNotificationState(out int state);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h,uint flags);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint x,uint y,uint data,UIntPtr extra);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h,int attribute,out int value,int size);
  [DllImport("dwmapi.dll",EntryPoint="DwmGetWindowAttribute")] public static extern int DwmGetFrameBounds(IntPtr h,int attribute,out Rect value,int size);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h,uint message,IntPtr w,IntPtr l);
  [DllImport("user32.dll")] public static extern bool GetMenuItemRect(IntPtr owner,IntPtr menu,uint item,out Rect rect);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetMenuString(IntPtr menu,uint item,StringBuilder text,int size,uint flags);
  [DllImport("user32.dll")] public static extern int GetMenuItemCount(IntPtr menu);
  [DllImport("user32.dll")] public static extern uint GetMenuState(IntPtr menu,uint item,uint flags);
  [DllImport("user32.dll")] public static extern IntPtr OpenInputDesktop(uint flags,bool inherit,uint access);
  [DllImport("user32.dll")] public static extern bool CloseDesktop(IntPtr desktop);
  [DllImport("user32.dll")] public static extern bool SwitchDesktop(IntPtr desktop);
  public static bool InputDesktopAvailable() {
    var desktop=OpenInputDesktop(0,false,0x100);
    if(desktop==IntPtr.Zero)return false;
    bool available=SwitchDesktop(desktop);CloseDesktop(desktop);return available;
  }
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
if (-not [Environment]::UserInteractive -or -not [PanelProbe]::InputDesktopAvailable()) {
    $result.status='not-run'; $result.reason='Windows runner has no accessible input desktop (locked or non-interactive)'
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
function Pet-IsTopmost { return ([PanelProbe]::GetWindowLong($script:pet,-20) -band 8) -ne 0 }
function Pet-IsAboveBackground {
    # GW_HWNDNEXT walks down the actual Z order; styles alone do not prove
    # that another application's window cannot cover the pet.
    $window=[PanelProbe]::GetWindow($script:pet,2)
    while($window -ne [IntPtr]::Zero) {
        if($window -eq $background.Handle){return $true}
        $window=[PanelProbe]::GetWindow($window,2)
    }
    return $false
}
function Verify-PetTopmost {
    Assert-That (Pet-IsTopmost) 'pet starts topmost'
    foreach($after in @(-2,1)) {
        $before=Bounds $script:pet
        # HWND_NOTOPMOST / HWND_BOTTOM reproduce losing the topmost band.
        # SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE preserves input/geometry.
        Assert-That ([PanelProbe]::SetWindowPos($script:pet,[IntPtr]::new($after),0,0,0,0,0x13)) "simulate pet demotion ($after)"
        Assert-That (Pet-IsTopmost) "pet prevents demotion before the next painted frame ($after)"
        Assert-That (Pet-IsAboveBackground) "pet remains above ordinary windows ($after)"
        Assert-That ([PanelProbe]::GetForegroundWindow() -eq $background.Handle) "topmost recovery preserves keyboard focus ($after)"
        $afterRect=Bounds $script:pet
        Assert-That ($before.Left -eq $afterRect.Left -and $before.Top -eq $afterRect.Top -and $before.Right -eq $afterRect.Right -and $before.Bottom -eq $afterRect.Bottom) "topmost recovery preserves pet geometry ($after)"
    }
    Screenshot 'pet-topmost-recovered'
}
function Verify-QuietDesktop {
    # Explicit monitor bounds make this fullscreen, independently of how WinForms
    # maximizes a borderless form on a particular Windows/.NET version.
    $before=Bounds $script:pet
    $fullscreen=New-Object System.Windows.Forms.Form
    $screen=[System.Windows.Forms.Screen]::FromHandle($script:pet)
    $fullscreen.Text='Aieyes isolated fullscreen app';$fullscreen.FormBorderStyle='None';$fullscreen.StartPosition='Manual';$fullscreen.Bounds=$screen.Bounds;$fullscreen.BackColor=[Drawing.Color]::FromArgb(20,24,31)
    try {
        $fullscreen.Show();$fullscreen.Activate();$null=[PanelProbe]::SetForegroundWindow($fullscreen.Handle)
        Wait-For { [PanelProbe]::GetForegroundWindow() -eq $fullscreen.Handle } 'fullscreen fixture has keyboard focus' 3
        $rect=Bounds $fullscreen.Handle
        $state=0;$hr=[PanelProbe]::SHQueryUserNotificationState([ref]$state)
        $result.fullscreen=@{bounds=@($rect.Left,$rect.Top,$rect.Right,$rect.Bottom);monitor=@($screen.Bounds.Left,$screen.Bounds.Top,$screen.Bounds.Right,$screen.Bounds.Bottom);notificationState=$state;notificationHResult=$hr}
        Assert-That ($rect.Left -eq $screen.Bounds.Left -and $rect.Top -eq $screen.Bounds.Top -and $rect.Right -eq $screen.Bounds.Right -and $rect.Bottom -eq $screen.Bounds.Bottom) 'fullscreen fixture covers the monitor including taskbar'
        Screenshot 'fullscreen'
        Wait-For { -not [PanelProbe]::IsWindowVisible($script:pet) } 'pet steps aside for fullscreen app' 8
        Assert-That (-not [PanelProbe]::IsWindowVisible($script:pet)) 'fullscreen app hides the pet'
        # The same foreground application occupying only the work area is not fullscreen.
        $fullscreen.Bounds=$screen.WorkingArea
        Wait-For { [PanelProbe]::IsWindowVisible($script:pet) } 'pet returns for ordinary work-area window' 8
        Assert-That ([PanelProbe]::GetForegroundWindow() -eq $fullscreen.Handle) 'leaving fullscreen preserves application focus'
    } finally {$fullscreen.Close();$fullscreen.Dispose()}
    $null=[PanelProbe]::SetForegroundWindow($background.Handle)
    Wait-For { [PanelProbe]::IsWindowVisible($script:pet) } 'pet returns after fullscreen app' 8
    Assert-That ((Pet-IsTopmost) -and (Pet-IsAboveBackground)) 'pet returns above ordinary windows'
    Assert-That ([PanelProbe]::GetForegroundWindow() -eq $background.Handle) 'returning pet preserves keyboard focus'
    $after=Bounds $script:pet
    Assert-That ($before.Left -eq $after.Left -and $before.Top -eq $after.Top -and $before.Right -eq $after.Right -and $before.Bottom -eq $after.Bottom) 'returning pet preserves geometry'
}
function Pet-Click([bool]$Right=$false) {
    $r=Bounds $script:pet;Click-At ($r.Left+($r.Right-$r.Left)/2) ($r.Bottom-11*[PanelProbe]::GetDpiForWindow($script:pet)/96) $Right
}
function Open-PetMenu {
    Pet-Click $true
    Wait-For { [PanelProbe]::FindMenu($app.Id) -ne [IntPtr]::Zero } 'pet menu appears'
}
function Choose-MenuItem([uint32]$Index,[string]$Expected) {
    Open-PetMenu
    $popup=[PanelProbe]::FindMenu($app.Id)
    # MN_GETHMENU obtains the actual popup menu; do not assume keyboard activation
    # of a WS_EX_NOACTIVATE pet or a fixed menu row height on different DPI.
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
function Verify-PetMenu {
    Open-PetMenu
    $popup=[PanelProbe]::FindMenu($app.Id)
    $menu=[PanelProbe]::SendMessage($popup,0x1e1,[IntPtr]::Zero,[IntPtr]::Zero)
    $expected=@('打开面板','设置…','桌宠设置','','检查更新','隐藏桌宠')
    Assert-That ([PanelProbe]::GetMenuItemCount($menu) -eq $expected.Count) 'native menu matches macOS entry count'
    for($index=0;$index -lt $expected.Count;$index++) {
        $label=New-Object Text.StringBuilder 256
        $null=[PanelProbe]::GetMenuString($menu,$index,$label,256,0x400)
        Assert-That ($label.ToString() -eq $expected[$index]) "native menu entry ${index}: $($expected[$index])"
    }
    $script:canHidePet=([PanelProbe]::GetMenuState($menu,5,0x400) -band 3) -eq 0
    Click-At 25 200
    Wait-For { [PanelProbe]::FindMenu($app.Id) -eq [IntPtr]::Zero } 'outside click cancels menu'
    Pump 200
}
function Panel-Visible { $script:panel=[PanelProbe]::Find($app.Id,'Aieyes 面板',$false);return $script:panel -ne [IntPtr]::Zero -and [PanelProbe]::IsWindowVisible($script:panel) }
function Panel-HasFocus {
    $foreground=[PanelProbe]::GetForegroundWindow()
    return $foreground -ne [IntPtr]::Zero -and [PanelProbe]::GetAncestor($foreground,2) -eq [PanelProbe]::GetAncestor($script:panel,2)
}
function Screenshot([string]$Name) {
    $bounds=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bitmap=New-Object System.Drawing.Bitmap $bounds.Width,$bounds.Height
    $graphics=[System.Drawing.Graphics]::FromImage($bitmap)
    try {$graphics.CopyFromScreen($bounds.Location,[System.Drawing.Point]::Empty,$bounds.Size);$bitmap.Save((Join-Path $out "$Name.png"))}
    finally {$graphics.Dispose();$bitmap.Dispose()}
}
# Opt-in on an interactive Windows desktop with transparency enabled. This checks
# actual changing desktop pixels, which WebView screenshots cannot validate.
# Build 22621+ gives the panel Desktop Acrylic (DWMSBT_TRANSIENTWINDOW, 3), which blurs the windows behind it,
# and the main window Mica (DWMSBT_MAINWINDOW, 2), which samples only the wallpaper. Older builds blur both.
function Verify-Backdrop([IntPtr]$Handle,[string]$Name,[int]$BackdropType) {
    $showsBehind=$true
    if([Environment]::OSVersion.Version.Build -ge 22621) {
        $actual=0;$null=[PanelProbe]::DwmGetWindowAttribute($Handle,38,[ref]$actual,4)
        Assert-That ($actual -eq $BackdropType) "$Name requests DWM backdrop type $BackdropType"
        $showsBehind=$BackdropType -ne 2
    }
    # The visible frame: GetWindowRect also covers the invisible resize border, where the desktop always shows.
    $rect=New-Object PanelProbe+Rect;$null=[PanelProbe]::DwmGetFrameBounds($Handle,9,[ref]$rect,16)
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
        if($showsBehind){Assert-That ($total -gt 0 -and $changed/$total -gt .25) "$Name visibly responds to desktop background changes"}
        else{Assert-That ($total -gt 0 -and $changed/$total -lt .05) "$Name keeps windows behind it out of the backdrop"}
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
$resolvedTestData=[IO.Path]::GetFullPath($testData)
if(-not $resolvedTestData.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()),[StringComparison]::OrdinalIgnoreCase) -or (Split-Path $resolvedTestData -Leaf) -notlike 'aieyes-panel-*'){throw 'Unexpected test directory'}
$app=$null;$background=$null
try {
    $startup=[Diagnostics.Stopwatch]::StartNew()
    $app=Start-Process -FilePath (Resolve-Path $Binary) -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $out 'app-stdout.log') -RedirectStandardError (Join-Path $out 'app-stderr.log')
    # A fresh runner initializes both WebViews before creating the pet.
    # Give cold startup its own budget; keep interaction deadlines at 15 seconds.
    Wait-For { $script:pet=[PanelProbe]::Find($app.Id,'AieyesNativePet',$true);$script:pet -ne [IntPtr]::Zero -and [PanelProbe]::IsWindowVisible($script:pet) } 'native pet startup' 60
    $result.startupMilliseconds=$startup.ElapsedMilliseconds
    $result.dpi=[PanelProbe]::GetDpiForWindow($script:pet)
    $expectedHeight=168*$result.dpi/96
    Wait-For { $r=Bounds $script:pet; [Math]::Abs(($r.Bottom-$r.Top)-$expectedHeight) -le 2 } 'DPI-scaled pet dimensions'
    Screenshot 'pet-startup'
    # The hidden main window is revealed once its page lays out, or by the native fallback 3 s after setup.
    # Either may land after the pet appears; close it first so it cannot take the background fixture's focus.
    Wait-For { $script:main=[PanelProbe]::Find($app.Id,'Aieyes',$false);$script:main -ne [IntPtr]::Zero -and [PanelProbe]::IsWindowVisible($script:main) } 'main window startup reveal'
    $null=[PanelProbe]::SendMessage($script:main,0x10,[IntPtr]::Zero,[IntPtr]::Zero)
    Wait-For { -not [PanelProbe]::IsWindowVisible($script:main) } 'closing the startup window retains background app'
    $background=New-Object System.Windows.Forms.Form
    # An ordinary application filling the work area; covering the taskbar too would make it a fullscreen app the pet steps aside for.
    $background.Text='Aieyes isolated background input target';$background.FormBorderStyle='None';$background.StartPosition='Manual';$background.Bounds=[System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea;$background.BackColor=[Drawing.Color]::FromArgb(49,67,83)
    $background.Show();$background.Activate();Pump 300
    $null=[PanelProbe]::SetForegroundWindow($background.Handle)
    Click-At 25 200
    Wait-For { [PanelProbe]::GetForegroundWindow() -eq $background.Handle } 'background focus established before pet click'
    Verify-PetTopmost
    Verify-QuietDesktop
    Pet-Click
    Wait-For {Panel-Visible} 'open panel'
    Assert-That ([PanelProbe]::GetForegroundWindow() -eq $background.Handle) 'opening preserves background keyboard focus'
    $exStyle=[PanelProbe]::GetWindowLong($script:panel,-20)
    # WS_EX_TOOLWINDOW (0x80) without WS_EX_APPWINDOW (0x40000): no taskbar button or Alt+Tab entry.
    Assert-That ((($exStyle -band 0x80) -ne 0) -and (($exStyle -band 0x40000) -eq 0)) 'panel is a tool window outside the taskbar and Alt+Tab'
    Assert-That (([PanelProbe]::GetWindowLong($script:panel,-20) -band 8) -ne 0) 'pet panel opens topmost'
    foreach($after in @(-2,1)) {
        Assert-That ([PanelProbe]::SetWindowPos($script:panel,[IntPtr]::new($after),0,0,0,0,0x13)) "simulate panel demotion ($after)"
        Assert-That (([PanelProbe]::GetWindowLong($script:panel,-20) -band 8) -ne 0) "panel prevents demotion before repaint ($after)"
        Assert-That ([PanelProbe]::GetForegroundWindow() -eq $background.Handle) "panel topmost protection preserves keyboard focus ($after)"
    }
    Screenshot 'open'
    $former=Bounds $script:panel
    # Deliberate interaction can activate the panel (enabling text/keyboard input).
    Click-At ($former.Left+200) ($former.Top+90)
    Wait-For {Panel-HasFocus} 'panel interaction acquires keyboard focus' 3
    Assert-That (Panel-HasFocus) 'panel interaction can acquire keyboard focus'
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
      $different=0;$total=0;$expected=$background.BackColor.ToArgb();$petRect=Bounds $script:pet
      for($x=$former.Left;$x -lt $former.Right;$x+=4) {for($y=$former.Top;$y -lt $former.Bottom;$y+=4) {
        # The pet deliberately remains visible, including its own shadow.
        if($x -ge $petRect.Left-8 -and $x -le $petRect.Right+8 -and $y -ge $petRect.Top-8 -and $y -le $petRect.Bottom+8){continue}
        if($x -ge 0 -and $y -ge 0 -and $x -lt $bitmap.Width -and $y -lt $bitmap.Height){$total++;if($bitmap.GetPixel($x,$y).ToArgb() -ne $expected){$different++}}
      }}
      $result.pixelSamples=$total;$result.differentPixels=$different
      Assert-That ($total -gt 0 -and $different/$total -lt .005) 'closed panel leaves no visible surface or frame'
    } finally {$bitmap.Dispose()}
    for($i=0;$i -lt 12;$i++) {Pet-Click;Wait-For {Panel-Visible} 'repeat open';Pet-Click;Wait-For {-not (Panel-Visible)} 'repeat close'}
    Assert-That (-not (Panel-Visible)) 'repeated pet toggles end fully hidden'
    Pet-Click;Wait-For {Panel-Visible} 'open before menu'
    Verify-PetMenu
    Assert-That (Panel-Visible) 'canceling pet menu preserves open panel'
    Pet-Click;Wait-For {-not (Panel-Visible)} 'close before menu open action'
    Choose-MenuItem 0 '打开面板'
    Wait-For {Panel-Visible} 'native menu opens panel'
    Pet-Click;Wait-For {-not (Panel-Visible)} 'menu-opened panel closes'
    foreach($entry in @(@(1,'设置…'),@(2,'桌宠设置'))) {
        Choose-MenuItem ([uint32]$entry[0]) ([string]$entry[1])
        Wait-For { $script:main=[PanelProbe]::Find($app.Id,'Aieyes',$false);$script:main -ne [IntPtr]::Zero -and [PanelProbe]::IsWindowVisible($script:main) } 'menu settings opens main window'
        $null=[PanelProbe]::SendMessage($script:main,0x10,[IntPtr]::Zero,[IntPtr]::Zero)
        Wait-For { -not [PanelProbe]::IsWindowVisible($script:main) } 'closing settings retains background app'
    }
    if($VerifyMaterials) {
        Pet-Click;Wait-For {Panel-Visible} 'open for native material check'
        # A pet click leaves keyboard focus in the background app; DWM would draw an inactive panel flat.
        Assert-That (-not (Panel-HasFocus)) 'floating material check runs on the inactive panel'
        Verify-Backdrop $script:panel 'floating' 3
        Pet-Click;Wait-For {-not (Panel-Visible)} 'close after native material check'
        Choose-MenuItem 1 '设置…'
        Wait-For { $script:main=[PanelProbe]::Find($app.Id,'Aieyes',$false);$script:main -ne [IntPtr]::Zero -and [PanelProbe]::IsWindowVisible($script:main) } 'main window appears'
        Verify-Backdrop $script:main 'main' 2
    }
    if($script:canHidePet) {
        Choose-MenuItem 5 '隐藏桌宠'
        Wait-For { -not [PanelProbe]::IsWindowVisible($script:pet) } 'native menu hides pet when tray is available'
        Assert-That (-not (Panel-Visible)) 'hiding pet also closes its panel'
        Assert-That ([PanelProbe]::SetWindowPos($script:pet,[IntPtr]::new(1),0,0,0,0,0x13)) 'demote hidden pet'
        Pump 200
        Assert-That (-not [PanelProbe]::IsWindowVisible($script:pet)) 'topmost recovery does not reveal a hidden pet'
    } else {
        Assert-That ([PanelProbe]::IsWindowVisible($script:pet)) 'sole pet entry remains visible when hiding is disabled'
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
        try {Remove-Item -LiteralPath $resolvedTestData -Recurse -Force;break}
        catch {if($attempt -eq 19){$result.cleanupWarning=$_.ToString();Write-Warning 'Temporary test directory is still locked'}else{Pump 100}}
    }
    $result | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $out 'result.json')
}
