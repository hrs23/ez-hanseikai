use std::io::Read;
use std::net::{TcpListener, TcpStream};
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

pub fn wait_until(wait: Duration, poll: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(poll);
    }
    false
}

pub fn accept(listener: &TcpListener, keep_going: impl Fn() -> bool) -> Option<TcpStream> {
    listener.set_nonblocking(true).ok()?;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream.set_nonblocking(false).ok().map(|_| stream),
            Err(_) if keep_going() => std::thread::sleep(POLL),
            Err(_) => return None,
        }
    }
}

/// Windows PowerShell with its own module path: a path inherited from PowerShell 7 (an app started from pwsh) hides built-in commands such as Get-FileHash.
pub fn powershell(args: &[&str]) -> Command {
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-NonInteractive"]).args(args).env_remove("PSModulePath");
    command
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

#[cfg(test)]
mod tests {
    use super::*;

    // cargo test runs from pwsh in GitHub Actions, so this inherits PowerShell 7's module path there.
    #[test]
    fn powershell_finds_its_built_in_commands_when_started_from_powershell_7() {
        let mut command = powershell(&["-Command", "Get-FileHash -LiteralPath $env:ComSpec -Algorithm SHA256 | Out-Null; Get-CimInstance Win32_OperatingSystem | Out-Null"]);
        assert!(command.get_envs().any(|(k, v)| k == "PSModulePath" && v.is_none()));
        assert_eq!(run(&mut command, 0, Duration::from_secs(60)), Ok(String::new()));
    }
}
