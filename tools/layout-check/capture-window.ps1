# Capture the main window of the HexaDOF desktop application to a PNG.
#
# A headless browser proves the frontend renders; this proves the packaged
# application renders it too, with the real webview and the real security policy.
param(
    [string]$Exe = "target\release\hexadof-desktop.exe",
    [string]$Out = "tools\layout-check\out\app-window.png",
    [int]$WaitSeconds = 12,
    [int[]]$Click = @(),
    [switch]$Attach
)

Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win32Capture {
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, IntPtr extra);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

$process = if ($Attach) {
    # Capture a window that is already open, for example a development session.
    $running = Get-Process -Name ([System.IO.Path]::GetFileNameWithoutExtension($Exe)) -ErrorAction SilentlyContinue |
        Where-Object { $_.MainWindowHandle -ne 0 } |
        Select-Object -First 1
    if (-not $running) { throw "no running window to attach to" }
    $running
} else {
    Start-Process -FilePath $Exe -PassThru
}
Start-Sleep -Seconds $WaitSeconds
$process.Refresh()
if ($process.HasExited) { throw "the application exited with code $($process.ExitCode)" }

$handle = $process.MainWindowHandle
if ($handle -eq [IntPtr]::Zero) {
    # The main window can take a moment to be registered as the process window.
    for ($i = 0; $i -lt 20 -and $handle -eq [IntPtr]::Zero; $i++) {
        Start-Sleep -Milliseconds 500
        $process.Refresh()
        $handle = $process.MainWindowHandle
    }
}
if ($handle -eq [IntPtr]::Zero) { Stop-Process -Id $process.Id -Force; throw "no main window appeared" }

[void][Win32Capture]::SetForegroundWindow($handle)
Start-Sleep -Milliseconds 800

$rect = New-Object Win32Capture+RECT
[void][Win32Capture]::GetWindowRect($handle, [ref]$rect)

# Optional click, in window coordinates, to switch to another screen before the
# capture. The sidebar entries sit at a known offset, so a click is enough.
if ($Click.Count -ge 2) {
    $x = $rect.Left + $Click[0]
    $y = $rect.Top + $Click[1]
    [void][Win32Capture]::SetCursorPos($x, $y)
    Start-Sleep -Milliseconds 200
    [Win32Capture]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
    [Win32Capture]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Seconds 3
}

$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
"window: $width x $height"

$bitmap = New-Object System.Drawing.Bitmap($width, $height)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$hdc = $graphics.GetHdc()
# Flag 2 is PW_RENDERFULLCONTENT, which is what a webview needs.
$ok = [Win32Capture]::PrintWindow($handle, $hdc, 2)
$graphics.ReleaseHdc($hdc)
$graphics.Dispose()

$directory = Split-Path -Parent $Out
if (-not (Test-Path $directory)) { New-Item -ItemType Directory -Path $directory -Force | Out-Null }
$bitmap.Save((Resolve-Path $directory).Path + "\" + (Split-Path -Leaf $Out), [System.Drawing.Imaging.ImageFormat]::Png)
$bitmap.Dispose()

# An attached window belongs to a session that is already running, so it is left
# alone. Only a window this script opened is closed again.
if (-not $Attach) {
    Stop-Process -Id $process.Id -Force
}
"captured: $Out (PrintWindow returned $ok)"
