extern crate libc;
extern crate mach;
extern crate spawn_task_port;

use mach::kern_return::{kern_return_t, KERN_SUCCESS};
use mach::types::task_t;
use spawn_task_port::CommandSpawnWithTask;
use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn test_process_path() -> Option<PathBuf> {
    env::current_exe()
        .ok()
        .and_then(|p| {
            p.parent().map(|p| {
                p.with_file_name("test")
                    .with_extension(env::consts::EXE_EXTENSION)
            })
        })
}

extern "C" {
    fn pid_for_task(task: task_t, pid: *mut libc::c_int) -> kern_return_t;
}

#[test]
fn test_process_pid() {
    let path = test_process_path().unwrap();
    let (mut child, task_port) = Command::new(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn_get_task_port()
        .expect("failed to spawn child");
    // Simplest use of the task port I could come up with--just
    // ask for its PID and compare with the value from the `fork`.
    unsafe {
        let mut pid = 0;
        assert_eq!(KERN_SUCCESS, pid_for_task(task_port, &mut pid));
        assert_eq!(pid as u32, child.id());
    }
    // wait will close the child's stdin, so it will terminate.
    let status = child.wait().expect("failed to wait for child");
    assert!(status.success(), "Child should have exited normally");
}
