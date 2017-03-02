extern crate mach;
extern crate uuid;

// re-export this for convenience.
pub use mach::port::mach_port_t;

use std::ffi::CString;
use std::io::{Error, ErrorKind, Result};
use std::mem;
use std::ops::Drop;
use std::os::unix::process::CommandExt;
use std::process::{Command, Child};

use mach::kern_return::{kern_return_t, KERN_SUCCESS};
use mach::port::{MACH_PORT_NULL, MACH_PORT_RIGHT_RECEIVE};
use mach::mach_port::{mach_port_allocate, mach_port_deallocate, mach_port_insert_right};
use mach::message::{MACH_MSG_TYPE_MAKE_SEND, MACH_MSGH_BITS, MACH_MSG_TYPE_COPY_SEND,
                    MACH_MSGH_BITS_COMPLEX, MACH_RCV_MSG, MACH_MSG_TIMEOUT_NONE, mach_msg_send,
                    mach_msg, mach_msg_header_t, mach_msg_body_t, mach_msg_port_descriptor_t,
                    mach_msg_trailer_t};
use mach::task::{TASK_BOOTSTRAP_PORT, task_get_special_port};
use mach::traps::mach_task_self;
use uuid::Uuid;

/// A macro to wrap mach APIs that return `kern_return_t` to early-return
/// a `std::io::Result` when they fail.
macro_rules! ktry {
    ($e:expr) => {{
        let kr = $e;
        if kr != KERN_SUCCESS {
            return Err(Error::new(ErrorKind::Other,
                                  format!("`{}` failed with return code {:x}",
                                          stringify!($e), kr)));
        }
    }}
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

/// The message format that the child sends to the parent.
#[allow(dead_code)]
struct SendMessage {
    header: mach_msg_header_t,
    body: mach_msg_body_t,
    task_port: mach_msg_port_descriptor_t,
}

/// The message format that the parent receives from the child.
#[allow(dead_code)]
struct RecvMessage {
    header: mach_msg_header_t,
    body: mach_msg_body_t,
    task_port: mach_msg_port_descriptor_t,
    //TODO: make this a mach_msg_audit_trailer_t so we can audit the child PID
    trailer: mach_msg_trailer_t,
}

extern "C" {
    // Technically name_t would be `[i8; 128]`, but that just makes it more
    // of a pain to use and the calling convention is the same.
    //TODO: put this in the mach crate.
    fn bootstrap_look_up(bp: mach_port_t,
                         service_name: *const i8,
                         sp: *mut mach_port_t)
                         -> kern_return_t;
    /// This is not a public API, but it's what everything uses internally.
    fn bootstrap_register2(bp: mach_port_t,
                           service_name: *const i8,
                           sp: mach_port_t,
                           flags: u64)
                           -> kern_return_t;
//TODO: use this for auditing
//fn audit_token_to_pid(audit_token_t atoken) -> pid_t;
}

/// As OS X-specific extension to `std::process::Command` to spawn a process and
/// get back access to its Mach task port.
pub trait CommandSpawnWithTask {
    /// Executes the command as a child process, returning both the `Child`
    /// as well as the process' Mach task port as a `mach_port_t`.
    fn spawn_get_task_port(&mut self) -> Result<(Child, mach_port_t)>;
}

impl CommandSpawnWithTask for Command {
    fn spawn_get_task_port(&mut self) -> Result<(Child, mach_port_t)> {
        // First, create a port to which the child can send us a message.
        let port = unsafe {
            let mut port: mach_port_t = mem::uninitialized();
            ktry!(mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_RECEIVE, &mut port));
            let port = MachPort(port);

            // Allocate a send right for the server port.
            ktry!(mach_port_insert_right(mach_task_self(),
                                         port.0,
                                         port.0,
                                         MACH_MSG_TYPE_MAKE_SEND));
            port
        };

        // Register the port with the bootstrap server.
        let uuid = Uuid::new_v4().simple().to_string();
        let name = CString::new(uuid).or(Err(Error::new(ErrorKind::Other, "CString")))?;
        unsafe {
            let mut bootstrap_port = mem::uninitialized();
            ktry!(task_get_special_port(mach_task_self(),
                                        TASK_BOOTSTRAP_PORT,
                                        &mut bootstrap_port));
            ktry!(bootstrap_register2(bootstrap_port, name.as_ptr(), port.0, 0));
        }

        let child = self.before_exec(move || {
                unsafe {
                    // Next, in the child process' `before_exec`, look up the
                    // registered port.
                    let mut bootstrap_port: mach_port_t = mem::uninitialized();
                    ktry!(task_get_special_port(mach_task_self(),
                                                TASK_BOOTSTRAP_PORT,
                                                &mut bootstrap_port));

                    let mut parent_port: mach_port_t = mem::uninitialized();
                    ktry!(bootstrap_look_up(bootstrap_port, name.as_ptr(), &mut parent_port));
                    let parent_port = MachPort(parent_port);
                    // Now use the port to send our task port to the parent.
                    let mut msg = SendMessage {
                        header: mach_msg_header_t {
                            msgh_bits: MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0) |
                                       MACH_MSGH_BITS_COMPLEX,
                            msgh_size: mem::size_of::<SendMessage>() as u32,
                            msgh_remote_port: parent_port.0,
                            msgh_local_port: MACH_PORT_NULL,
                            msgh_voucher_port: MACH_PORT_NULL,
                            msgh_id: 0,
                        },
                        body: mach_msg_body_t { msgh_descriptor_count: 1 },
                        task_port: mach_msg_port_descriptor_t::new(mach_task_self(),
                                                                   MACH_MSG_TYPE_COPY_SEND),
                    };
                    ktry!(mach_msg_send(&mut msg.header));
                }
                Ok(())
            })
            .spawn()?;
        // In the parent, receive the child's task port.
        let child_task_port = unsafe {
            let mut msg: RecvMessage = mem::uninitialized();
            //TODO: MACH_RCV_MSG |
            // MACH_RCV_TRAILER_TYPE(MACH_RCV_TRAILER_AUDIT) |
            // MACH_RCV_TRAILER_ELEMENTS(MACH_RCV_TRAILER_AUDIT)
            ktry!(mach_msg(&mut msg.header,
                           MACH_RCV_MSG,
                           0,
                           mem::size_of::<RecvMessage>() as u32,
                           port.0,
                           MACH_MSG_TIMEOUT_NONE,
                           MACH_PORT_NULL));
            //TODO: Check that this message came from the child process
            // with `audit_token_to_pid`.
            msg.task_port.name
        };
        Ok((child, child_task_port))
    }
}
