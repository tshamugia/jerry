//! Spike #11: process start times and memory of Jerry and its WebView2 processes.

use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// One process of Jerry's tree.
#[derive(Debug, Clone, Default)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: u32,
    pub name: String,
    /// Creation time, ms since the Unix epoch (the kernel stamps it on the clock tick).
    pub created_unix_ms: f64,
    /// Commit charge (Task Manager's "Commit size").
    pub private_bytes: u64,
    pub working_set: u64,
    /// Task Manager's "Memory" column.
    pub private_working_set: u64,
}

/// Converts a FILETIME (100 ns ticks since 1601) to ms since the Unix epoch.
pub fn filetime_unix_ms(time: FILETIME) -> f64 {
    const UNIX_EPOCH_MS: f64 = 11_644_473_600_000.0;
    let ticks = (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    ticks as f64 / 10_000.0 - UNIX_EPOCH_MS
}

/// When the current process was created, in ms since the Unix epoch.
pub fn current_process_created_unix_ms() -> Option<f64> {
    // SAFETY: the pseudo handle of the current process is always valid and needs no closing.
    created_unix_ms(unsafe { GetCurrentProcess() })
}

fn created_unix_ms(process: HANDLE) -> Option<f64> {
    let (mut created, mut exited, mut kernel, mut user) = Default::default();
    // SAFETY: `process` is a valid process handle; the four out-pointers are valid FILETIMEs.
    unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) }
        .ok()
        .map(|()| filetime_unix_ms(created))
}

/// Closes a handle when dropped.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by this module and is closed only here.
        if let Err(error) = unsafe { CloseHandle(self.0) } {
            tracing::warn!(%error, "CloseHandle failed");
        }
    }
}

/// `root` and every process descended from it (WebView2's browser, GPU, utility and renderer
/// processes for Jerry), with start times and memory. Processes that can't be opened keep zeros.
pub fn process_tree(root: u32) -> Vec<ProcessInfo> {
    // SAFETY: plain snapshot of the process list; the handle is closed by `Owned`.
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return Vec::new();
    };
    let snapshot = Owned(snapshot);
    let mut all = Vec::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: `entry` is a PROCESSENTRY32W with dwSize set, as both calls require.
    let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) }.is_ok();
    while more {
        let len = entry
            .szExeFile
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(entry.szExeFile.len());
        all.push(ProcessInfo {
            pid: entry.th32ProcessID,
            parent_pid: entry.th32ParentProcessID,
            name: String::from_utf16_lossy(&entry.szExeFile[..len]),
            ..Default::default()
        });
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot.0, &mut entry) }.is_ok();
    }
    for info in &mut all {
        fill(info);
    }

    // Walk down from the root. A child must be younger than its parent: parent ids are reused.
    let mut tree: Vec<ProcessInfo> = all.iter().filter(|p| p.pid == root).cloned().collect();
    let mut next = 0;
    while next < tree.len() {
        let parent = tree[next].clone();
        for child in &all {
            if child.parent_pid == parent.pid
                && child.pid != parent.pid
                && child.created_unix_ms >= parent.created_unix_ms
                && !tree.iter().any(|p| p.pid == child.pid)
            {
                tree.push(child.clone());
            }
        }
        next += 1;
    }
    tree
}

fn fill(info: &mut ProcessInfo) {
    // SAFETY: plain open by pid; the handle is closed by `Owned`.
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, info.pid) })
    else {
        return;
    };
    let process = Owned(process);
    info.created_unix_ms = created_unix_ms(process.0).unwrap_or(0.0);
    let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        ..Default::default()
    };
    // SAFETY: `counters` is a PROCESS_MEMORY_COUNTERS_EX2 and `cb` is its size, which the call
    // accepts in place of the base struct.
    let ok = unsafe {
        GetProcessMemoryInfo(
            process.0,
            (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    }
    .is_ok();
    if ok {
        info.private_bytes = counters.PrivateUsage as u64;
        info.working_set = counters.WorkingSetSize as u64;
        info.private_working_set = counters.PrivateWorkingSetSize as u64;
    }
}
