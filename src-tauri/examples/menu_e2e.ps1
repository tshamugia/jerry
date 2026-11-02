# Spike #16: drives the real jerry.exe through Windows' context menu without touching the
# keyboard or the mouse. Jerry's case buttons are pressed through UI Automation (WebView2 exposes
# the page to it); the menu is driven by menu_probe (MN_SELECTITEM + WM_KEYDOWN posted to the
# menu window); results come from the files the commands create and %TEMP%\jerry-spike-16.log.
#
# Menus and a Properties window pop up briefly. Jerry is shown without taking the focus
# (JERRY_SPIKE_NOACTIVATE). Run by hand after
#   pnpm tauri build --no-bundle
#   cargo build -p jerry-win --example menu_probe
#   powershell -ExecutionPolicy Bypass -File src-tauri\examples\menu_e2e.ps1 [-Archiver 7-Zip|WinRAR]
param([string]$Archiver = '')
$ErrorActionPreference = 'Stop'
$repo = Resolve-Path "$PSScriptRoot\..\.."
$exe = "$repo\target\release\jerry.exe"
$probe = "$repo\target\debug\examples\menu_probe.exe"
$lab = Join-Path $env:TEMP 'jerry-spike-16'
$log = Join-Path $env:TEMP 'jerry-spike-16.log'
# Georgian names from code points: Windows PowerShell reads scripts as ANSI without a BOM.
$drawing = (-join [char[]](0x10DC, 0x10D0, 0x10EE, 0x10D0, 0x10D6, 0x10D8)) + ' ' + [char]0x2014 + ' ' +
  (-join [char[]](0x10E1, 0x10D0, 0x10E0, 0x10D7, 0x10E3, 0x10DA, 0x10D8)) + ' 1'           # ნახაზი — სართული 1
$project = -join [char[]](0x10DE, 0x10E0, 0x10DD, 0x10D4, 0x10E5, 0x10E2, 0x10D8)          # პროექტი

if (-not $Archiver) {
  $Archiver = if (Test-Path 'C:\Program Files\7-Zip\7-zip.dll') { '7-Zip' } elseif (Test-Path 'C:\Program Files\WinRAR\WinRAR.exe') { 'WinRAR' } else { '' }
}
$ext = @{ '7-Zip' = '7z'; 'WinRAR' = 'rar' }[$Archiver]

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Win {
  public delegate bool EnumProc(IntPtr hwnd, IntPtr data);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc proc, IntPtr data);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hwnd, System.Text.StringBuilder text, int max);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
}
'@
$AE = [System.Windows.Automation.AutomationElement]
$Scope = [System.Windows.Automation.TreeScope]
$results = New-Object System.Collections.Generic.List[string]

function Check($name, $ok, $detail) {
  $line = "{0} {1}: {2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $detail
  $results.Add($line)
  Write-Host $line
}

function WaitFor($what, [scriptblock]$condition, $seconds = 20) {
  $deadline = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $deadline) {
    if (& $condition) { return $true }
    Start-Sleep -Milliseconds 100
  }
  Write-Host "  (timed out waiting for $what)"
  $false
}

# Paths past MAX_PATH: Test-Path can't see them, .NET with \\?\ can.
function Exists($path) { [IO.File]::Exists("\\?\$path") -or [IO.Directory]::Exists("\\?\$path") }
function NewFile($dir, $pattern) {
  if (-not [IO.Directory]::Exists("\\?\$dir")) { return $null }
  [IO.Directory]::EnumerateFiles("\\?\$dir", $pattern) | Select-Object -First 1
}

function LogLines {
  if (-not (Test-Path $log)) { return @() }
  @([IO.File]::ReadAllLines($log, [Text.Encoding]::UTF8))
}

function StopJerry {
  Get-Process jerry -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe } | Stop-Process -Force
}

# Starts this checkout's jerry.exe (others may run their own) and finds its window.
function StartJerry($envs = @{}) {
  StopJerry
  $env:JERRY_SPIKE_NOACTIVATE = '1'
  foreach ($key in $envs.Keys) { Set-Item "env:$key" $envs[$key] }
  $script:jerry = Start-Process $exe -PassThru
  foreach ($key in $envs.Keys) { Remove-Item "env:$key" }
  $pidCondition = New-Object System.Windows.Automation.AndCondition(
    (New-Object System.Windows.Automation.PropertyCondition($AE::ProcessIdProperty, $script:jerry.Id)),
    (New-Object System.Windows.Automation.PropertyCondition($AE::NameProperty, 'Jerry')))
  $script:window = $null
  [void](WaitFor 'the Jerry window' { $script:window = $AE::RootElement.FindFirst($Scope::Children, $pidCondition); $script:window })
  [void](Element 'One file')
}

function Element($name) {
  $condition = New-Object System.Windows.Automation.PropertyCondition($AE::NameProperty, $name)
  for ($i = 0; $i -lt 100; $i++) {
    $element = $script:window.FindFirst($Scope::Descendants, $condition)
    if ($element -and $element.Current.IsEnabled) { return $element }
    Start-Sleep -Milliseconds 100
  }
  throw "no enabled element named '$name' in Jerry"
}

function Press($name) {
  $pattern = (Element $name).GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
  $pattern.Invoke()
}

# Presses a case button, picks $choice in the menu Jerry shows, returns Jerry's report line.
function Menu($case, $choice) {
  $before = (LogLines).Count
  Press $case
  # Native argument passing in Windows PowerShell drops inner quotes unless escaped.
  $out = & $probe drive --pid $script:jerry.Id ($choice -replace '"', '\"') 2>&1 | Out-String
  [void](WaitFor 'the report' { (LogLines).Count -gt $before } 30)
  $line = (LogLines | Select-Object -Skip $before | Select-Object -First 1)
  [pscustomobject]@{ Drive = $out.Trim(); Line = "$line" }
}

function Field($line, $name) {
  if ($line -match "$name`: ([^,}]+(\([^)]*\))?)") { $Matches[1] } else { '?' }
}

function Summary($r) {
  $l = $r.Line
  "chosen=$(Field $l 'chosen') query=$(Field $l 'query_ms') ms ack_during_menu=$(Field $l 'ack_during_menu') ack=$(Field $l 'ack_ms') prewarm=$(Field $l 'prewarm_ms') init_popup=$(Field $l 'init_popup') | $($r.Drive)"
}

# Fresh lab (only the script's own folder in %TEMP%; through \\?\ for its long path).
StopJerry
if (Test-Path $lab) { [IO.Directory]::Delete("\\?\$lab", $true) }
StartJerry
# The long item's folder, as spike.rs seeds it: six levels of "ნახაზები-N-xxxx…".
$drawings = -join [char[]](0x10DC, 0x10D0, 0x10EE, 0x10D0, 0x10D6, 0x10D4, 0x10D1, 0x10D8)
$longDir = $lab
foreach ($level in 1..6) { $longDir += "\$drawings-$level-" + ('x' * 40) }
Write-Host "Jerry pid $($script:jerry.Id), long folder $($longDir.Length) chars, exists $(Exists $longDir)"

$zip = 'Send to>Compressed (zipped) folder'

# The first menu after start: Send to on the Georgian file.
$r = Menu 'One file' $zip
$ok = WaitFor 'the zip' { Exists "$lab\$drawing.zip" }
Check 'one file (Georgian) / Send to' ($ok -and $r.Line -match 'ack_during_menu: true') (Summary $r)

$r = Menu 'Folder' $zip
$ok = WaitFor 'the zip' { Exists "$lab\$project.zip" }
Check 'folder / Send to' ($ok -and $r.Line -match 'ack_during_menu: true') (Summary $r)

$r = Menu 'Long path' $zip
$ok = WaitFor 'the zip' { NewFile $longDir '*.zip' }
Check 'long path / Send to' $ok "created $(Split-Path -Leaf "$(NewFile $longDir '*.zip')") | $(Summary $r)"

if ($ext) {
  $r = Menu 'One file' "$Archiver>Add to `"$drawing.$ext`""
  $ok = WaitFor 'the archive' { Exists "$lab\$drawing.$ext" }
  Check "one file / $Archiver" ($ok -and $r.Line -match 'ack_during_menu: true') (Summary $r)

  $r = Menu 'Several files' "$Archiver>Add to `"jerry-spike-16.$ext`""
  $ok = WaitFor 'the archive' { Exists "$lab\jerry-spike-16.$ext" }
  Check "several files / $Archiver" $ok (Summary $r)

  $r = Menu 'Folder' "$Archiver>Add to `"$project.$ext`""
  $ok = WaitFor 'the archive' { Exists "$lab\$project.$ext" }
  Check "folder / $Archiver" $ok (Summary $r)

  $r = Menu 'Long path' "$Archiver>Add to `""
  $ok = WaitFor 'the archive' { NewFile $longDir "*.$ext" }
  Check "long path / $Archiver" $ok "created $(Split-Path -Leaf "$(NewFile $longDir "*.$ext")") | $(Summary $r)"
} else {
  Check 'archiver' $false 'neither 7-Zip nor WinRAR is installed'
}

# A dialog verb: Properties opens Windows' own window in Jerry's process.
$r = Menu 'One file' 'Properties'
$found = [IntPtr]::Zero
$ok = WaitFor 'the Properties window' {
  $script:props = [IntPtr]::Zero
  [void][Win]::EnumWindows({ param($h, $d)
      $p = 0; [void][Win]::GetWindowThreadProcessId($h, [ref]$p)
      $t = New-Object System.Text.StringBuilder 256; [void][Win]::GetWindowText($h, $t, 256)
      if ($p -eq $script:jerry.Id -and [Win]::IsWindowVisible($h) -and $t.ToString() -like '*Properties*') { $script:props = $h }
      $true }, [IntPtr]::Zero)
  $script:props -ne [IntPtr]::Zero
}
if ($ok) { [void][Win]::PostMessage($script:props, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) } # WM_CLOSE
Check 'Properties dialog' ($ok -and $r.Line -match 'verb: Some\("properties"\)') (Summary $r)

# Jerry's own verb: returned to Jerry, not invoked.
$r = Menu 'One file' 'Rename'
Check 'Rename stays Jerry''s' ($r.Line -match 'verb: Some\("rename"\)' -and $r.Line -match 'invoked: false' -and (Exists "$lab\$drawing.dwg")) (Summary $r)

# Esc: nothing chosen.
$r = Menu 'One file' 'ESC'
Check 'Esc closes without a choice' ($r.Line -match 'chosen: None') (Summary $r)

# Mixed folders: refused before any menu, with an explanation on the page.
Press 'Mixed folders'
$alert = New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)
$ok = WaitFor 'the explanation' { $script:window.FindAll($Scope::Descendants, $alert) | Where-Object { $_.Current.Name -match 'different folders' } }
Check 'mixed folders refused' $ok ''

# Through Tauri's run_on_main_thread: the page's answer only arrives after the menu closed.
StartJerry @{ JERRY_SPIKE_TAO = '1' }
$r = Menu 'Several files' 'ESC'
Check 'tao callback holds events (expected: ack_during_menu false)' ($r.Line -match 'via: "tao"' -and $r.Line -match 'ack_during_menu: false') (Summary $r)

# Without the startup prewarm: the first menu's QueryContextMenu, for comparison.
StartJerry @{ JERRY_SPIKE_NO_PREWARM = '1' }
$r = Menu 'One file' 'ESC'
Check 'first menu without prewarm (informational)' $true (Summary $r)
StopJerry

Write-Host ''
$results | ForEach-Object { Write-Host $_ }
