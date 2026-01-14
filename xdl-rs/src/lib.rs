use std::ffi::{CString, c_void};
use std::os::raw::c_int;
use std::ptr::NonNull;

#[allow(non_upper_case_globals)]
#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[allow(dead_code)]
mod ffi;

pub use ffi::{
    XDL_ALWAYS_FORCE_LOAD, XDL_DEFAULT, XDL_DI_DLINFO, XDL_FULL_PATHNAME, XDL_NON_SYM,
    XDL_TRY_FORCE_LOAD, dl_phdr_info, xdl_info_t,
};

/// A handle to an opened library.
///
/// This struct represents a library loaded via `xdl_open`.
/// It automatically closes the library when dropped.
#[derive(Debug)]
pub struct Library(NonNull<c_void>);

// Libraries loaded with xdl are generally thread-safe to access (pointers are valid).
unsafe impl Send for Library {}
unsafe impl Sync for Library {}

impl Library {
    /// Open a library/executable/linker.
    ///
    /// # Arguments
    ///
    /// * `filename` - The path or name of the library to open.
    /// * `flags` - The flags to use. Usually `XDL_DEFAULT` or `XDL_TRY_FORCE_LOAD`.
    ///
    /// # Returns
    ///
    /// Returns `Ok(Library)` if successful, or `Err(String)` if the library could not be opened.
    pub fn open(filename: impl AsRef<str>, flags: u32) -> Result<Self, String> {
        let c_filename = CString::new(filename.as_ref()).map_err(|e| e.to_string())?;
        unsafe {
            // Clear any existing error
            // ffi::dlerror();

            let handle = ffi::xdl_open(c_filename.as_ptr(), flags as c_int);
            if !handle.is_null() {
                Ok(Library(NonNull::new_unchecked(handle)))
            } else {
                let error_ptr = ffi::dlerror();
                if !error_ptr.is_null() {
                    let error_msg = std::ffi::CStr::from_ptr(error_ptr)
                        .to_string_lossy()
                        .into_owned();
                    Err(error_msg)
                } else {
                    Err("Failed to open library: Unknown error".to_string())
                }
            }
        }
    }

    /// Find a symbol in the library.
    ///
    /// # Arguments
    ///
    /// * `symbol` - The name of the symbol to find.
    ///
    /// # Returns
    ///
    /// Returns `Some(*mut c_void)` if the symbol is found, or `None` otherwise.
    /// The pointer is raw and must be cast to the correct type by the caller.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the library handle is valid.
    pub unsafe fn sym(&self, symbol: &str) -> Option<*mut c_void> {
        let c_symbol = CString::new(symbol).ok()?;
        let mut size: usize = 0;
        let ptr = unsafe { ffi::xdl_sym(self.0.as_ptr(), c_symbol.as_ptr(), &mut size) };
        if ptr.is_null() { None } else { Some(ptr) }
    }

    /// Get a symbol from the library and cast it to the desired type.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the symbol exists and is of type `T`.
    pub unsafe fn get<T>(&self, symbol: &str) -> Option<T> {
        unsafe { self.sym(symbol).map(|ptr| std::mem::transmute_copy(&ptr)) }
    }

    /// Find a symbol in the library (using .dynsym only).
    ///
    /// # Arguments
    ///
    /// * `symbol` - The name of the symbol to find.
    ///
    /// # Returns
    ///
    /// Returns `Some(*mut c_void)` if the symbol is found, or `None` otherwise.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the library handle is valid.
    pub unsafe fn dsym(&self, symbol: &str) -> Option<*mut c_void> {
        let c_symbol = CString::new(symbol).ok()?;
        let mut size: usize = 0;
        let ptr = unsafe { ffi::xdl_dsym(self.0.as_ptr(), c_symbol.as_ptr(), &mut size) };
        if ptr.is_null() { None } else { Some(ptr) }
    }

    /// Get information about the library.
    ///
    /// Wrapper for `xdl_info`.
    pub fn info(&self, info: &mut xdl_info_t) -> Result<(), String> {
        let res = unsafe {
            ffi::xdl_info(
                self.0.as_ptr(),
                XDL_DI_DLINFO as c_int,
                info as *mut _ as *mut _,
            )
        };
        if res == 0 {
            Ok(())
        } else {
            Err("Failed to get library info".to_string())
        }
    }

    /// Get the raw handle.
    pub fn as_ptr(&self) -> *mut c_void {
        self.0.as_ptr()
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        unsafe {
            ffi::xdl_close(self.0.as_ptr());
        }
    }
}

/// Iterate over loaded shared objects.
///
/// This is a wrapper around `xdl_iterate_phdr`.
///
/// # Arguments
///
/// * `callback` - A callback function that is called for each loaded shared object.
///   Return non-zero to stop iteration.
/// * `flags` - Flags, e.g., `XDL_FULL_PATHNAME`.
pub fn iterate_phdr<F>(mut callback: F, flags: u32) -> i32
where
    F: FnMut(&dl_phdr_info, usize) -> i32,
{
    unsafe extern "C" fn trampoline<F>(
        info: *mut ffi::dl_phdr_info,
        size: usize,
        data: *mut c_void,
    ) -> c_int
    where
        F: FnMut(&dl_phdr_info, usize) -> i32,
    {
        let callback = unsafe { &mut *(data as *mut F) };
        // Bindgen might generate dl_phdr_info as a struct, pass reference.
        let info_ref = if info.is_null() {
            // Should not happen according to spec
            return 0;
        } else {
            unsafe { &*info }
        };

        callback(info_ref, size) as c_int
    }

    unsafe {
        ffi::xdl_iterate_phdr(
            Some(trampoline::<F>),
            &mut callback as *mut F as *mut c_void,
            flags as c_int,
        )
    }
}

/// Get information about an address.
///
/// Wrapper for `xdl_addr`.
///
/// # Safety
///
/// `addr` must be a valid pointer. `info` and `cache` must be valid references.
pub unsafe fn addr(addr: *mut c_void, info: &mut xdl_info_t, cache: &mut *mut c_void) -> i32 {
    unsafe { ffi::xdl_addr(addr, info, cache) }
}

/// Clean up the cache used by `addr`.
///
/// # Safety
///
/// `cache` must be a valid reference to a pointer that was previously used with `addr`.
pub unsafe fn addr_clean(cache: &mut *mut c_void) {
    unsafe { ffi::xdl_addr_clean(cache) }
}
