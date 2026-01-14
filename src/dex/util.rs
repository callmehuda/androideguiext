use anyhow::{Context, Result};
use jni::{JNIEnv, objects::JValue};

use crate::{android::get_api_level, dex::ClassLoader, jni::jni_result_ext::JniResultExt};

pub fn inject_dex<'local>(
    env: &mut JNIEnv<'local>,
    dex_bytes: &[u8],
) -> Result<ClassLoader<'local>> {
    let api = get_api_level().context("getting android version")?;
    debug_assert_ne!(api, 0);
    if api >= 26 {
        tracing::info!("Injecting dex from memory");
        return load_dex_from_memory(env, dex_bytes);
    }
    unimplemented!("Loading dex not not implemented for api level {api}");
}

fn load_dex_from_memory<'local>(
    env: &mut JNIEnv<'local>,
    dex_bytes: &[u8],
) -> Result<ClassLoader<'local>> {
    let dex_byte_array = env
        .new_byte_array(dex_bytes.len() as _)
        .check_exception(env)?;

    let dex_bytes_i8: &[i8] =
        unsafe { std::slice::from_raw_parts(dex_bytes.as_ptr() as *const i8, dex_bytes.len()) };
    env.set_byte_array_region(&dex_byte_array, 0, dex_bytes_i8)
        .check_exception(env)?;

    let byte_buffer_class = env.find_class("java/nio/ByteBuffer").check_exception(env)?;

    let dex_byte_buffer = env
        .call_static_method(
            &byte_buffer_class,
            "wrap",
            "([B)Ljava/nio/ByteBuffer;",
            &[JValue::Object(&*dex_byte_array)],
        )
        .check_exception(env)?
        .l()?;

    let dex_buffers = env
        .new_object_array(1, byte_buffer_class, dex_byte_buffer)
        .check_exception(env)?;

    let in_memory_dex_class_loader_class = env
        .find_class("dalvik/system/InMemoryDexClassLoader")
        .check_exception(env)?;

    let class_loader_class = env
        .find_class("java/lang/ClassLoader")
        .check_exception(env)?;

    let system_class_loader = env
        .call_static_method(
            class_loader_class,
            "getSystemClassLoader",
            "()Ljava/lang/ClassLoader;",
            &[],
        )
        .check_exception(env)?
        .l()?;

    let class_loader = env
        .new_object(
            in_memory_dex_class_loader_class,
            "([Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V",
            &[
                JValue::Object(&dex_buffers),
                JValue::Object(&system_class_loader),
            ],
        )
        .check_exception(env)?;

    Ok(ClassLoader(class_loader))
}
