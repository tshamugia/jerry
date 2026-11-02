# Spike #16: runs Windows' real context menu (TrackPopupMenuEx + subclass) through menu_probe for
# every case and checks that the chosen command really ran. No mouse or keyboard: the probe's
# driver thread picks the items with menu messages. Menus pop up briefly in the top-left corner.
#
#   cargo build -p jerry-win --example menu_probe
#   powershell -ExecutionPolicy Bypass -File crates\jerry-win\examples\menu_matrix.ps1 [-Archiver 7-Zip|WinRAR]
#
# The archiver's cascaded submenu is the "7-Zip" criterion; WinRAR stands in when 7-Zip isn't
# installed (both fill their submenu only on WM_INITMENUPOPUP).
param([string]$Archiver = '')
$ErrorActionPreference = 'Stop'
$repo = Resolve-Path "$PSScriptRoot\..\..\.."
$probe = "$repo\target\debug\examples\menu_probe.exe"
$lab = Join-Path $env:TEMP 'jerry-spike-16-matrix'
# Georgian names from code points: Windows PowerShell reads scripts as ANSI without a BOM.
$drawing = -join [char[]](0x10DC, 0x10D0, 0x10EE, 0x10D0, 0x10D6, 0x10D8)          # ნახაზი
$project = -join [char[]](0x10DE, 0x10E0, 0x10DD, 0x10D4, 0x10E5, 0x10E2, 0x10D8)  # პროექტი
$results = New-Object System.Collections.Generic.List[string]

if (-not $Archiver) {
  $Archiver = if (Test-Path 'C:\Program Files\7-Zip\7-zip.dll') { '7-Zip' } elseif (Test-Path 'C:\Program Files\WinRAR\WinRAR.exe') { 'WinRAR' } else { '' }
}

function Check($name, $ok, $detail) {
  $line = "{0} {1}: {2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $detail
  $results.Add($line)
  Write-Host $line
}

# Runs the menu for $paths, picks $choice and waits for $expect (a file, or DIR\*.ext) to appear.
function Menu($name, $choice, $paths, $expect, [switch]$NoForward) {
  $args = @('show', '--prewarm')
  if ($NoForward) { $args += '--no-forward' }
  if ($expect) { $args += @('--wait', $expect) }
  $args += $choice
  $args += $paths
  # Native argument passing in Windows PowerShell drops inner quotes unless escaped.
  $escaped = $args | ForEach-Object { $_ -replace '"', '\"' }
  $out = & $probe @escaped 2>&1 | Out-String
  Get-Process menu_probe -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $probe } | Stop-Process -Force
  $timing = ($out -split "`n" | Where-Object { $_ -match '^resolve' }) -join ''
  $popups = ($out -split "`n" | Where-Object { $_ -match 'WM_INITMENUPOPUP "[^"]' } | ForEach-Object { $_.Trim() }) -join '; '
  $chosen = ($out -split "`n" | Where-Object { $_ -match '^chosen' }) -join ''
  $driver = ($out -split "`n" | Where-Object { $_ -match '^driver' }) -join ''
  # The probe checks the result itself: Test-Path can't see paths past MAX_PATH.
  $found = if ($out -match 'wait: .* -> Some\("(.*)"\)') { $Matches[1] -replace '\\\\', '\' } else { $null }
  [pscustomobject]@{ Name = $name; Out = $out; Timing = $timing.Trim(); Popups = $popups; Chosen = $chosen.Trim(); Driver = $driver.Trim();
    Exists = [bool]$found; Found = $found }
}

# Only the script's own lab; through \\?\, since Remove-Item can't reach its long path.
if (Test-Path $lab) { [IO.Directory]::Delete("\\?\$lab", $true) }
New-Item -ItemType Directory -Force $lab, "$lab\$project", "$lab\other" | Out-Null
[IO.File]::WriteAllText("$lab\$drawing.dwg", 'jerry')
[IO.File]::WriteAllText("$lab\plain.txt", 'jerry')
[IO.File]::WriteAllText("$lab\$project\inner.txt", 'jerry')
[IO.File]::WriteAllText("$lab\other\elsewhere.txt", 'jerry')
# A path past MAX_PATH (created through \\?\: LongPathsEnabled may be off).
$long = $lab
foreach ($i in 1..6) { $long += "\$project-$i-" + ('x' * 40) }
[IO.Directory]::CreateDirectory("\\?\$long") | Out-Null
$longFile = "$long\$drawing-long.txt"
[IO.File]::WriteAllText("\\?\$longFile", 'jerry')
Write-Host "long path: $($longFile.Length) chars"

$zip = 'Send to>Compressed (zipped) folder'

# One file, Georgian name: Send to.
$r = Menu 'one file / Send to' $zip @("$lab\$drawing.dwg") "$lab\$drawing.zip"
Check $r.Name $r.Exists "$($r.Chosen) | $($r.Timing) | $($r.Popups) $($r.Driver)"

# Several files: the zip is named after the first item.
$r = Menu 'several files / Send to' $zip @("$lab\plain.txt", "$lab\$drawing.dwg") "$lab\plain.zip"
$entries = if ($r.Exists) { Add-Type -AssemblyName System.IO.Compression.FileSystem; ([IO.Compression.ZipFile]::OpenRead("$lab\plain.zip").Entries | ForEach-Object Name) -join ', ' } else { '' }
Check $r.Name ($r.Exists -and $entries -match 'plain.txt' -and $entries -match $drawing) "entries: $entries | $($r.Chosen) | $($r.Popups) $($r.Driver)"

# A folder.
$r = Menu 'folder / Send to' $zip @("$lab\$project") "$lab\$project.zip"
Check $r.Name $r.Exists "$($r.Chosen) | $($r.Timing) $($r.Driver)"

# Long path.
$r = Menu 'long path / Send to' $zip @($longFile) "$long\*.zip"
Check $r.Name $r.Exists "created $(Split-Path -Leaf "$($r.Found)") | $($r.Chosen) | $($r.Timing) $($r.Driver)"

if ($Archiver -eq 'WinRAR') {
  $r = Menu 'one file / WinRAR' "WinRAR>Add to `"$drawing.rar`"" @("$lab\$drawing.dwg") "$lab\$drawing.rar"
  Check $r.Name $r.Exists "$($r.Chosen) | $($r.Popups) $($r.Driver)"
  $r = Menu 'several files / WinRAR' "WinRAR>Add to `"jerry-spike-16-matrix.rar`"" @("$lab\plain.txt", "$lab\$drawing.dwg") "$lab\jerry-spike-16-matrix.rar"
  Check $r.Name $r.Exists "$($r.Chosen) | $($r.Popups) $($r.Driver)"
  $r = Menu 'folder / WinRAR' "WinRAR>Add to `"$project.rar`"" @("$lab\$project") "$lab\$project.rar"
  Check $r.Name $r.Exists "$($r.Chosen) | $($r.Popups) $($r.Driver)"
  $r = Menu 'long path / WinRAR' 'WinRAR>Add to "' @($longFile) "$long\*.rar"
  Check $r.Name $r.Exists "created $(Split-Path -Leaf "$($r.Found)") | $($r.Chosen) | $($r.Popups) $($r.Driver)"
  # Without the subclass the cascaded submenu stays an empty placeholder.
  $r = Menu 'no subclass / WinRAR (must fail)' "WinRAR>Add to `"plain.rar`"" @("$lab\plain.txt") $null -NoForward
  Check $r.Name ($r.Driver -match 'no item') "$($r.Driver)"
} elseif ($Archiver -eq '7-Zip') {
  $r = Menu 'one file / 7-Zip' "7-Zip>Add to `"$drawing.7z`"" @("$lab\$drawing.dwg") "$lab\$drawing.7z"
  Check $r.Name $r.Exists "$($r.Chosen) | $($r.Popups) $($r.Driver)"
  $r = Menu 'several files / 7-Zip' "7-Zip>Add to `"jerry-spike-16-matrix.7z`"" @("$lab\plain.txt", "$lab\$drawing.dwg") "$lab\jerry-spike-16-matrix.7z"
  Check $r.Name $r.Exists "$($r.Chosen) | $($r.Popups) $($r.Driver)"
  $r = Menu 'folder / 7-Zip' "7-Zip>Add to `"$project.7z`"" @("$lab\$project") "$lab\$project.7z"
  Check $r.Name $r.Exists "$($r.Chosen) | $($r.Popups) $($r.Driver)"
  $r = Menu 'long path / 7-Zip' '7-Zip>Add to "' @($longFile) "$long\*.7z"
  Check $r.Name $r.Exists "created $(Split-Path -Leaf "$($r.Found)") | $($r.Chosen) | $($r.Popups) $($r.Driver)"
  $r = Menu 'no subclass / 7-Zip' "7-Zip>Add to `"plain.7z`"" @("$lab\plain.txt") "$lab\plain.7z" -NoForward
  Check $r.Name $true "(informational) exists=$($r.Exists) $($r.Driver)"
} else {
  Check 'archiver' $false 'neither 7-Zip nor WinRAR is installed'
}

# Send to without the subclass (informational: Windows 11 fills it in QueryContextMenu).
$r = Menu 'no subclass / Send to' $zip @("$lab\plain.txt") $null -NoForward
Check $r.Name $true "(informational) $($r.Chosen) $($r.Driver)"

# Items of two folders can't share one menu.
$out = & $probe show "$lab\plain.txt" "$lab\other\elsewhere.txt" 2>&1 | Out-String
Check 'mixed folders refused' ($out -match 'different folders') ($out.Trim())

Write-Host ''
$results | ForEach-Object { Write-Host $_ }
