# Spike #15: drives the real jerry.exe through the clipboard with Explorer, without touching the
# keyboard or the mouse. Jerry's buttons and rows are clicked through UI Automation (WebView2
# exposes the page to it), Explorer's verbs run in explorer.exe (explorer_verb.ps1), and the
# results are read from the clipboard, the folders and %TEMP%\jerry-spike-15.log.
#
# Overwrites the clipboard and opens an Explorer window. Run by hand after
# `pnpm tauri build --no-bundle`:
#   powershell -ExecutionPolicy Bypass -File src-tauri\examples\clipboard_e2e.ps1
$ErrorActionPreference = 'Stop'
$repo = Resolve-Path "$PSScriptRoot\..\.."
$exe = "$repo\target\release\jerry.exe"
$verbScript = "$repo\crates\jerry-win\examples\explorer_verb.ps1"
$sample = Join-Path $env:TEMP 'jerry-spike-15'
$work = Join-Path $env:TEMP 'jerry-spike-15-e2e'
$log = Join-Path $env:TEMP 'jerry-spike-15.log'
# "ქართული" from code points: Windows PowerShell reads scripts as ANSI without a BOM.
$ka = -join [char[]](0x10E5, 0x10D0, 0x10E0, 0x10D7, 0x10E3, 0x10DA, 0x10D8)

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms
$AE = [System.Windows.Automation.AutomationElement]
$Scope = [System.Windows.Automation.TreeScope]
$results = New-Object System.Collections.Generic.List[string]

function Check($name, $ok, $detail) {
  $line = "{0} {1}: {2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $detail
  $results.Add($line)
  Write-Host $line
}

function Explorer($verb, $dir, $names = @()) {
  $env:JERRY_VERB = $verb; $env:JERRY_DIR = $dir; $env:JERRY_NAMES = ($names -join '|')
  $out = powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $verbScript
  if ($LASTEXITCODE -ne 0) { throw "explorer_verb $verb failed: $out" }
  $out
}

function WaitFor($what, [scriptblock]$condition, $seconds = 15) {
  $deadline = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $deadline) {
    if (& $condition) { return $true }
    Start-Sleep -Milliseconds 100
  }
  Write-Host "  (timed out waiting for $what)"
  $false
}

function ClipFiles { @([System.Windows.Forms.Clipboard]::GetFileDropList()) }

function ClipEffect {
  $data = [System.Windows.Forms.Clipboard]::GetDataObject()
  if (-not $data -or -not $data.GetDataPresent('Preferred DropEffect')) { return $null }
  $stream = $data.GetData('Preferred DropEffect')
  $stream.Position = 0
  $bytes = New-Object byte[] 4
  [void]$stream.Read($bytes, 0, 4)
  [BitConverter]::ToInt32($bytes, 0)
}

function Element($name) {
  $condition = New-Object System.Windows.Automation.PropertyCondition($AE::NameProperty, $name)
  for ($i = 0; $i -lt 50; $i++) {
    $element = $script:window.FindFirst($Scope::Descendants, $condition)
    if ($element) { return $element }
    Start-Sleep -Milliseconds 200
  }
  throw "no element named '$name' in Jerry"
}

# A plain click on a button or a row, through its UI Automation pattern.
function Click($name) {
  $element = Element $name
  $pattern = $null
  if ($element.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
    $pattern.Invoke()
  } elseif ($element.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$pattern)) {
    $pattern.Select()
  } else {
    throw "can't click '$name'"
  }
  Start-Sleep -Milliseconds 300
}

function LogSince($offset) {
  # Jerry keeps the log open for writing.
  $stream = [System.IO.FileStream]::new($log, 'Open', 'Read', 'ReadWrite')
  $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
  $text = $reader.ReadToEnd()
  $reader.Close()
  $text.Substring([Math]::Min($offset, $text.Length))
}

# Fresh folders, a fresh log offset, and an Explorer window on the work folder.
# Only this checkout's exe: other worktrees may have their own Jerry running.
Get-Process jerry -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe } | Stop-Process -Force
Remove-Item -Recurse -Force $sample, $work -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $work | Out-Null
$logStart = if (Test-Path $log) { (Get-Item $log).Length } else { 0 }
Start-Process explorer.exe $work

$jerry = Start-Process $exe -PassThru
# The process also has tao's hidden event-target window; we want the one named Jerry.
$pidCondition = New-Object System.Windows.Automation.AndCondition(
  (New-Object System.Windows.Automation.PropertyCondition($AE::ProcessIdProperty, $jerry.Id)),
  (New-Object System.Windows.Automation.PropertyCondition($AE::NameProperty, 'Jerry')))
$script:window = $null
[void](WaitFor 'the Jerry window' { $script:window = $AE::RootElement.FindFirst($Scope::Children, $pidCondition); $script:window })
$report = 'Report Q3 (final) [v2].pdf'
[void](Element $report)

function Items($dir) { @(Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue) }

# Opens Explorer on a new subfolder of the work folder and runs Explorer's Paste there.
function PasteInExplorer($name) {
  $dir = "$work\$name"
  New-Item -ItemType Directory -Force $dir | Out-Null
  Start-Process explorer.exe $dir
  [void](Explorer 'paste' $dir)
  $dir
}

# 1. Jerry Copy -> Explorer Paste
Click 'Select all (Ctrl+A)'
Click 'Copy (Ctrl+C)'
$files = ClipFiles
$seeds = (Items $sample).Count
Check 'Jerry Copy puts the files on the clipboard' (($files.Count -eq $seeds) -and ($files -contains "$sample\$report") -and ((ClipEffect) -eq 5)) "$($files.Count) of $seeds files, Preferred DropEffect $(ClipEffect)"
$target = PasteInExplorer 'copy'
$ok = WaitFor 'the pasted copy' { (Items $target).Count -eq $seeds }
Check 'Explorer pastes Jerry''s copy' ($ok -and ((Items $sample).Count -eq $seeds)) "in Explorer: $((Items $target).Count), still in Jerry: $((Items $sample).Count)"
Explorer 'close' $target | Out-Null

# 2. Jerry Cut -> Explorer Paste
Click 'Refresh'
Click 'Select all (Ctrl+A)'
Click 'Cut (Ctrl+X)'
Check 'Jerry Cut marks them MOVE' ((ClipEffect) -eq 2) "Preferred DropEffect $(ClipEffect)"
$beforePaste = (Get-Item $log).Length
$target = PasteInExplorer 'cut'
$ok = WaitFor 'the move' { ((Items $target).Count -eq $seeds) -and ((Items $sample).Count -eq 0) }
Start-Sleep -Milliseconds 800
$changed = LogSince $beforePaste
Check 'Explorer moves Jerry''s cut; Jerry hears the clipboard change' ($ok -and ($changed -match 'clipboard changed.*owner: Other.*cut_ids: \[\]')) "moved: $ok, in Explorer: $((Items $target).Count), left in Jerry: $((Items $sample).Count), files on the clipboard now: $((ClipFiles).Count)"
Explorer 'close' $target | Out-Null

# 3. Explorer Copy -> Jerry Paste
$copyName = "$ka copy.txt"
Set-Content -LiteralPath "$work\$copyName" -Value 'from Explorer' -Encoding UTF8
[void](Explorer 'copy' $work @($copyName))
[void](WaitFor 'Explorer''s copy on the clipboard' { (ClipFiles) -contains "$work\$copyName" })
Click 'Paste (Ctrl+V)'
$ok = WaitFor 'Jerry''s paste' { Test-Path -LiteralPath "$sample\$copyName" }
Check 'Jerry pastes Explorer''s copy' ($ok -and (Test-Path -LiteralPath "$work\$copyName")) "in Jerry: $ok, still in Explorer's folder: $(Test-Path -LiteralPath "$work\$copyName")"

# 4. Explorer Cut -> Jerry Paste
$cutName = "$ka cut.txt"
Set-Content -LiteralPath "$work\$cutName" -Value 'cut in Explorer' -Encoding UTF8
[void](Explorer 'cut' $work @($cutName))
[void](WaitFor 'Explorer''s cut on the clipboard' { ((ClipFiles) -contains "$work\$cutName") -and ((ClipEffect) -eq 2) })
Click 'Paste (Ctrl+V)'
$ok = WaitFor 'Jerry''s move' { (Test-Path -LiteralPath "$sample\$cutName") -and -not (Test-Path -LiteralPath "$work\$cutName") }
Start-Sleep -Milliseconds 500
Check 'Jerry moves Explorer''s cut and empties the clipboard' ($ok -and (ClipFiles).Count -eq 0) "moved: $ok, files on the clipboard now: $((ClipFiles).Count)"
Explorer 'close' $work | Out-Null

# 5. Jerry Copy, close Jerry -> Explorer Paste
Click 'Refresh'
Click 'Select all (Ctrl+A)'
Click 'Copy (Ctrl+C)'
$count = (Items $sample).Count
Click 'Close'
$exited = $jerry.WaitForExit(10000)
$files = ClipFiles
Check 'Jerry''s files stay on the clipboard after it exits' ($exited -and ($files.Count -eq $count) -and ($files -contains "$sample\$copyName")) "exited: $exited, $($files.Count) of $count files, Preferred DropEffect $(ClipEffect)"
$target = PasteInExplorer 'after-exit'
$ok = WaitFor 'the paste after exit' { (Items $target).Count -eq $count }
Check 'Explorer pastes them with Jerry gone' $ok "pasted: $((Items $target).Count) of $count"
Explorer 'close' $target | Out-Null

Write-Host ''
Write-Host 'Jerry log (this run):'
(LogSince $logStart) -split "`n" | Where-Object { $_ -match 'copied|pasting|pasted|clipboard at exit' } | ForEach-Object { '  ' + ($_.Trim() -replace '^\S+\s+', '') }
Write-Host ''
$results | ForEach-Object { Write-Host $_ }
