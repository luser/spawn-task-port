[![Build Status](https://travis-ci.org/luser/rust-spawn-task-port.svg?branch=master)](https://travis-ci.org/luser/rust-spawn-task-port) [![crates.io](https://img.shields.io/crates/v/spawn-task-port.svg)](https://crates.io/crates/spawn-task-port) [![](https://docs.rs/spawn-task-port/badge.svg)](https://docs.rs/spawn-task-port)

A crate to spawn a child process on OS X and get the child's Mach task port. [Many useful OS X kernel APIs](http://web.mit.edu/darwin/src/modules/xnu/osfmk/man/) require access to the task port, and in recent releases of OS X the security around `task_for_pid` has been tightened such that it no longer works reliably even as root. However, for processes that you are spawning it is possible to have the child cooperate and send its task port to the parent. This crate uses `CommandExt::before_exec` and a handful of Mach APIs to have the child process do just that.

Much of this code is written using information from Michael Weber's [Some Fun with Mach Ports](http://www.foldr.org/%7Emichaelw/log/computers/macosx/task-info-fun-with-mach) blog post, and other bits were gleaned from [Chromium's mach_port_broker.mm](https://chromium.googlesource.com/chromium/src.git/+/466f0cb8d47e7da69a06cb6dc9b60fe5511fc8d1/base/mac/mach_port_broker.mm).

This crate was written so I could use it to write tests for the [read-process-memory](https://github.com/luser/read-process-memory) crate. You may find this crate useful in conjunction with that one!


# Example

```rust,no_run
extern crate spawn_task_port;

use std::io;
use std::process::Command;
use spawn_task_port::CommandSpawnWithTask;

// Spawn `exe` with `args` as a child process and do interesting
// things to it.
fn do_some_work(exe: &str, args: &[&str]) -> io::Result<()> {
 let (mut child, task_port) = Command::new(&)
        .args(args)
        .spawn_get_task_port()?
 // Now you can call mach APIs that require a `mach_port_t` using `task_port`,
 // like `vm_read`.
 child.wait()?;
 Ok(())
}
```

# Documentation

[https://docs.rs/spawn-task-port](https://docs.rs/spawn-task-port)
