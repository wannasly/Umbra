//! Windows Job Object that binds the sing-box core's lifetime to ours.
//!
//! `Command::kill_on_drop(true)` only helps while the parent unwinds normally.
//! `TerminateProcess` — Task Manager's "End task", a hard crash, a forced
//! session end — runs no destructor at all, so the core used to outlive the GUI
//! and keep both the tunnel and the registry proxy alive with nobody owning
//! them.
//!
//! A job object created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` moves the
//! guarantee into the kernel: every process assigned to the job is terminated
//! as soon as the *last handle* to the job closes, and Windows closes our
//! handle when our process dies, whatever killed it. The handle is therefore
//! never closed by us and never handed out — it is a process-global that lives
//! for the whole app session.

/// Assign a freshly spawned child to the app-wide kill-on-close job.
///
/// Best-effort: a failure only costs the hard-kill guarantee, so callers log
/// and carry on rather than failing the connect.
#[cfg(windows)]
pub fn assign_current_job(raw_handle: std::os::windows::io::RawHandle) -> Result<(), String> {
    imp::assign(raw_handle)
}

#[cfg(not(windows))]
pub fn assign_current_job(_raw_handle: *mut std::ffi::c_void) -> Result<(), String> {
    Err("job objects are a Windows feature".into())
}

#[cfg(windows)]
mod imp {
    use std::os::windows::io::RawHandle;
    use std::sync::OnceLock;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// Owns the job handle for the life of the process. Never closed on
    /// purpose: closing it is exactly what kills the core, so the only close
    /// we want is the implicit one Windows performs when we die.
    struct Job(HANDLE);

    // A job object handle is a plain kernel handle with no thread affinity;
    // sharing the raw value across threads is what the API is designed for.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    static JOB: OnceLock<Option<Job>> = OnceLock::new();

    fn create() -> Result<Job, String> {
        // Unnamed job: a name would let a second instance (or anything else on
        // the machine) open a handle and keep the job alive past our death.
        let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
            .map_err(|e| format!("CreateJobObjectW failed: {e}"))?;

        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(|e| format!("SetInformationJobObject failed: {e}"))?;

        Ok(Job(handle))
    }

    fn job() -> Option<HANDLE> {
        JOB.get_or_init(|| match create() {
            Ok(job) => Some(job),
            Err(e) => {
                eprintln!("[umbra] {e}; sing-box will not be killed on a hard parent kill");
                None
            }
        })
        .as_ref()
        .map(|job| job.0)
    }

    pub fn assign(raw_handle: RawHandle) -> Result<(), String> {
        let job = job().ok_or_else(|| "job object unavailable".to_string())?;
        // Windows 8+ supports nested jobs, so this still works when the app
        // itself was launched inside somebody else's job (an installer, a CI
        // harness, the Windows Terminal shell job).
        unsafe { AssignProcessToJobObject(job, HANDLE(raw_handle.cast())) }
            .map_err(|e| format!("AssignProcessToJobObject failed: {e}"))
    }

    #[cfg(test)]
    pub fn probe_handle() -> Option<usize> {
        job().map(|h| h.0 as usize)
    }

    /// Read the limit flags back off the live job object.
    #[cfg(test)]
    pub fn probe_limit_flags() -> Option<u32> {
        use windows::Win32::System::JobObjects::QueryInformationJobObject;
        let handle = job()?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        unsafe {
            QueryInformationJobObject(
                Some(handle),
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of_mut!(info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                None,
            )
        }
        .ok()?;
        Some(info.BasicLimitInformation.LimitFlags.0)
    }

    /// Ask the kernel whether a process is a member of *our* job.
    #[cfg(test)]
    pub fn is_member(raw_handle: RawHandle) -> bool {
        use windows::Win32::System::JobObjects::IsProcessInJob;
        let Some(job) = job() else { return false };
        let mut result = windows::core::BOOL(0);
        unsafe {
            IsProcessInJob(
                HANDLE(raw_handle.cast()),
                Some(job),
                std::ptr::addr_of_mut!(result),
            )
        }
        .is_ok()
            && result.as_bool()
    }
}

#[cfg(all(test, windows))]
mod tests {
    use windows::Win32::System::JobObjects::JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

    /// The job is created lazily and cached. A second job object would be a
    /// silent disaster: it carries KILL_ON_JOB_CLOSE, so letting one go out of
    /// scope would terminate whatever had been assigned to it.
    #[test]
    fn job_handle_is_created_once_and_reused() {
        let first = super::imp::probe_handle();
        let second = super::imp::probe_handle();
        assert!(first.is_some(), "creating a job object must succeed");
        assert_eq!(
            first, second,
            "the job handle must be a process-wide singleton"
        );
    }

    /// The whole point of the job: read the limit back off the kernel object
    /// rather than trusting the struct we filled in.
    #[test]
    fn job_object_carries_kill_on_job_close() {
        let flags = super::imp::probe_limit_flags().expect("query limits");
        assert_eq!(
            flags & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE.0,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE.0,
            "the core would survive a TerminateProcess on the parent"
        );
    }

    /// End-to-end on the real code path: a live child assigned through
    /// `assign_current_job` must actually be a member of the job, which is what
    /// makes Windows terminate it when our last handle closes. Stops short of
    /// killing the parent — that part is verified by hand.
    #[test]
    fn a_spawned_child_becomes_a_member_of_the_job() {
        use std::os::windows::io::AsRawHandle;
        use std::process::{Command, Stdio};

        // ~9s of doing nothing, long enough to assign and inspect.
        let child = Command::new("cmd.exe")
            .args(["/C", "ping", "-n", "10", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else {
            return; // no cmd.exe: nothing meaningful to assert
        };
        let raw = child.as_raw_handle();
        assert!(
            !super::imp::is_member(raw),
            "a fresh child must start outside the job"
        );
        let assigned = super::assign_current_job(raw);
        assert!(assigned.is_ok(), "assign failed: {assigned:?}");
        assert!(
            super::imp::is_member(raw),
            "the child is not in the job, so a hard parent kill would leave it running"
        );
        let _ = child.kill();
        let _ = child.wait();
    }
}
