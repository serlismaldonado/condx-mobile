//! Notas de voz para los hilos: grabar con el micrófono (Android, `MediaRecorder` por JNI, AAC en
//! un .m4a dentro de la app) y devolver los bytes para subirlos como adjunto; el daemon lo transcribe.
//!
//! El permiso RECORD_AUDIO va en el manifiesto (lo agrega `scripts/android-apk.sh`, nsh 0.2 no deja
//! declararlo) y se pide en tiempo de ejecución a la actividad que está al frente; si Android no deja
//! llegar a ella, se abre la pantalla de permisos de la app para darlo a mano.

/// Hay micrófono al que grabar.
pub fn supported() -> bool {
    cfg!(target_os = "android")
}

/// El permiso del micrófono.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Permission {
    Granted,
    Missing,
}

/// Lo grabado: los bytes del .m4a y cuántos segundos duró.
pub struct Recording {
    pub bytes: Vec<u8>,
    pub seconds: f64,
}

/// Lo más que dura una nota de voz (luego se corta sola).
pub const MAX_SECONDS: f64 = 180.0;

#[cfg(target_os = "android")]
mod platform {
    use std::sync::Mutex;
    use std::time::Instant;

    use jni::objects::{GlobalRef, JObject, JValue};
    use jni::JNIEnv;

    use super::{Permission, Recording};

    struct Active {
        recorder: GlobalRef,
        path: std::path::PathBuf,
        started: Instant,
    }

    static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

    fn jerr(e: jni::errors::Error) -> String {
        format!("microphone: {e}")
    }

    /// El error de un paso, con la excepción de Java si la hubo.
    fn jx(env: &mut JNIEnv<'_>, step: &str, e: jni::errors::Error) -> String {
        let mut detail = e.to_string();
        if matches!(e, jni::errors::Error::JavaException) {
            if let Ok(ex) = env.exception_occurred() {
                let _ = env.exception_clear();
                if let Ok(s) = env.call_method(&ex, "toString", "()Ljava/lang/String;", &[]).and_then(|v| v.l()) {
                    if let Ok(text) = env.get_string(&s.into()) {
                        detail = String::from(text);
                    }
                }
            }
        }
        let _ = env.exception_clear();
        log::error!("audio: {step}: {detail}");
        format!("{step}: {detail}")
    }

    macro_rules! j {
        ($env:ident, $step:expr, $call:expr) => {{
            match $call {
                Ok(v) => v,
                Err(e) => return Err(jx($env, $step, e)),
            }
        }};
    }

    fn with_env<R>(f: impl FnOnce(&mut JNIEnv<'_>) -> Result<R, String>) -> Result<R, String> {
        let vm = crate::secure::platform::vm()?;
        let mut env = vm.attach_current_thread().map_err(jerr)?;
        let out = f(&mut env);
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        out
    }

    fn context<'a>(env: &mut JNIEnv<'a>) -> Result<JObject<'a>, String> {
        Ok(j!(env, "app context", env.call_static_method("android/app/ActivityThread", "currentApplication", "()Landroid/app/Application;", &[]).and_then(|v| v.l())))
    }

    /// La actividad al frente (por reflexión: nsh no la expone), o None si Android no deja llegar.
    fn activity<'a>(env: &mut JNIEnv<'a>) -> Option<JObject<'a>> {
        let r: Result<Option<JObject<'a>>, jni::errors::Error> = (|| {
            let at = env.call_static_method("android/app/ActivityThread", "currentActivityThread", "()Landroid/app/ActivityThread;", &[])?.l()?;
            let cls = env.get_object_class(&at)?;
            let name = env.new_string("mActivities")?;
            let field = env.call_method(&cls, "getDeclaredField", "(Ljava/lang/String;)Ljava/lang/reflect/Field;", &[JValue::Object(&name)])?.l()?;
            env.call_method(&field, "setAccessible", "(Z)V", &[JValue::Bool(1)])?;
            let map = env.call_method(&field, "get", "(Ljava/lang/Object;)Ljava/lang/Object;", &[JValue::Object(&at)])?.l()?;
            let values = env.call_method(&map, "values", "()Ljava/util/Collection;", &[])?.l()?;
            let iter = env.call_method(&values, "iterator", "()Ljava/util/Iterator;", &[])?.l()?;
            while env.call_method(&iter, "hasNext", "()Z", &[])?.z()? {
                let record = env.call_method(&iter, "next", "()Ljava/lang/Object;", &[])?.l()?;
                let rcls = env.get_object_class(&record)?;
                let aname = env.new_string("activity")?;
                let af = env.call_method(&rcls, "getDeclaredField", "(Ljava/lang/String;)Ljava/lang/reflect/Field;", &[JValue::Object(&aname)])?.l()?;
                env.call_method(&af, "setAccessible", "(Z)V", &[JValue::Bool(1)])?;
                let a = env.call_method(&af, "get", "(Ljava/lang/Object;)Ljava/lang/Object;", &[JValue::Object(&record)])?.l()?;
                if !a.is_null() {
                    return Ok(Some(a));
                }
            }
            Ok(None)
        })();
        match r {
            Ok(a) => a,
            Err(e) => {
                let _ = jx(env, "front activity", e);
                None
            }
        }
    }

    pub fn permission() -> Permission {
        with_env(|env| {
            let ctx = context(env)?;
            let p = j!(env, "permission", env.new_string("android.permission.RECORD_AUDIO"));
            let r = j!(env, "permission", env.call_method(&ctx, "checkSelfPermission", "(Ljava/lang/String;)I", &[JValue::Object(&p)]).and_then(|v| v.i()));
            Ok(if r == 0 { Permission::Granted } else { Permission::Missing })
        })
        .unwrap_or(Permission::Missing)
    }

    /// Pide el permiso a la actividad (el diálogo de Android); si no se puede, abre los permisos de
    /// la app. La respuesta no vuelve aquí: quien llama vuelve a mirar `permission()`.
    pub fn request_permission() -> Result<(), String> {
        with_env(|env| {
            if let Some(a) = activity(env) {
                let arr = j!(env, "permission", env.new_object_array(1, "java/lang/String", JObject::null()));
                let p = j!(env, "permission", env.new_string("android.permission.RECORD_AUDIO"));
                j!(env, "permission", env.set_object_array_element(&arr, 0, &p));
                if env.call_method(&a, "requestPermissions", "([Ljava/lang/String;I)V", &[JValue::Object(&arr), JValue::Int(7)]).is_ok() {
                    return Ok(());
                }
                let _ = env.exception_clear();
            }
            // Los ajustes de la app: Permisos → Micrófono.
            let ctx = context(env)?;
            let action = j!(env, "settings", env.new_string("android.settings.APPLICATION_DETAILS_SETTINGS"));
            let pkg = j!(env, "settings", env.call_method(&ctx, "getPackageName", "()Ljava/lang/String;", &[]).and_then(|v| v.l()));
            let scheme = j!(env, "settings", env.new_string("package"));
            let uri = j!(env, "settings", env.call_static_method("android/net/Uri", "fromParts", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&scheme), JValue::Object(&pkg), JValue::Object(&JObject::null())]).and_then(|v| v.l()));
            let intent = j!(env, "settings", env.new_object("android/content/Intent", "(Ljava/lang/String;Landroid/net/Uri;)V", &[JValue::Object(&action), JValue::Object(&uri)]));
            j!(env, "settings", env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(0x1000_0000)]));
            j!(env, "settings", env.call_method(&ctx, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)]));
            Ok(())
        })
    }

    pub fn start() -> Result<(), String> {
        cancel();
        let (recorder, path) = with_env(|env| {
            let ctx = context(env)?;
            let dir = j!(env, "files dir", env.call_method(&ctx, "getCacheDir", "()Ljava/io/File;", &[]).and_then(|v| v.l()));
            let dir_s = j!(env, "files dir", env.call_method(&dir, "getAbsolutePath", "()Ljava/lang/String;", &[]).and_then(|v| v.l()));
            let dir_s: jni::objects::JString = dir_s.into();
            let dir: String = j!(env, "files dir", env.get_string(&dir_s)).into();
            let path = std::path::PathBuf::from(dir).join(format!("voice-{}.m4a", crate::sync::now_ms()));
            // Android 12+: MediaRecorder(Context); antes, el constructor sin argumentos.
            let rec = match env.new_object("android/media/MediaRecorder", "(Landroid/content/Context;)V", &[JValue::Object(&ctx)]) {
                Ok(r) => r,
                Err(_) => {
                    let _ = env.exception_clear();
                    j!(env, "recorder", env.new_object("android/media/MediaRecorder", "()V", &[]))
                }
            };
            // MIC, MPEG_4, AAC, 64 kbps mono a 44,1 kHz: chico y claro para transcribir.
            j!(env, "audio source", env.call_method(&rec, "setAudioSource", "(I)V", &[JValue::Int(1)]));
            j!(env, "format", env.call_method(&rec, "setOutputFormat", "(I)V", &[JValue::Int(2)]));
            j!(env, "encoder", env.call_method(&rec, "setAudioEncoder", "(I)V", &[JValue::Int(3)]));
            j!(env, "bitrate", env.call_method(&rec, "setAudioEncodingBitRate", "(I)V", &[JValue::Int(64_000)]));
            j!(env, "sample rate", env.call_method(&rec, "setAudioSamplingRate", "(I)V", &[JValue::Int(44_100)]));
            j!(env, "channels", env.call_method(&rec, "setAudioChannels", "(I)V", &[JValue::Int(1)]));
            let p = j!(env, "output", env.new_string(path.to_string_lossy()));
            j!(env, "output", env.call_method(&rec, "setOutputFile", "(Ljava/lang/String;)V", &[JValue::Object(&p)]));
            j!(env, "prepare", env.call_method(&rec, "prepare", "()V", &[]));
            j!(env, "start recording", env.call_method(&rec, "start", "()V", &[]));
            let g = j!(env, "recorder", env.new_global_ref(&rec));
            Ok((g, path))
        })?;
        *ACTIVE.lock().unwrap() = Some(Active { recorder, path, started: Instant::now() });
        Ok(())
    }

    fn release(a: &Active, stop: bool) -> Result<(), String> {
        with_env(|env| {
            let r = a.recorder.as_obj();
            let stopped = if stop { env.call_method(r, "stop", "()V", &[]).map(|_| ()) } else { Ok(()) };
            let _ = env.exception_clear();
            let _ = env.call_method(r, "release", "()V", &[]);
            let _ = env.exception_clear();
            stopped.map_err(|e| jx(env, "stop recording", e))
        })
    }

    pub fn stop() -> Result<Recording, String> {
        let a = ACTIVE.lock().unwrap().take().ok_or("not recording")?;
        let seconds = a.started.elapsed().as_secs_f64();
        let stopped = release(&a, true);
        let bytes = std::fs::read(&a.path).map_err(|e| format!("couldn't read the recording: {e}"));
        let _ = std::fs::remove_file(&a.path);
        stopped?;
        let bytes = bytes?;
        if bytes.is_empty() {
            return Err("nothing was recorded".into());
        }
        Ok(Recording { bytes, seconds })
    }

    pub fn cancel() {
        if let Some(a) = ACTIVE.lock().unwrap().take() {
            let _ = release(&a, true);
            let _ = std::fs::remove_file(&a.path);
        }
    }

    pub fn elapsed() -> Option<f64> {
        ACTIVE.lock().unwrap().as_ref().map(|a| a.started.elapsed().as_secs_f64())
    }
}

#[cfg(not(target_os = "android"))]
mod platform {
    use super::{Permission, Recording};

    pub fn permission() -> Permission {
        Permission::Missing
    }
    pub fn request_permission() -> Result<(), String> {
        Err("there's no microphone here".into())
    }
    pub fn start() -> Result<(), String> {
        Err("there's no microphone here".into())
    }
    pub fn stop() -> Result<Recording, String> {
        Err("not recording".into())
    }
    pub fn cancel() {}
    pub fn elapsed() -> Option<f64> {
        None
    }
}

pub use platform::{cancel, elapsed, permission, request_permission, start, stop};
