# Spike #15: runs Explorer's own Copy, Cut or Paste inside explorer.exe, for clip_probe.rs.
#
# Shell automation on an open Explorer window (Shell.Application -> window -> Document) hands out
# objects that live in explorer.exe, so FolderItem(s).InvokeVerb runs Explorer's verb there, the
# same code Ctrl+C / Ctrl+X / Ctrl+V run. Arguments come in environment variables, which carry
# Georgian names intact:
#   JERRY_VERB   copy | cut | paste | close | formats
#   JERRY_DIR    the folder an Explorer window shows (copy, cut: the items' folder; paste: target)
#   JERRY_NAMES  item names separated by '|' (copy, cut)
$ErrorActionPreference = 'Stop'
$verb = $env:JERRY_VERB
$dir = $env:JERRY_DIR
$names = if ($env:JERRY_NAMES) { $env:JERRY_NAMES -split '\|' } else { @() }

if ($verb -eq 'formats') {
  # What another process sees on the clipboard.
  Add-Type -AssemblyName System.Windows.Forms
  $data = [System.Windows.Forms.Clipboard]::GetDataObject()
  if ($data) { $data.GetFormats() -join ', ' }
  exit 0
}

$shell = New-Object -ComObject Shell.Application
$window = $null
for ($i = 0; $i -lt 75 -and -not $window; $i++) {
  $window = @($shell.Windows()) |
    Where-Object { try { $_.Document.Folder.Self.Path -eq $dir } catch { $false } } |
    Select-Object -First 1
  if (-not $window) { Start-Sleep -Milliseconds 200 }
}
if (-not $window) { Write-Error "no Explorer window shows $dir"; exit 2 }
$view = $window.Document
$folder = $view.Folder

if ($verb -eq 'close') {
  $window.Quit()
  exit 0
}

if ($verb -eq 'paste') {
  # The folder's own Paste (its context menu as seen from its parent): paste into it.
  $folder.Self.InvokeVerb('paste')
  exit 0
}

# Copy / Cut: select the items in the window (new files may need a moment to show up in the
# view), then run the verb on the selection, as Ctrl+C / Ctrl+X do. SelectItem quietly does
# nothing for an item the view doesn't show yet, so check the names, not just the count.
function Selected-Exactly {
  $selected = @($view.SelectedItems() | ForEach-Object { $_.Name }) | Sort-Object
  $wanted = @($names | ForEach-Object { $folder.ParseName($_).Name }) | Sort-Object
  ($selected -join '|') -eq ($wanted -join '|')
}
for ($try = 0; $try -lt 25; $try++) {
  $first = $true
  foreach ($name in $names) {
    $item = $folder.ParseName($name)
    if (-not $item) { break }
    # 1 select, 4 deselect all others, 16 focus
    $flags = if ($first) { 1 + 4 + 16 } else { 1 }
    $view.SelectItem($item, $flags)
    $first = $false
  }
  if (Selected-Exactly) { break }
  Start-Sleep -Milliseconds 200
}
if (-not (Selected-Exactly)) { Write-Error "couldn't select $($names -join ', ')"; exit 3 }
$view.SelectedItems().InvokeVerbEx($verb)
