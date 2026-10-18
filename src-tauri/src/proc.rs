use std::io::Read;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

pub const NO_WINDOW: u32 = 0x08000000;
pub const BELOW_NORMAL: u32 = 0x00004000;
pub const IDLE: u32 = 0x00000040;
const POLL: Duration = Duration::from_millis(20);

struct Job(isize);

impl Job {
    fn new() -> Option<Self> {
        let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.ok()?;
        let job = Job(handle.0 as isize);
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
        unsafe { SetInformationJobObject(job.handle(), JobObjectExtendedLimitInformation, &info as *const _ as *const _, size) }.ok()?;
        Some(job)
    }

    fn handle(&self) -> HANDLE {
        HANDLE(self.0 as *mut _)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.handle()) };
    }
}

pub struct Process {
    pub child: Child,
    job: Option<Job>,
}

impl Process {
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn kill(&mut self) {
        if let Some(job) = &self.job {
            let _ = unsafe { TerminateJobObject(job.handle(), 1) };
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    pub fn wait_for(&mut self, wait: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + wait;
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                self.kill();
                return None;
            }
            std::thread::sleep(POLL);
        }
    }
}

pub fn spawn(command: &mut Command, flags: u32) -> std::io::Result<Process> {
    let child = command.creation_flags(NO_WINDOW | flags).spawn()?;
    let job = Job::new().filter(|job| unsafe { AssignProcessToJobObject(job.handle(), HANDLE(child.as_raw_handle())) }.is_ok());
    Ok(Process { child, job })
}

pub fn run(command: &mut Command, flags: u32, wait: Duration) -> Result<String, String> {
    let mut process = spawn(command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()), flags).map_err(|e| e.to_string())?;
    let mut stderr = process.child.stderr.take().unwrap();
    let errors = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).trim().to_string()
    });
    let status = process.wait_for(wait);
    let text = errors.join().unwrap_or_default();
    match status {
        Some(s) if s.success() => Ok(text),
        Some(s) => Err(if text.is_empty() { format!("stopped with {s}") } else { text }),
        None => Err(format!("did not finish in {} s", wait.as_secs())),
    }
}
