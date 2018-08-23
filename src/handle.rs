use std::io;
use std::cmp;
use std::ptr;

use winapi::shared::minwindef::*;
use winapi::shared::ntdef::HANDLE;
use winapi::shared::winerror::*;
use winapi::um::fileapi::*;
use winapi::um::handleapi::*;
use winapi::um::minwinbase::*;

#[derive(Debug)]
pub struct Handle(HANDLE);

unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Handle {
    pub fn new(handle: HANDLE) -> Handle {
        Handle(handle)
    }

    pub fn raw(&self) -> HANDLE { self.0 }

    pub fn into_raw(self) -> HANDLE {
        use std::mem;

        let ret = self.0;
        mem::forget(self);
        ret
    }

    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        let mut bytes = 0;
        let len = cmp::min(buf.len(), <DWORD>::max_value() as usize) as DWORD;
        try!(::cvt(unsafe {
            WriteFile(self.0, buf.as_ptr() as *const _, len, &mut bytes,
                      0 as *mut _)
        }));
        Ok(bytes as usize)
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let mut bytes = 0;
        let len = cmp::min(buf.len(), <DWORD>::max_value() as usize) as DWORD;
        try!(::cvt(unsafe {
            ReadFile(self.0, buf.as_mut_ptr() as *mut _, len, &mut bytes,
                     0 as *mut _)
        }));
        Ok(bytes as usize)
    }

    pub unsafe fn read_overlapped(&self, buf: &mut [u8],
                                  overlapped: *mut OVERLAPPED)
                                  -> io::Result<Option<usize>> {
        let len = cmp::min(buf.len(), <DWORD>::max_value() as usize) as DWORD;
        let res = ::cvt({
            ReadFile(self.0,
                     buf.as_mut_ptr() as *mut _,
                     len,
                     ptr::null_mut(),
                     overlapped)
        });
        match res {
            Ok(_) => (),
            Err(ref e) if e.raw_os_error() == Some(ERROR_IO_PENDING as i32)
                => (),
            Err(e) => return Err(e),
        }

        let mut bytes = 0;
        let res = ::cvt({
            GetOverlappedResult(self.0,
                                overlapped,
                                &mut bytes,
                                false)
        });
        match res {
            Ok(_) => Ok(Some(bytes as usize)),
            Err(ref e) if e.raw_os_error() == Some(ERROR_IO_INCOMPLETE as i32)
                => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub unsafe fn write_overlapped(&self, buf: &[u8],
                                   overlapped: *mut OVERLAPPED)
                                   -> io::Result<Option<usize>> {
        let len = cmp::min(buf.len(), <DWORD>::max_value() as usize) as DWORD;
        let res = ::cvt({
            WriteFile(self.0,
                      buf.as_ptr() as *const _,
                      len,
                      ptr::null_mut(),
                      overlapped)
        });
        match res {
            Ok(_) => Ok(Some(bytes as usize)),
            Err(ref e) if e.raw_os_error() == Some(ERROR_IO_PENDING as i32)
                => Ok(None),
            Err(e) => Err(e),
        }

        let mut bytes = 0;
        let res = ::cvt({
            GetOverlappedResult(self.0,
                                overlapped,
                                &mut bytes,
                                false)
        });
        match res {
            Ok(_) => Ok(Some(bytes as usize)),
            Err(ref e) if e.raw_os_error() == Some(ERROR_IO_INCOMPLETE as i32)
                => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}
