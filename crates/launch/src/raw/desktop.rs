//! The desktop calls: the desktop's creation and close, a process run on it in a job that ends
//! with the run, and this process's startup desktop and command line.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::Path;

use tracing::warn;
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Environment::GetCommandLineW;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_LIMIT_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, DESKTOP_CONTROL_FLAGS, DESKTOP_CREATEWINDOW, HDESK,
};
use windows::Win32::System::Threading::{
    CREATE_SUSPENDED, CreateProcessW, GetExitCodeProcess, GetStartupInfoW, INFINITE,
    PROCESS_INFORMATION, ResumeThread, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

use super::ENDED_EXIT_CODE;
use crate::desktop::{DesktopError, check_name};

/// `STARTUPINFOW.cb`, the structure's size in bytes.
const STARTUP_INFO_SIZE: u32 = 104;
const _: () = assert!(STARTUP_INFO_SIZE as usize == size_of::<STARTUPINFOW>());

/// The size in bytes of `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`, which `SetInformationJobObject`
/// takes beside the structure.
const EXTENDED_LIMITS_SIZE: u32 = 144;
const _: () =
    assert!(EXTENDED_LIMITS_SIZE as usize == size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>());

/// `ResumeThread`'s result when the call failed.
const RESUME_FAILED: u32 = u32::MAX;

/// A process started on a desktop, and the job that holds it and every process it starts.
struct Started {
    pid: u32,
    process: OwnedHandle,
    /// Closing the job's last handle ends every process still in it.
    job: OwnedHandle,
}

/// A desktop this process created. Dropping it closes this process's handle. Windows removes the
/// desktop once no handle and no process on it is left.
pub struct Desktop {
    name: String,
    handle: HDESK,
}

impl Desktop {
    /// Creates the desktop `name` in this process's window station, or opens it when it exists.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError::BadName`] for an empty name or one with a backslash, and
    /// [`DesktopError::Create`] when Windows cannot create the desktop.
    pub fn create(name: &str) -> Result<Self, DesktopError> {
        check_name(name)?;
        let wide = HSTRING::from(name);
        // SAFETY: the name is a NUL-terminated wide string that outlives the call, the device and
        // the mode are the reserved nulls, and no attributes leave the handle not inheritable.
        let created = unsafe {
            CreateDesktopW(
                &wide,
                PCWSTR::null(),
                None,
                DESKTOP_CONTROL_FLAGS(0),
                DESKTOP_CREATEWINDOW.0,
                None,
            )
        };
        match created {
            Ok(handle) => Ok(Self { name: name.to_owned(), handle }),
            Err(source) => Err(DesktopError::Create { name: name.to_owned(), source }),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Starts `program` on this desktop with `command_line`, the entire command line with the
    /// program name first, waits until it exits and returns its exit code. The process inherits
    /// this process's console, environment and current folder, and no handle.
    ///
    /// The process and every process it starts run in a job that ends them when this call returns
    /// or this process ends, such as on a Ctrl+C. A game with no console would otherwise outlive
    /// the run on a desktop nobody sees.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError::Start`] when the process does not start, [`DesktopError::Job`]
    /// when it cannot join its job, and [`DesktopError::Wait`] or [`DesktopError::ExitCode`] when
    /// its end cannot be read.
    pub fn run(&self, program: &Path, command_line: &OsStr) -> Result<u32, DesktopError> {
        let Started { pid, process, job: _job } = self.start(program, command_line)?;
        let handle = HANDLE(process.as_raw_handle());
        // SAFETY: `handle` is the process handle `process` keeps open during the call.
        let waited = unsafe { WaitForSingleObject(handle, INFINITE) };
        if waited != WAIT_OBJECT_0 {
            let source = windows::core::Error::from_thread();
            return Err(DesktopError::Wait { pid, name: self.name.clone(), source });
        }
        let mut code = 0u32;
        // SAFETY: `handle` is open during the call, and `code` is a writable local.
        match unsafe { GetExitCodeProcess(handle, &raw mut code) } {
            Ok(()) => Ok(code),
            Err(source) => Err(DesktopError::ExitCode { pid, name: self.name.clone(), source }),
        }
    }

    /// Starts `program` suspended on this desktop, puts it in a job that ends with its last
    /// handle, and resumes it.
    fn start(&self, program: &Path, command_line: &OsStr) -> Result<Started, DesktopError> {
        let (pid, process, thread) = self.start_suspended(program, command_line)?;
        let job = match ending_job(&process) {
            Ok(job) => job,
            Err(source) => {
                end_suspended(&process, pid);
                return Err(DesktopError::Job { pid, name: self.name.clone(), source });
            }
        };
        // SAFETY: `thread` is the open handle of the process's first thread, which the start left
        // suspended.
        if unsafe { ResumeThread(HANDLE(thread.as_raw_handle())) } == RESUME_FAILED {
            // The return drops the job, and the job's close ends the suspended process.
            let source = windows::core::Error::from_thread();
            let name = self.name.clone();
            return Err(DesktopError::Start { program: program.to_path_buf(), name, source });
        }
        Ok(Started { pid, process, job })
    }

    /// Starts `program` on this desktop with its first thread suspended, and returns its process
    /// id, its process handle and the handle of that thread.
    fn start_suspended(
        &self,
        program: &Path,
        command_line: &OsStr,
    ) -> Result<(u32, OwnedHandle, OwnedHandle), DesktopError> {
        let application = HSTRING::from(program);
        // CreateProcessW may write into the command line, so it gets a buffer of its own.
        let mut line: Vec<u16> = command_line.encode_wide().chain([0]).collect();
        let mut desktop: Vec<u16> = self.name.encode_utf16().chain([0]).collect();
        let startup = STARTUPINFOW {
            cb: STARTUP_INFO_SIZE,
            lpDesktop: PWSTR(desktop.as_mut_ptr()),
            ..STARTUPINFOW::default()
        };
        let mut started = PROCESS_INFORMATION::default();
        // SAFETY: the program, the command line and the desktop name are NUL-terminated wide
        // strings that outlive the call, the command line is writable, `startup` holds its size,
        // and `started` is a writable local. No handle is inherited.
        let result = unsafe {
            CreateProcessW(
                &application,
                Some(PWSTR(line.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_SUSPENDED,
                None,
                PCWSTR::null(),
                &raw const startup,
                &raw mut started,
            )
        };
        if let Err(source) = result {
            let name = self.name.clone();
            return Err(DesktopError::Start { program: program.to_path_buf(), name, source });
        }
        // SAFETY: the start opened the thread handle, and only the returned owner closes it.
        let thread = unsafe { OwnedHandle::from_raw_handle(started.hThread.0) };
        // SAFETY: the start opened the process handle, and only the returned owner closes it.
        let process = unsafe { OwnedHandle::from_raw_handle(started.hProcess.0) };
        Ok((started.dwProcessId, process, thread))
    }
}

/// A new job that holds `process` and ends every process in it once its last handle closes. A
/// process that `process` starts joins the job.
fn ending_job(process: &OwnedHandle) -> Result<OwnedHandle, windows::core::Error> {
    // SAFETY: no attributes and no name create an unnamed job whose handle is not inheritable.
    let created = unsafe { CreateJobObjectW(None, PCWSTR::null()) }?;
    // SAFETY: the create opened the job handle, and only the returned owner closes it.
    let job = unsafe { OwnedHandle::from_raw_handle(created.0) };
    let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
            LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            ..JOBOBJECT_BASIC_LIMIT_INFORMATION::default()
        },
        ..JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default()
    };
    let job_handle = HANDLE(job.as_raw_handle());
    // SAFETY: the job handle is open, and `limits` is a structure of the stated size that
    // outlives the call.
    unsafe {
        SetInformationJobObject(
            job_handle,
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            EXTENDED_LIMITS_SIZE,
        )
    }?;
    // SAFETY: both handles are open during the call, and the process handle has all access.
    unsafe { AssignProcessToJobObject(job_handle, HANDLE(process.as_raw_handle())) }?;
    Ok(job)
}

/// Ends the suspended process `pid`, which no job holds.
fn end_suspended(process: &OwnedHandle, pid: u32) {
    let handle = HANDLE(process.as_raw_handle());
    // SAFETY: the process handle is open during the call and has all access.
    if let Err(error) = unsafe { TerminateProcess(handle, ENDED_EXIT_CODE) } {
        warn!(pid, error = %error, "the suspended process did not end");
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        // SAFETY: `handle` is the desktop handle `create` opened, and only this drop closes it.
        if let Err(error) = unsafe { CloseDesktop(self.handle) } {
            warn!(desktop = %self.name, error = %error, "the desktop handle did not close");
        }
    }
}

/// The desktop name in this process's `STARTUPINFO`, which may lead with a window station name, or
/// `None` when the process's start named no desktop.
pub(crate) fn startup_desktop() -> Option<String> {
    let mut startup = STARTUPINFOW { cb: STARTUP_INFO_SIZE, ..STARTUPINFOW::default() };
    // SAFETY: `startup` is a writable STARTUPINFOW that holds its size.
    unsafe { GetStartupInfoW(&raw mut startup) };
    if startup.lpDesktop.is_null() {
        return None;
    }
    // SAFETY: a desktop name that is not null is a NUL-terminated wide string in this process's
    // start parameters, which last as long as the process.
    let units = unsafe { startup.lpDesktop.as_wide() };
    Some(String::from_utf16_lossy(units))
}

/// This process's entire command line as Windows holds it.
pub(crate) fn own_command_line() -> OsString {
    // SAFETY: GetCommandLineW has no arguments.
    let line = unsafe { GetCommandLineW() };
    // SAFETY: the command line is a NUL-terminated wide string that lasts as long as the process.
    OsString::from_wide(unsafe { line.as_wide() })
}
