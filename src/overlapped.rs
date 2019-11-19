#![feature(pin)]
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

use std::os::windows::io::AsRawSocket;

extern crate event_loop;
use self::event_loop::Watcher;

// This doesn't work??
// use event_loop::Watcher;


/// A wrapper around `OVERLAPPED` to provide "rustic" accessors and
/// initializers.
pub struct Overlapped(OVERLAPPED);

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
        Overlapped(unsafe { mem::zeroed() } )
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

/// Awaitable overlapped.
pub struct OverlappedFuture<'a, T: AsRawSocket> {
    overlapped: Overlapped,
    watcher: &'a Watcher<T>,
}

impl<'a, T: AsRawSocket> OverlappedFuture<'a, T> {
    /// Create pinned with watcher.
    pub fn pinned_with_watcher(watcher: &'a Watcher<T>) -> Pin<Box<Self>> {
        Pin::new(Box::new(OverlappedFuture {
            overlapped: Overlapped::zero(),
            watcher,
        }))
    }

    /// Create with watcher.
    pub fn with_watcher(watcher: &'a Watcher<T>) -> Self {
        OverlappedFuture {
            overlapped: Overlapped::zero(),
            watcher,
        }
    }

    /// Gain access to the raw underlying data
    pub fn raw(&self) -> *mut OVERLAPPED {
        &self.overlapped.0 as *const _ as *mut _
    }

    
    fn get_overlapped_result(
        &self
    ) -> Poll<<Self as Future>::Output>
    {
        let mut transferred = 0;
        let mut flags = 0;
        let res = ::cvt(unsafe {
            WSAGetOverlappedResult(self.watcher.get_ref().as_raw_socket() as SOCKET,
                                self.raw(),
                                &mut transferred,
                                FALSE,
                                &mut flags)
        });

        kv_log_macro::info!("Overlapped polled", {
            thread: std::thread::current().name().unwrap(),
            thread_id: format!("{:?}", std::thread::current().id()),
            result: format!("{:?}", res),
            overlapped: format!("{:#x}", self.raw() as usize),
            socket: format!("{:#}", self.watcher.get_ref().as_raw_socket() as SOCKET)
        });
        kv_log_macro::info!("Dumping overlapped bytes", {
            thread: std::thread::current().name().unwrap(),
            thread_id: format!("{:?}", std::thread::current().id()),
            internal: format!("{:?}", unsafe { (*self.raw()).Internal }),
            internalhigh: format!("{:?}", unsafe { (*self.raw()).InternalHigh }),
            u: format!("{:?}", unsafe { (*self.raw()).u.Pointer() }),
            event: format!("{:?}", unsafe { (*self.raw()).hEvent }),
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
    }
}


impl<'a, T: AsRawSocket> Future for OverlappedFuture<'a, T> {
    type Output = io::Result<()>;


    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let polled = self.as_ref().get_overlapped_result();
        if let Poll::Ready(_) = polled {
            return polled
        }

        // If in the watcher thread at and we are at this point:
        // 1. the IO completion is triggered
        // 2. It wakes everyone in the waker list.
        // And then we register ourselves to the watcher,
        // we will never be woken!
        // Solution: lock the waker list and check again.
        kv_log_macro::info!("Poll locking waker", {
            thread: std::thread::current().name().unwrap(),
            thread_id: format!("{:?}", std::thread::current().id()),
            socket: format!("{:#}", self.watcher.get_ref().as_raw_socket() as usize),
            waker: format!("{:?}", cx.waker()),
        });
        let mut wakers = self.watcher.lock_wakers().unwrap();

        let polled = self.as_ref().get_overlapped_result();
        if let Poll::Ready(_) = polled {
            return polled
        }
        

        // Register in wakers list.
       kv_log_macro::info!("Registering waker", {
            thread: std::thread::current().name().unwrap(),
            thread_id: format!("{:?}", std::thread::current().id()),
            socket: format!("{:#}", self.watcher.get_ref().as_raw_socket() as usize),
            waker: format!("{:?}", cx.waker()),
        });
        if !wakers.iter().any(|w| w.will_wake(cx.waker())) {
            wakers.push(cx.waker().clone());
        }

        return polled;
         
    }
    // fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
    //     let mut transferred = 0;
    //     let mut flags = 0;
    //     let res = ::cvt(unsafe {
    //         WSAGetOverlappedResult(self.watcher.get_ref().as_raw_socket() as SOCKET,
    //                             self.raw(),
    //                             &mut transferred,
    //                             FALSE,
    //                             &mut flags)
    //     });


    //     match res {
    //         Err(e) => {
    //             if e.raw_os_error() == Some(WSA_IO_INCOMPLETE) {
    //                 self.watcher.register(cx);
    //                 Poll::Pending
    //             } else {
    //                 Poll::Ready(Err(e))
    //             }
    //         }
    //         Ok(_) => {
    //             Poll::Ready(Ok(()))
    //         }
    //     }
    // }
}