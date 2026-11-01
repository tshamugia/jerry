# Spike #11: empties Windows' standby list (the file cache) on request, so the next Jerry launch
# reads jerry.exe and the WebView2 runtime from disk, as after a reboot. Needs administrator
# rights; it asks for them (one UAC prompt) and serves the startup probe until closed.
#
#   powershell -ExecutionPolicy Bypass -File scripts\spike-11-purge.ps1
#
# The probe (crates/jerry-win/examples/startup_probe.rs, --purge <dir>) writes <dir>\request;
# this script purges and answers in <dir>\done. Emptying the standby list is what RAMMap's
# "Empty Standby List" does: the cache refills as programs read files again.

param([string]$Dir = "$env:TEMP\jerry-spike-11\purge")

$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  Start-Process powershell -Verb RunAs -ArgumentList @(
    "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$PSCommandPath`"", "-Dir", "`"$Dir`"")
  exit
}

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public static class JerryPurge {
  [StructLayout(LayoutKind.Sequential, Pack = 1)]
  struct TokenPrivileges { public int Count; public long Luid; public int Attributes; }

  [DllImport("advapi32.dll", SetLastError = true)]
  static extern bool OpenProcessToken(IntPtr process, int access, out IntPtr token);
  [DllImport("advapi32.dll", SetLastError = true)]
  static extern bool LookupPrivilegeValue(string system, string name, out long luid);
  [DllImport("advapi32.dll", SetLastError = true)]
  static extern bool AdjustTokenPrivileges(IntPtr token, bool disableAll, ref TokenPrivileges state,
    int length, IntPtr previous, IntPtr returnLength);
  [DllImport("kernel32.dll")]
  static extern IntPtr GetCurrentProcess();
  [DllImport("ntdll.dll")]
  static extern int NtSetSystemInformation(int infoClass, ref int info, int length);

  const int SystemMemoryListInformation = 80;
  const int MemoryFlushModifiedList = 3;
  const int MemoryPurgeStandbyList = 4;

  static void Enable(string privilege) {
    IntPtr token;
    if (!OpenProcessToken(GetCurrentProcess(), 0x28, out token)) throw new Exception("OpenProcessToken");
    var state = new TokenPrivileges { Count = 1, Attributes = 2 };
    if (!LookupPrivilegeValue(null, privilege, out state.Luid)) throw new Exception("LookupPrivilegeValue");
    if (!AdjustTokenPrivileges(token, false, ref state, 0, IntPtr.Zero, IntPtr.Zero)) throw new Exception("AdjustTokenPrivileges");
  }

  public static string Purge() {
    Enable("SeProfileSingleProcessPrivilege");
    int command = MemoryFlushModifiedList;
    int status = NtSetSystemInformation(SystemMemoryListInformation, ref command, 4);
    if (status != 0) return "flush failed 0x" + status.ToString("x");
    command = MemoryPurgeStandbyList;
    status = NtSetSystemInformation(SystemMemoryListInformation, ref command, 4);
    return status == 0 ? "ok" : "purge failed 0x" + status.ToString("x");
  }
}
"@

New-Item -ItemType Directory -Force $Dir | Out-Null
$request = Join-Path $Dir "request"
$done = Join-Path $Dir "done"
$stop = Join-Path $Dir "stop"
Write-Host "Jerry spike #11: purging the standby list on request in $Dir. Close this window when done."
while (-not (Test-Path $stop)) {
  if (Test-Path $request) {
    Remove-Item $request -Force
    try { $answer = [JerryPurge]::Purge() } catch { $answer = "error: $_" }
    Set-Content -Path $done -Value $answer -Encoding ascii
    Write-Host "$(Get-Date -Format HH:mm:ss) $answer"
  }
  Start-Sleep -Milliseconds 20
}
Remove-Item $stop -Force
