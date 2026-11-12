//! Programs of hers that go when she goes. Cursor's agent kept as a server doesn't quit when she does (closing its
//! input isn't enough), and each of her restarts left one running.

use std::sync::OnceLock;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

/// Her job object, open for as long as she runs: Windows ends everything in it when she exits, however she exits.
struct Job(HANDLE);
// The handle is only ever passed to Windows, which may be called from any thread.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

fn job() -> Option<&'static Job> {
    static JOB: OnceLock<Option<Job>> = OnceLock::new();
    JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(None, None).ok()?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(job, JobObjectExtendedLimitInformation, &limits as *const _ as *const _, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32).ok()?;
        Some(Job(job))
    })
    .as_ref()
}

/// The program `pid` (and whatever it starts after this) ends when she does.
pub fn end_with_me(pid: u32) -> Result<()> {
    let job = job().context("no job object")?;
    unsafe {
        let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid)?;
        let done = AssignProcessToJobObject(job.0, process);
        let _ = CloseHandle(process);
        done?;
    }
    Ok(())
}
