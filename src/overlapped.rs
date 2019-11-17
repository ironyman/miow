//! Overlapped type.
use std::fmt;
use std::io;
use std::mem;
use std::ptr;

use winapi::shared::ntdef::{
    HANDLE,
    NULL,
};
use winapi::shared::minwindef::{FALSE, TRUE};

use winapi::um::minwinbase::*;
use winapi::um::synchapi::*;
pub use winapi::um::winsock2::{
    SOCKET,
    WSAGetOverlappedResult,
    WSA_IO_INCOMPLETE,
};

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use event_loop::Watcher;

/// General type for HANDLE and SOCKET.
#[derive(Copy, Clone)]
pub enum GeneralHandle {
    /// Handle
    Handle(HANDLE),
    /// Socket
    Socket(SOCKET)
}

/// A wrapper around `OVERLAPPED` to provide "rustic" accessors and
/// initializers.
pub struct Overlapped(OVERLAPPED, Option<GeneralHandle>);

impl fmt::Debug for Overlapped {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "OVERLAPPED")
    }
}

unsafe impl Send for Overlapped {}
unsafe impl Sync for Overlapped {}

impl Overlapped {
    /// Creates a new zeroed out instance of an overlapped I/O tracking state.
    ///
    /// This is suitable for passing to methods which will then later get
    /// notified via an I/O Completion Port.
    pub fn zero() -> Overlapped {
        Overlapped(unsafe { mem::zeroed() }, None)
    }

    /// Create a new instance with the handle associated with this overlapped operation.
    pub fn with_handle(h: HANDLE) -> Overlapped {
        Overlapped(unsafe { mem::zeroed() }, Some(GeneralHandle::Handle(h)) )
    }

    /// Create a new instance with the socket associated with this overlapped operation.
    pub fn with_socket(s: SOCKET) -> Overlapped {
        Overlapped(unsafe { mem::zeroed() }, Some(GeneralHandle::Socket(s)) )
    }

    /// Creates a new `Overlapped` with an initialized non-null `hEvent`.  The caller is
    /// responsible for calling `CloseHandle` on the `hEvent` field of the returned
    /// `Overlapped`.  The event is created with `bManualReset` set to `FALSE`, meaning after a
    /// single thread waits on the event, it will be reset.
    pub fn initialize_with_autoreset_event() -> io::Result<Overlapped> {
        let event = unsafe {CreateEventW(ptr::null_mut(), 0i32, 0i32, ptr::null())};
        if event == NULL {
            return Err(io::Error::last_os_error());
        }
        let mut overlapped = Self::zero();
        overlapped.set_event(event);
        Ok(overlapped)
    }

    /// Creates a new `Overlapped` function pointer from the underlying
    /// `OVERLAPPED`, wrapping in the "rusty" wrapper for working with
    /// accessors.
    ///
    /// # Unsafety
    ///
    /// This function doesn't validate `ptr` nor the lifetime of the returned
    /// pointer at all, it's recommended to use this method with extreme
    /// caution.
    pub unsafe fn from_raw<'a>(ptr: *mut OVERLAPPED) -> &'a mut Overlapped {
        &mut *(ptr as *mut Overlapped)
    }

    /// Gain access to the raw underlying data
    pub fn raw(&self) -> *mut OVERLAPPED {
        &self.0 as *const _ as *mut _
    }

    /// Sets the offset inside this overlapped structure.
    ///
    /// Note that for I/O operations in general this only has meaning for I/O
    /// handles that are on a seeking device that supports the concept of an
    /// offset.
    pub fn set_offset(&mut self, offset: u64) {
        let s = unsafe { self.0.u.s_mut() };
        s.Offset = offset as u32;
        s.OffsetHigh = (offset >> 32) as u32;
    }

    /// Reads the offset inside this overlapped structure.
    pub fn offset(&self) -> u64 {
        let s = unsafe { self.0.u.s() };
        (s.Offset as u64) | ((s.OffsetHigh as u64) << 32)
    }

    /// Sets the `hEvent` field of this structure.
    ///
    /// The event specified can be null.
    pub fn set_event(&mut self, event: HANDLE) {
        self.0.hEvent = event;
    }

    /// Reads the `hEvent` field of this structure, may return null.
    pub fn event(&self) -> HANDLE {
        self.0.hEvent
    }
}

impl Future for Overlapped {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut transferred = 0;
        let mut flags = 0;

        if self.1.is_none() {
            println!("IS NONE");
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Need handle set.",
            )));
        }

        let g = self.1.unwrap();

        if let GeneralHandle::Socket(s) = g {
            let res = ::cvt(unsafe {
                WSAGetOverlappedResult(s,
                                    self.raw(),
                                    &mut transferred,
                                    FALSE,
                                    &mut flags)
            });
            match res {
                Err(e) => {
                    if e.raw_os_error() == Some(WSA_IO_INCOMPLETE) {
                        
                        Poll::Pending
                    } else {
                        Poll::Ready(Err(e))
                    }
                }
                Ok(_) => {
                    Poll::Ready(Ok(()))
                }
            }
        } else {
            panic!("Not implemented.");
        }
    }
}