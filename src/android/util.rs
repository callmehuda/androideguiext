use anyhow::Result;
use std::ffi::CStr;

use libc::{c_char, c_int};

unsafe extern "C" {
    fn __system_property_get(name: *const c_char, value: *mut c_char) -> c_int;
}

pub fn get_api_level() -> Result<u32> {
    let api_level = unsafe {
        let mut api_level = [0; 256];
        __system_property_get(
            c"ro.build.version.sdk".as_ptr() as *const c_char,
            api_level.as_mut_ptr(),
        );
        CStr::from_ptr(api_level.as_ptr()).to_string_lossy()
    };
    Ok(api_level.parse::<u32>()?)
}

pub fn get_android_version() -> Result<u32> {
    let android_version = unsafe {
        let mut android_version = [0; 256];
        __system_property_get(
            c"ro.build.version.release".as_ptr() as *const c_char,
            android_version.as_mut_ptr(),
        );

        CStr::from_ptr(android_version.as_ptr()).to_string_lossy()
    };
    Ok(android_version.parse()?)
}
