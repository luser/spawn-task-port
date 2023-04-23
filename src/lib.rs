//! A crate to spawn a child process on OS X and get the child's Mach task port.
//! [Many useful OS X kernel APIs](http://web.mit.edu/darwin/src/modules/xnu/osfmk/man/) require access to the task port, and in recent releases of OS X the security around `task_for_pid` has been tightened such that it no longer works reliably even as root.
//! However, for processes that you are spawning it is possible to have the child cooperate and send its task port to the parent.
//!
//! This crate uses `CommandExt::pre_exec` and a handful of Mach APIs to have the child process do just that.
//! The technique used is based on [Chromium's mach_port_broker.mm](https://chromium.googlesource.com/chromium/src.git/+/466f0cb8d47e7da69a06cb6dc9b60fe5511fc8d1/base/mac/mach_port_broker.mm)
//!
//! # Example
//! ```rust
//! use std::env;
//! use std::ffi::c_uchar;
//! use std::io::Read;
//! use std::mem::{size_of, MaybeUninit};
//! use std::path::PathBuf;
//! use std::process::{Command, Stdio};
//! use std::thread::sleep;
//! use std::time::Duration;
//!
//! use mach2::kern_return::KERN_SUCCESS;
//! use mach2::task::task_info;
//! use mach2::task_info::TASK_EXTMOD_INFO;
//! use mach2::vm_types::natural_t;
//!
//! use spawn_task_port::CommandSpawnWithTask;
//!
//! // Structs that should be defined in mach2 but aren't
//! #[allow(non_camel_case_types)]
//! #[derive(Debug)]
//! #[repr(C, align(8))]
//! struct vm_extmod_statistics_data_t {
//!     task_for_pid_count: i64,
//!     task_for_pid_caller_count: i64,
//!     thread_creation_count: i64,
//!     thread_creation_caller_count: i64,
//!     thread_set_state_count: i64,
//!     thread_set_state_caller_count: i64,
//! }
//!
//! #[allow(non_camel_case_types)]
//! #[derive(Debug)]
//! #[repr(C)]
//! struct task_extmod_info {
//!     uuid: [c_uchar; 16],
//!     info: vm_extmod_statistics_data_t,
//! }
//!
//! // Use this example as its own target
//! if env::args().nth(1) == Some("--child".to_string()) {
//!     let mut s = String::new();
//!     std::io::stdin().read_to_string(&mut s).unwrap();
//!     std::process::exit(0);
//! }
//! // Create Command as usual
//! let (mut child, task_port) = Command::new(env::current_exe().unwrap())
//!     .arg("--child")
//!     .stdin(Stdio::piped())  // Make child block on stdin to wait forever
//!     .stdout(Stdio::piped())
//!     .spawn_get_task_port()  // Here's the important part
//!     .expect("failed to spawn child");
//!
//! // Let child process init//!
//! sleep(Duration::from_millis(10));
//!
//! // Now we have a task port!
//! // Example use: task_info call, to check if the process has been externally modified
//! let info: task_extmod_info = unsafe {
//!     let mut info = MaybeUninit::zeroed();
//!     assert_eq!(
//!         task_info(
//!             task_port,
//!             TASK_EXTMOD_INFO,
//!             info.as_mut_ptr() as *mut i32,
//!             &mut ((size_of::<task_extmod_info>() / size_of::<natural_t>()) as u32),
//!         ),
//!         KERN_SUCCESS
//!     );
//!     info.assume_init()
//! };
//! // No task_for_pid usage!
//! println!(
//!     "Child process {} modification statistics: {:#?}",
//!     child.id(),
//!     info.info
//! );
//!
//! // `wait()` will close the child's stdin, so it will exit.
//! let status = child.wait().expect("failed to wait for child");
//! assert!(status.success(), "child should have exited normally");
//! ```

// re-export this for convenience.
pub use mach2::port::mach_port_t;

use std::ffi::CString;
use std::io::{Error, ErrorKind, Result};
use std::mem;
use std::mem::MaybeUninit;
use std::ops::Drop;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

use mach2::bootstrap::bootstrap_look_up;
use mach2::kern_return::KERN_SUCCESS;
use mach2::mach_port::{mach_port_allocate, mach_port_deallocate, mach_port_insert_right};
use mach2::message::{
    mach_msg, mach_msg_body_t, mach_msg_header_t, mach_msg_port_descriptor_t,
    MACH_MSGH_BITS_COMPLEX, MACH_MSG_TIMEOUT_NONE, MACH_MSG_TYPE_COPY_SEND,
    MACH_MSG_TYPE_MAKE_SEND, MACH_RCV_MSG, MACH_SEND_MSG,
};
use mach2::port::{MACH_PORT_NULL, MACH_PORT_RIGHT_RECEIVE};
use mach2::task::{task_get_special_port, TASK_BOOTSTRAP_PORT};
use mach2::traps::mach_task_self;

use uuid::Uuid;

mod stubs;
use crate::stubs::{
     bootstrap_register2, mach_msg_recv_t, mach_msg_send_t,
    MACH_MSGH_BITS_REMOTE
};
#[cfg(feature = "audit_pid")]
use crate::stubs::{audit_token_to_pid, MACH_RCV_TRAILER_AUDIT, MACH_RCV_TRAILER_ELEMENTS, MACH_RCV_TRAILER_TYPE};

/// A macro to wrap mach APIs that return `kern_return_t` to early-return
/// a `std::io::Result` when they fail.
macro_rules! ktry {
    ($e:expr) => {{
        let kr = $e;
        if kr != KERN_SUCCESS {
            return Err(Error::new(
                ErrorKind::Other,
                format!("`{}` failed with return code {:x}", stringify!($e), kr),
            ));
        }
    }};
}

/// A wrapper for a `mach_port_t` to deallocate the port on drop.
struct MachPort(mach_port_t);

impl Drop for MachPort {
    fn drop(&mut self) {
        // Ignore failures, there's not much that can be done here.
        unsafe {
            mach_port_deallocate(mach_task_self(), self.0);
        }
    }
}

/// As OS X-specific extension to `std::process::Command` to spawn a process and gain
/// with access to its Mach task port.
pub trait CommandSpawnWithTask {
    /// Executes the command as a child process, returning both the `Child`
    /// as well as the process' Mach task port as a `mach_port_t`.
    fn spawn_get_task_port(&mut self) -> Result<(Child, mach_port_t)>;
}

impl CommandSpawnWithTask for Command {
    fn spawn_get_task_port(&mut self) -> Result<(Child, mach_port_t)> {
        // First, create a port to which the child can send us a message.
        let port = unsafe {
            let port: MachPort = {
                let mut r = MaybeUninit::zeroed();
                ktry!(mach_port_allocate(
                    mach_task_self(),
                    MACH_PORT_RIGHT_RECEIVE,
                    r.as_mut_ptr()
                ));
                MachPort(r.assume_init())
            };

            // Allocate a send right for the server port.
            ktry!(mach_port_insert_right(
                mach_task_self(),
                port.0,
                port.0,
                MACH_MSG_TYPE_MAKE_SEND
            ));
            port
        };

        // Register the port with the bootstrap server.
        let uuid = Uuid::new_v4().simple().to_string();
        let name = CString::new(uuid).or(Err(Error::new(ErrorKind::Other, "CString")))?;
        unsafe {
            let bootstrap_port: mach_port_t = {
                let mut r = MaybeUninit::zeroed();
                ktry!(task_get_special_port(
                    mach_task_self(),
                    TASK_BOOTSTRAP_PORT,
                    r.as_mut_ptr()
                ));
                r.assume_init()
            };
            ktry!(bootstrap_register2(
                bootstrap_port,
                name.as_ptr(),
                port.0,
                0
            ));
        }

        let child = unsafe {
            self.pre_exec(move || {
                // Next, in the child process' `before_exec`, look up the
                // registered port.
                let bootstrap_port: mach_port_t = {
                    let mut r = MaybeUninit::zeroed();
                    ktry!(task_get_special_port(
                        mach_task_self(),
                        TASK_BOOTSTRAP_PORT,
                        r.as_mut_ptr()
                    ));
                    r.assume_init()
                };
                let parent_port: MachPort = {
                    let mut r = MaybeUninit::zeroed();
                    ktry!(bootstrap_look_up(
                        bootstrap_port,
                        name.as_ptr(),
                        r.as_mut_ptr()
                    ));
                    MachPort(r.assume_init())
                };
                // Now use the port to send our task port to the parent.
                let mut msg = mach_msg_send_t {
                    msg_header: mach_msg_header_t {
                        msgh_bits: MACH_MSGH_BITS_REMOTE(MACH_MSG_TYPE_COPY_SEND)
                            | MACH_MSGH_BITS_COMPLEX,
                        msgh_size: mem::size_of::<mach_msg_send_t>() as u32,
                        msgh_remote_port: parent_port.0,
                        msgh_local_port: MACH_PORT_NULL,
                        msgh_voucher_port: MACH_PORT_NULL,
                        msgh_id: 0,
                    },
                    msg_body: mach_msg_body_t {
                        msgh_descriptor_count: 1,
                    },
                    task_port: mach_msg_port_descriptor_t::new(
                        mach_task_self(),
                        MACH_MSG_TYPE_COPY_SEND,
                    ),
                };
                ktry!(mach_msg(
                    &mut msg.msg_header,
                    MACH_SEND_MSG,
                    mem::size_of::<mach_msg_send_t>() as u32,
                    0,
                    MACH_PORT_NULL,
                    MACH_MSG_TIMEOUT_NONE,
                    MACH_PORT_NULL
                ));
                Ok(())
            })
            .spawn()?
        };

        // In the parent, receive the child's task port.
        let child_task_port = unsafe {
            let msg: mach_msg_recv_t = {
                let mut r: MaybeUninit<mach_msg_recv_t> = MaybeUninit::zeroed();
                #[cfg(feature = "audit_pid")]
                let options = MACH_RCV_TRAILER_TYPE(MACH_RCV_TRAILER_AUDIT)
                | MACH_RCV_TRAILER_ELEMENTS(MACH_RCV_TRAILER_AUDIT);
                #[cfg(not(feature = "audit_pid"))]
                let options = 0;
                ktry!(mach_msg(
                    std::ptr::addr_of_mut!((*r.as_mut_ptr()).msg_header),
                    MACH_RCV_MSG
                        | options,
                    0,
                    mem::size_of::<mach_msg_recv_t>() as u32,
                    port.0,
                    MACH_MSG_TIMEOUT_NONE,
                    MACH_PORT_NULL
                ));
                r.assume_init()
            };

            // Check that the message was send by the child
            // Because the bootstrap name is a random UUID, it's unlikely that another process
            // could have intentionally or accidentally send another port, but it's not difficult to check
            #[cfg(feature = "audit_pid")]
            if audit_token_to_pid(msg.msg_trailer.msgh_audit) != child.id() {
                return Err(Error::new(
                    ErrorKind::Other,
                    format!(
                        "expected task port for child pid {}, got pid {} instead",
                        child.id(),
                        audit_token_to_pid(msg.msg_trailer.msgh_audit)
                    ),
                ));
            }

            msg.task_port.name
        };
        Ok((child, child_task_port))
    }
}
