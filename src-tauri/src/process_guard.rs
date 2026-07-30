use std::process::Child;

pub struct ChildJob {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

// Windows kernel handles can be transferred between threads. ChildJob owns the
// handle and only closes it from Drop, so moving the guard into an async task is safe.
#[cfg(windows)]
unsafe impl Send for ChildJob {}

impl ChildJob {
    pub fn attach(child: &mut Child) -> Result<Self, ()> {
        #[cfg(windows)]
        {
            attach_windows(child)
        }

        #[cfg(not(windows))]
        {
            let _ = child;
            Ok(Self {})
        }
    }
}

#[cfg(windows)]
fn attach_windows(child: &mut Child) -> Result<ChildJob, ()> {
    use std::{mem::size_of, os::windows::io::AsRawHandle};
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
    };

    unsafe {
        let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if handle.is_null() {
            terminate(child);
            return Err(());
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) != 0;
        let assigned =
            configured && AssignProcessToJobObject(handle, child.as_raw_handle() as _) != 0;
        if !assigned {
            CloseHandle(handle);
            terminate(child);
            return Err(());
        }

        Ok(ChildJob { handle })
    }
}

#[cfg(windows)]
fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

impl Drop for ChildJob {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            if !self.handle.is_null() {
                windows_sys::Win32::Foundation::CloseHandle(self.handle);
            }
        }
    }
}
