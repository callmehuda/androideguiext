use anyhow::Result;
use jni::{
    JNIEnv,
    objects::{JClass, JObject, JValue},
};

use crate::jni::jni_result_ext::JniResultExt;

pub mod util;

#[derive(Debug)]
pub struct ClassLoader<'local>(pub JObject<'local>);

impl<'local> ClassLoader<'local> {
    pub fn find_class_as_object(
        &self,
        env: &mut JNIEnv<'local>,
        class_name: &str,
    ) -> Result<JObject<'local>> {
        let cls = env
            .call_method(
                &self.0,
                "findClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(&env.new_string(class_name).unwrap())],
            )
            .check_exception(env)?
            .l()?;
        Ok(cls)
    }

    pub fn find_class(&self, env: &mut JNIEnv<'local>, class_name: &str) -> Result<JClass<'local>> {
        Ok(JClass::from(self.find_class_as_object(env, class_name)?))
    }
}

impl<'local> std::ops::Deref for ClassLoader<'local> {
    type Target = JObject<'local>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'local> std::ops::DerefMut for ClassLoader<'local> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
