#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

#[cfg(feature = "audit_pid")]
use std::ffi::c_uint;

use mach2::kern_return::kern_return_t;
use mach2::message::{
    mach_msg_bits_t, mach_msg_body_t, mach_msg_header_t,
    mach_msg_port_descriptor_t,
    MACH_MSGH_BITS_REMOTE_MASK,
};
#[cfg(feature = "audit_pid")]
use mach2::message::{mach_msg_option_t, mach_msg_trailer_size_t, mach_msg_trailer_type_t};
#[cfg(not(feature = "audit_pid"))]
use mach2::message::mach_msg_trailer_t;
use mach2::port::mach_port_t;
#[cfg(feature = "audit_pid")]
use mach2::vm_types::natural_t;

#[cfg(feature = "audit_pid")]
pub(crate) const MACH_RCV_TRAILER_AUDIT: mach_msg_option_t = 3;

#[cfg(feature = "audit_pid")]
pub(crate) type mach_port_seqno_t = natural_t;
#[cfg(feature = "audit_pid")]
pub(crate) type audit_token_t = [c_uint; 8];
#[cfg(feature = "audit_pid")]
pub(crate) type security_token_t = [c_uint; 2];

#[cfg(feature = "audit_pid")]
#[repr(C)]
pub(crate) struct mach_msg_audit_trailer_t {
    pub msgh_trailer_type: mach_msg_trailer_type_t,
    pub msgh_trailer_size: mach_msg_trailer_size_t,
    pub msgh_seqno: mach_port_seqno_t,
    pub msgh_sender: security_token_t,
    pub msgh_audit: audit_token_t,
}

#[repr(C)]
pub(crate) struct mach_msg_send_t {
    pub msg_header: mach_msg_header_t,
    pub msg_body: mach_msg_body_t,
    pub task_port: mach_msg_port_descriptor_t,
}

#[repr(C)]
pub(crate) struct mach_msg_recv_t {
    pub msg_header: mach_msg_header_t,
    pub msg_body: mach_msg_body_t,
    pub task_port: mach_msg_port_descriptor_t,
    #[cfg(feature = "audit_pid")]
    pub msg_trailer: mach_msg_audit_trailer_t,
    #[cfg(not(feature = "audit_pid"))]
    pub msg_trailer: mach_msg_trailer_t,
}

extern "C" {
    // Not public, but used internally by the Obj-C bootstrap API
    pub(crate) fn bootstrap_register2(
        bp: mach_port_t,
        service_name: *const i8,
        sp: mach_port_t,
        flags: u64,
    ) -> kern_return_t;
}

#[cfg(feature = "audit_pid")]
#[link(name = "bsm")]
extern "C" {
    // Rust complains about passing [c_uint; 8] through the C ABI, but that's what the arg is in apple's docs
    #[allow(improper_ctypes)]
    pub(crate) fn audit_token_to_pid(audit_token: audit_token_t) -> u32;
}

pub(crate) fn MACH_MSGH_BITS_REMOTE(remote: mach_msg_bits_t) -> mach_msg_bits_t {
    (remote) & MACH_MSGH_BITS_REMOTE_MASK
}

#[cfg(feature = "audit_pid")]
pub(crate) fn MACH_RCV_TRAILER_TYPE(msg_type: mach_msg_option_t) -> mach_msg_option_t {
    ((msg_type) & 0xf) << 28
}

#[cfg(feature = "audit_pid")]
pub(crate) fn MACH_RCV_TRAILER_ELEMENTS(msg_elems: mach_msg_option_t) -> mach_msg_option_t {
    ((msg_elems) & 0xf) << 24
}
