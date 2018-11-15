#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
use std::os::raw::{c_char, c_uchar, c_void};

extern "C" {
    pub fn sg_request_info() -> *mut sapi_request_info;
    pub fn sg_server_context() -> *mut c_void;
    pub fn sg_set_server_context(context: *mut c_void);
    pub fn sg_sapi_headers() -> *mut sapi_headers_struct;
    pub fn sg_headers_sent() -> c_uchar;
    pub fn sg_set_headers_sent(is_sent: c_uchar);
    pub fn zend_tsrmls_cache_update();
    pub fn phprpm_fopen(filename: *const c_char, mode: *const c_char) -> *mut FILE;
}

include!("bindings.rs");
