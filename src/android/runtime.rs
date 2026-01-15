use anyhow::{Context, Result};
use jni::{
    JNIEnv, JavaVM,
    sys::{JNI_TRUE, JNI_VERSION_1_6, JavaVMInitArgs, jint, jsize},
};
use std::ffi::{CStr, CString, c_char, c_void};
use tracing::info;

use xdl_rs::Library;

const ANDROID_RUNTIME_DSO: &str = "libandroid_runtime.so";

#[repr(C)]
#[allow(non_snake_case)]
pub struct JniInvocationImpl {
    pub jni_provider_library_name: *const c_char,
    pub jni_provider_library: *mut c_void,

    pub JNI_GetDefaultJavaVMInitArgs: Option<unsafe extern "C" fn(*mut c_void) -> jint>,

    // jint (*JNI_CreateJavaVM)(JavaVM**, JNIEnv**, void*);
    pub JNI_CreateJavaVM: Option<
        unsafe extern "C" fn(
            *mut *mut jni::sys::JavaVM,
            *mut *mut jni::sys::JNIEnv,
            *mut c_void,
        ) -> jint,
    >,

    // jint (*JNI_GetCreatedJavaVMs)(JavaVM**, jsize, jsize*);
    pub JNI_GetCreatedJavaVMs:
        Option<unsafe extern "C" fn(*mut *mut jni::sys::JavaVM, jsize, *mut jsize) -> jint>,
}

impl std::fmt::Debug for JniInvocationImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let lib_name = if self.jni_provider_library_name.is_null() {
            "null".into()
        } else {
            unsafe { CStr::from_ptr(self.jni_provider_library_name) }.to_string_lossy()
        };

        f.debug_struct("JniInvocationImpl")
            .field("jni_provider_library_name", &lib_name)
            .field("jni_provider_library", &self.jni_provider_library)
            .field(
                "JNI_GetDefaultJavaVMInitArgs",
                &self.JNI_GetDefaultJavaVMInitArgs,
            )
            .field("JNI_CreateJavaVM", &self.JNI_CreateJavaVM)
            .field("JNI_GetCreatedJavaVMs", &self.JNI_GetCreatedJavaVMs)
            .finish()
    }
}

type JniInvocationCreate = unsafe extern "C" fn() -> *mut JniInvocationImpl;
type JniInvocationInit =
    unsafe extern "C" fn(*mut JniInvocationImpl, *const c_char) -> *mut JniInvocationImpl;
type StartReg = unsafe extern "C" fn(JNIEnv) -> jint;

type JNICreateJavaVM = unsafe extern "C" fn(
    *mut *mut jni::sys::JavaVM,
    *mut *mut jni::sys::JNIEnv,
    *mut c_void,
) -> jint;

pub struct AndroidRuntime {
    handle: Library,
}

impl AndroidRuntime {
    pub fn load() -> Result<Self> {
        Ok(Self {
            handle: Library::open(ANDROID_RUNTIME_DSO, xdl_rs::XDL_TRY_FORCE_LOAD)
                .map_err(|err| anyhow::anyhow!(err))
                .context("Failed to open libandroid_runtime.so")?,
        })
    }

    pub fn init_invocation(&self) -> Result<*const JniInvocationImpl> {
        unsafe {
            let create = self
                .handle
                .get::<JniInvocationCreate>("JniInvocationCreate")
                .ok_or(anyhow::anyhow!("JniInvocationCreate symbol not found"))?;

            let init = self
                .handle
                .get::<JniInvocationInit>("JniInvocationInit")
                .ok_or(anyhow::anyhow!("JniInvocationInit symbol not found"))?;

            // TODO: Fallback: manually find JniInvocation constructor and init method if symbols not found

            let invocation = create();

            if invocation.is_null() {
                anyhow::bail!("JniInvocationCreate returned null");
            }

            let lib_name = CString::new(ANDROID_RUNTIME_DSO).unwrap();
            init(invocation, lib_name.as_ptr());

            Ok(invocation)
        }
    }

    pub fn create_java_vm(&self) -> Result<JavaVM> {
        let jni_create_java_vm = unsafe {
            self.handle
                .get::<JNICreateJavaVM>("JNI_CreateJavaVM")
                .ok_or(anyhow::anyhow!("JNI_CreateJavaVM symbol not found"))?
        };

        info!("JNI_CreateJavaVM found at {:?}", jni_create_java_vm);

        let mut args = JavaVMInitArgs {
            version: JNI_VERSION_1_6,
            nOptions: 0,
            options: std::ptr::null_mut(),
            ignoreUnrecognized: JNI_TRUE,
        };

        let mut vm_ptr: *mut jni::sys::JavaVM = std::ptr::null_mut();
        let mut env_ptr: *mut jni::sys::JNIEnv = std::ptr::null_mut();

        let status = unsafe {
            jni_create_java_vm(
                &mut vm_ptr,
                &mut env_ptr,
                &mut args as *mut _ as *mut c_void,
            )
        };

        if status != 0 {
            anyhow::bail!("JNI_CreateJavaVM failed with status: {}", status);
        }

        info!("VM created successfully");

        // Patch AndroidRuntime::mJavaVM
        unsafe {
            let avm_ptr = self
                .handle
                .get::<*mut *mut c_void>("_ZN7android14AndroidRuntime7mJavaVME")
                .ok_or(anyhow::anyhow!(
                    "_ZN7android14AndroidRuntime7mJavaVME symbol not found"
                ))?;

            *avm_ptr = vm_ptr as *mut c_void;
            info!("Patched AndroidRuntime::mJavaVM");
        }

        let vm = unsafe { JavaVM::from_raw(vm_ptr)? };

        Ok(vm)
    }

    pub fn start_registration(&self, env: &mut JNIEnv) -> Result<()> {
        let start_reg = unsafe {
            self.handle
                .get::<StartReg>("_ZN7android14AndroidRuntime8startRegEP7_JNIEnv")
                .ok_or(anyhow::anyhow!(
                    "_ZN7android14AndroidRuntime8startRegEP7_JNIEnv symbol not found"
                ))?
        };

        // startReg expects a raw JNIEnv*
        unsafe {
            let result = start_reg(env.unsafe_clone());
            if result != 0 {
                anyhow::bail!("startReg failed with result: {}", result);
            }
        };

        info!("AndroidRuntime::startReg completed");
        Ok(())
    }
}
