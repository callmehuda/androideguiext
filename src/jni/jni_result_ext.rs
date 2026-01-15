use anyhow::{Context, Result};
use jni::objects::JString;
use jni::{JNIEnv, errors::Error};

fn get_java_exception(env: &mut JNIEnv) -> Result<String> {
    // A JavaException guarantees a pending exception; do not re-check.
    let exception = env
        .exception_occurred()
        .context("ExceptionOccurred failed")?;

    env.exception_clear().context("ExceptionClear failed")?;

    // Try getMessage() first (may be null)
    let message_obj = env
        .call_method(&exception, "getMessage", "()Ljava/lang/String;", &[])
        .context("Throwable.getMessage() failed")?
        .l()
        .ok();

    if let Some(obj) = message_obj {
        let jstr: JString = obj.into();
        return Ok(env.get_string(&jstr)?.into());
    }

    let to_string = env
        .call_method(exception, "toString", "()Ljava/lang/String;", &[])
        .context("Throwable.toString() failed")?
        .l()?;

    let jstr: JString = to_string.into();
    Ok(env.get_string(&jstr)?.into())
}

pub trait JniResultExt<T> {
    fn check_exception(self, env: &mut JNIEnv) -> Result<T>;
}

impl<T> JniResultExt<T> for Result<T, Error> {
    #[track_caller]
    fn check_exception(self, env: &mut JNIEnv) -> Result<T> {
        let caller = std::panic::Location::caller();

        match self {
            Ok(v) => Ok(v),

            Err(Error::JavaException) => {
                let msg = get_java_exception(env).unwrap_or_else(|e| {
                    tracing::warn!("Failed to extract Java exception: {e}");
                    "Java exception (failed to extract message)".to_owned()
                });

                Err(anyhow::anyhow!(msg))
                    .with_context(|| format!("at {}:{}", caller.file(), caller.line()))
            }

            Err(err) => Err(anyhow::Error::from(err))
                .with_context(|| format!("at {}:{}", caller.file(), caller.line())),
        }
    }
}
