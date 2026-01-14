use anyhow::Result;
use jni::{
    JNIEnv,
    objects::{JIntArray, JObject, JValue},
};
use ndk::native_window::NativeWindow;

use crate::dex::util::inject_dex;

pub struct JavaBridge<'a> {
    main_class: jni::objects::JClass<'a>,
}

impl<'a> JavaBridge<'a> {
    pub fn new(env: &mut JNIEnv<'a>) -> Result<Self> {
        let dex_bytes = include_bytes!("../classes.dex");
        let cl = inject_dex(env, dex_bytes)?;
        let main_class = cl.find_class(env, "com.example.mylibrary.Main")?;
        Ok(Self { main_class })
    }

    pub fn call_main(&self, env: &mut JNIEnv<'a>) -> Result<()> {
        let string_cls = env.find_class("java/lang/String")?;
        let empty_array = env.new_object_array(0, string_cls, JObject::null())?;
        env.call_static_method(
            &self.main_class,
            "main",
            "([Ljava/lang/String;)V",
            &[JValue::Object(&empty_array)],
        )?;
        Ok(())
    }

    pub fn get_display_size(&self, env: &mut JNIEnv<'a>) -> Result<(i32, i32, i32)> {
        let display_info_array: JIntArray = env
            .call_static_method(&self.main_class, "getDisplayInfo", "()[I", &[])?
            .l()?
            .into();
        let mut buf = vec![0i32; 3];
        env.get_int_array_region(&display_info_array, 0, &mut buf)?;
        Ok((buf[0], buf[1], buf[2]))
    }

    pub fn create_native_window(
        &self,
        env: &mut JNIEnv<'a>,
        width: i32,
        height: i32,
    ) -> Result<NativeWindow> {
        let surface = env
            .call_static_method(
                &self.main_class,
                "createNativeWindow",
                "(IIZZ)Landroid/view/Surface;",
                &[
                    JValue::Int(width),
                    JValue::Int(height),
                    JValue::Bool(1), // isHide = true
                    JValue::Bool(0), // isSecure = false
                ],
            )?
            .l()?;

        let window = unsafe {
            NativeWindow::from_surface(env.get_raw(), surface.as_raw()).ok_or(anyhow::anyhow!(
                "Failed to create NativeWindow from surface"
            ))?
        };
        Ok(window)
    }
}
