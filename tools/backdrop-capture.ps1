# Copyright © Kentaro Inomata
# SPDX-License-Identifier: MIT-0
#
# Shows a probe over a magenta backdrop window of its own at the right edge of the primary screen,
# captures only the probe's rectangle, and reports whether the window lets the backdrop through.
# It never captures the rest of the desktop.
#
# Usage: tools/backdrop-capture.ps1 -Exe <probe.exe> -Out <png> [-Arguments ...] [-Env @{NAME="value"}]
# Run from PowerShell. Prints the extended style, whether the window has a menu, and sample pixels:
# magenta (#FF00FF) means the window lets what is behind it through.
param(
    [Parameter(Mandatory)] [string] $Exe,
    [Parameter(Mandatory)] [string] $Out,
    [hashtable] $Env = @{},
    [string[]] $Arguments = @(),
    [int] $WaitMs = 2000
)
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class CC {
  [StructLayout(LayoutKind.Sequential)] public struct R { public int l,t,r,b; }
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern int GetWindowLongW(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern IntPtr GetMenu(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out R r, int s);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  // The largest visible top-level window of the process.
  public static IntPtr Largest(uint pid) {
    IntPtr best = IntPtr.Zero; long area = 0;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      R r;
      if (p == pid && IsWindowVisible(h) && GetWindowRect(h, out r)) {
        long a = (long)(r.r - r.l) * (r.b - r.t);
        if (a > area) { area = a; best = h; }
      }
      return true;
    }, IntPtr.Zero);
    return best;
  }
}
'@
[CC]::SetProcessDpiAwarenessContext([IntPtr]-4) | Out-Null  # per-monitor v2: physical pixels

$screen = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
$backdrop = New-Object System.Windows.Forms.Form
$backdrop.FormBorderStyle = 'None'; $backdrop.BackColor = [System.Drawing.Color]::Magenta
$backdrop.StartPosition = 'Manual'; $backdrop.ShowInTaskbar = $false
$bw = 1100; $bh = 900
$backdrop.Bounds = New-Object System.Drawing.Rectangle(($screen.Right - $bw), ($screen.Top + 40), $bw, $bh)
$backdrop.Show(); [System.Windows.Forms.Application]::DoEvents()

foreach ($k in $Env.Keys) { [Environment]::SetEnvironmentVariable($k, $Env[$k]) }
# In this console: a console program started in a new one loses the foreground to the terminal
# that opens for it, and an inactive window shows Mica without the wallpaper's tint.
$proc = if ($Arguments.Count) { Start-Process -FilePath $Exe -ArgumentList $Arguments -PassThru -NoNewWindow } else { Start-Process -FilePath $Exe -PassThru -NoNewWindow }
foreach ($k in $Env.Keys) { [Environment]::SetEnvironmentVariable($k, $null) }
$h = [IntPtr]::Zero
for ($i = 0; $i -lt 100; $i++) {
    Start-Sleep -Milliseconds 100
    $h = [CC]::Largest([uint32]$proc.Id)
    $rr = New-Object CC+R
    if ($h -ne [IntPtr]::Zero -and [CC]::GetWindowRect($h, [ref]$rr) -and ($rr.r - $rr.l) -gt 100) { break }
    $h = [IntPtr]::Zero
}
if ($h -eq [IntPtr]::Zero) { $backdrop.Close(); throw "no window" }
# Inside the backdrop, above it.
# Against the right edge of the screen, away from desktop icons and other windows.
$wr = New-Object CC+R; [CC]::GetWindowRect($h, [ref]$wr) | Out-Null
[CC]::SetWindowPos($h, [IntPtr]::Zero, ($screen.Right - ($wr.r - $wr.l) - 40), ($screen.Top + 100), 0, 0, 0x0001) | Out-Null
# The backdrop right behind the probe, so that nothing else sits between them.
[CC]::SetWindowPos($backdrop.Handle, $h, 0, 0, 0, 0, 0x0001 -bor 0x0002 -bor 0x0010) | Out-Null
Start-Sleep -Milliseconds $WaitMs
[System.Windows.Forms.Application]::DoEvents()

$r = New-Object CC+R
[CC]::DwmGetWindowAttribute($h, 9, [ref]$r, 16) | Out-Null   # DWMWA_EXTENDED_FRAME_BOUNDS
$w = $r.r - $r.l; $hh = $r.b - $r.t
$bmp = New-Object System.Drawing.Bitmap($w, $hh)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.l, $r.t, 0, 0, $bmp.Size)
$bmp.Save($Out)

function Px($x, $y) { $c = $bmp.GetPixel($x, $y); '#{0:X2}{1:X2}{2:X2}' -f $c.R, $c.G, $c.B }
$ex = [CC]::GetWindowLongW($h, -20) -band 0xFFFFFFFF
[pscustomobject]@{
    exe = Split-Path $Exe -Leaf
    noRedirection = [bool]($ex -band 0x00200000)
    hasMenu = ([CC]::GetMenu($h) -ne [IntPtr]::Zero)
    size = "${w}x${hh}"
    topLeft = Px 2 2; topRight = Px ($w - 3) 2; bottomLeft = Px 2 ($hh - 3); bottomRight = Px ($w - 3) ($hh - 3)
    center = Px ([int]($w / 2)) ([int]($hh / 2))
    lowerRight = Px ([int]($w * 3 / 4)) ([int]($hh * 3 / 4))
}
$g.Dispose(); $bmp.Dispose()
[CC]::PostMessageW($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null  # WM_CLOSE
$proc.WaitForExit(5000) | Out-Null
$backdrop.Close()
