[![crates.io](https://img.shields.io/crates/v/spawn-task-port.svg)](https://crates.io/crates/spawn-task-port)
[![docs.rs](https://docs.rs/spawn-task-port/badge.svg)](https://docs.rs/spawn-task-port)

A crate to spawn a child process on OS X and get the child's Mach task port.
[Many useful OS X kernel APIs](http://web.mit.edu/darwin/src/modules/xnu/osfmk/man/) require access to the task port, and in recent releases of OS X the security around `task_for_pid` has been tightened such that it no longer works reliably even as root.
However, for processes that you are spawning it is possible to have the child cooperate and send its task port to the parent.

This crate uses `CommandExt::before_exec` and a handful of Mach APIs to have the child process do just that.
The technique used is based on [Chromium's mach_port_broker.mm](https://chromium.googlesource.com/chromium/src.git/+/466f0cb8d47e7da69a06cb6dc9b60fe5511fc8d1/base/mac/mach_port_broker.mm) and information from Michael Weber's [Some Fun with Mach Ports](http://web.archive.org/web/20160703203506/https://www.foldr.org/~michaelw/log/computers/macosx/task-info-fun-with-mach) blog post, and other bits were gleaned from [Chromium's mach_port_broker.mm](https://chromium.googlesource.com/chromium/src.git/+/466f0cb8d47e7da69a06cb6dc9b60fe5511fc8d1/base/mac/mach_port_broker.mm).

This crate was written so I could use it to write tests for the [read-process-memory](https://github.com/luser/read-process-memory) crate. You may find this crate useful in conjunction with that one!

# Example
```rust
use std::env;
use std::ffi::c_uchar;
use std::io::Read;
use std::mem::{size_of, MaybeUninit};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use mach2::kern_return::KERN_SUCCESS;
use mach2::task::task_info;
use mach2::task_info::TASK_EXTMOD_INFO;
use mach2::vm_types::natural_t;

use spawn_task_port::CommandSpawnWithTask;

// Structs that should be defined in mach2 but aren't
#[allow(non_camel_case_types)]
#[derive(Debug)]
#[repr(C, align(8))]
struct vm_extmod_statistics_data_t {
    task_for_pid_count: i64,
    task_for_pid_caller_count: i64,
    thread_creation_count: i64,
    thread_creation_caller_count: i64,
    thread_set_state_count: i64,
    thread_set_state_caller_count: i64,
}

#[allow(non_camel_case_types)]
#[derive(Debug)]
#[repr(C)]
struct task_extmod_info {
    uuid: [c_uchar; 16],
    info: vm_extmod_statistics_data_t,
}

fn main() {
    // Use this example as its own target
    if env::args().nth(1) == Some("--child".to_string()) {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).unwrap();
        std::process::exit(0);
    }
    // Create Command as usual
    let (mut child, task_port) = Command::new(env::current_exe().unwrap())
        .arg("--child")
        .stdin(Stdio::piped())  // Make child block on stdin to wait forever
        .stdout(Stdio::piped())
        .spawn_get_task_port()  // Here's the important part
        .expect("failed to spawn child");

    // Now we have a task port!
    // Example use: task_info call, to check if the process has been externally modified
    let info: task_extmod_info = unsafe {
        let mut info = MaybeUninit::zeroed();
        assert_eq!(
            task_info(
                task_port,
                TASK_EXTMOD_INFO,
                info.as_mut_ptr() as *mut i32,
                &mut ((size_of::<task_extmod_info>() / size_of::<natural_t>()) as u32),
            ),
            KERN_SUCCESS
        );
        info.assume_init()
    };
    // No task_for_pid usage!
    println!(
        "Child process {} modification statistics: {:#?}",
        child.id(),
        info.info
    );

    // `wait()` will close the child's stdin, so it will exit.
    let status = child.wait().expect("failed to wait for child");
    assert!(status.success(), "child should have exited normally");
}
```
