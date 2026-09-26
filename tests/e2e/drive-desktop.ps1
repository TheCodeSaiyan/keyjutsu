param(
    [Parameter(Mandatory)] [string] $Action,
    [string] $Arg = "",
    [string] $Out = ""
)
# Drives the KeyJutsu desktop window for end-to-end checks: screenshots,
# mouse clicks and real keystrokes via SendKeys.
#
# SendKeys and mouse clicks go to whatever window is in the foreground, and
# Windows refuses SetForegroundWindow while someone is using another app. So
# every action checks that KeyJutsu really is in front and stops otherwise.
# Without this check, keys meant for KeyJutsu were once typed into a browser.
# Run it only on a machine nobody is using at the same time.
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms, System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class W {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
[void][W]::SetProcessDPIAware()

$proc = Get-Process keyjutsu-desktop -ErrorAction Stop | Where-Object MainWindowHandle -ne 0 | Select-Object -First 1
$hwnd = $proc.MainWindowHandle
$root = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)

function Focus {
  # Inside Windows Sandbox only, where nobody else is at the keyboard: a lone
  # Alt press lifts Windows' lock on changing the foreground, which a newly
  # started window is otherwise often refused. Never on a real desktop, where
  # the key would go to whatever someone is using.
  if ($env:USERNAME -eq 'WDAGUtilityAccount' -and [W]::GetForegroundWindow() -ne $hwnd) {
    Add-Type -MemberDefinition '[DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, int f, int e);' -Name K -Namespace U
    [U.K]::keybd_event(0x12, 0, 0, 0); [U.K]::keybd_event(0x12, 0, 2, 0)
  }
  [void][W]::ShowWindow($hwnd, 9); [void][W]::SetForegroundWindow($hwnd); Start-Sleep -Milliseconds 300
  if ([W]::GetForegroundWindow() -ne $hwnd) {
    throw "KeyJutsu is not the foreground window; refusing to send input or capture the screen"
  }
}

switch ($Action) {
  "capture" {
    # Read-only: asks the window to draw itself into a bitmap. Needs no focus
    # and sends no input, so it is safe while someone is using the machine.
    $r = New-Object W+RECT; [void][W]::GetWindowRect($hwnd, [ref]$r)
    $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    [void][W]::PrintWindow($hwnd, $hdc, 2) # PW_RENDERFULLCONTENT, needed for WebView2
    $g.ReleaseHdc($hdc)
    $bmp.Save($Out); "captured $Out ($($bmp.Width)x$($bmp.Height))"
  }
  "shot" {
    $r = New-Object W+RECT; [void][W]::GetWindowRect($hwnd, [ref]$r)
    $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    Focus
    $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
    $bmp.Save($Out); "saved $Out ($($bmp.Width)x$($bmp.Height))"
  }
  "click" {
    # WebView2 builds its accessibility tree on the first query, so the first
    # search often finds nothing: ask again before giving up.
    $cond = New-Object System.Windows.Automation.PropertyCondition ([System.Windows.Automation.AutomationElement]::NameProperty), $Arg
    # A heading or label can share a control's name ("Terminal", "Profile"):
    # prefer the button or drop-down.
    $controls = @('ControlType.Button', 'ControlType.ComboBox', 'ControlType.MenuItem')
    $el = $null
    foreach ($try in 1..5) {
      $all = @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond))
      if ($all.Count -eq 0) {
        # A label styled in capitals is reported in capitals ("ARM KEYJUTSU"),
        # and a list entry's name runs its lines together: match ignoring
        # case, then by the start of the name.
        $every = @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition))
        $all = @($every | Where-Object { $_.Current.Name -ieq $Arg })
        if ($all.Count -eq 0) { $all = @($every | Where-Object { $_.Current.Name -like "$Arg*" }) }
      }
      $el = @($all | Where-Object { $controls -contains $_.Current.ControlType.ProgrammaticName }) + $all | Select-Object -First 1
      if ($el) { break }
      Start-Sleep -Seconds 1
    }
    if (-not $el) { throw "no element named '$Arg'" }
    Focus
    $invoke = $null
    if ($el.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$invoke)) {
      $invoke.Invoke(); "invoked $Arg"
    } else {
      # A drop-down has nothing to invoke: click the middle of it instead.
      $b = $el.Current.BoundingRectangle
      [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point ([int]($b.X + $b.Width / 2)), ([int]($b.Y + $b.Height / 2))
      Add-Type -MemberDefinition '[DllImport("user32.dll")] public static extern void mouse_event(int f, int x, int y, int d, int e);' -Name M -Namespace U
      [U.M]::mouse_event(2, 0, 0, 0, 0); [U.M]::mouse_event(4, 0, 0, 0, 0); "clicked $Arg"
    }
  }
  "select" {
    $cond = New-Object System.Windows.Automation.PropertyCondition ([System.Windows.Automation.AutomationElement]::NameProperty), $Arg
    $el = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
    if (-not $el) { throw "no element named '$Arg'" }
    Focus
    $el.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select(); "selected $Arg"
  }
  "keys" {
    Focus
    foreach ($k in $Arg.ToCharArray()) {
      $t = if ('+^%~(){}[]'.Contains($k)) { "{$k}" } else { [string]$k }
      [System.Windows.Forms.SendKeys]::SendWait($t); Start-Sleep -Milliseconds 25
    }
    "sent $($Arg.Length) keys"
  }
  "chord" { Focus; [System.Windows.Forms.SendKeys]::SendWait($Arg); "sent chord $Arg" }
  "mouse" {
    # $Arg is "x,y" relative to the window's top-left corner.
    $r = New-Object W+RECT; [void][W]::GetWindowRect($hwnd, [ref]$r)
    $x, $y = $Arg.Split(",") | ForEach-Object { [int]$_ }
    Focus
    [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point ($r.L + $x), ($r.T + $y)
    Add-Type -MemberDefinition '[DllImport("user32.dll")] public static extern void mouse_event(int f, int x, int y, int d, int e);' -Name M -Namespace U
    [U.M]::mouse_event(2, 0, 0, 0, 0); [U.M]::mouse_event(4, 0, 0, 0, 0); "clicked $Arg"
  }
  "title" { (Get-Process -Id $proc.Id).MainWindowTitle }
  "has" {
    # Read-only: whether an element with this name is on screen now. Exits 0
    # if it is and 3 if not; sends no input.
    $cond = New-Object System.Windows.Automation.PropertyCondition ([System.Windows.Automation.AutomationElement]::NameProperty), $Arg
    $found = $null
    foreach ($try in 1..2) {
      $found = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
      if ($found) { break }
      Start-Sleep -Seconds 1
    }
    if ($found) { "present $Arg"; exit 0 } else { "absent $Arg"; exit 3 }
  }
}
