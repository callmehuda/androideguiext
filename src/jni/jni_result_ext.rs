use anyhow::Result;
use jni::{JNIEnv, errors::Error};

fn get_java_exception(env: &mut JNIEnv) -> Result<String> {
    if env.exception_check()? {
        let exception = env.exception_occurred()?;
        env.exception_clear().ok();
        let message = env
            .call_method(exception, "getMessage", "()Ljava/lang/String;", &[])?
            .l()?;
        let rust_message: String = env.get_string(&message.into())?.into();
        return Ok(rust_message);
    }
    Err(anyhow::anyhow!("There is no exception"))
}

pub trait JniResultExt<T> {
    fn check_exception(self, env: &mut JNIEnv) -> Result<T>;
}

impl<T> JniResultExt<T> for Result<T, Error> {
    #[track_caller]
    fn check_exception(self, env: &mut JNIEnv) -> Result<T> {
        match self {
            Ok(v) => Ok(v),
            Err(err) => match err {
                jni::errors::Error::JavaException => {
                    let msg = match get_java_exception(env) {
                        Ok(msg) => msg,
                        Err(err) => {
                            tracing::warn!("Failed to get java exception : {err}");
                            "failed to get java exception".into()
                        }
                    };
                    anyhow::bail!("{msg}")
                }
                _ => Err(err.into()),
            },
        }
    }
}
