//! La identidad del teléfono (llave Ed25519, token, deployment y daemon) guardada donde el sistema
//! protege secretos:
//! - **iOS**: el Keychain (servicio `dev.condx.mobile`, accesible solo con el teléfono desbloqueado,
//!   y no se copia a otros dispositivos).
//! - **Android**: una llave AES-GCM creada dentro del Android Keystore (nunca sale de ahí) cifra el
//!   archivo de la identidad en la carpeta privada de la app.
//! - **Escritorio** (pruebas): un archivo en una carpeta temporal; nunca el llavero de la Mac.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub url: String,
    pub daemon_id: String,
    pub device_id: String,
    pub name: String,
    /// Token del teléfono para Condx Cloud (hex).
    pub token: String,
    /// Semilla de la llave Ed25519 (hex, 32 bytes).
    pub seed: String,
}

pub fn load() -> Result<Option<Identity>, String> {
    match platform::read()? {
        Some(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

pub fn save(identity: &Identity) -> Result<(), String> {
    platform::write(&serde_json::to_vec(identity).map_err(|e| e.to_string())?)
}

pub fn clear() -> Result<(), String> {
    platform::delete()
}

/// Dónde se guarda (para mostrarlo en la app).
pub fn backend() -> &'static str {
    platform::NAME
}

#[cfg(target_os = "ios")]
mod platform {
    use security_framework::passwords::{delete_generic_password, generic_password, set_generic_password_options, PasswordOptions};

    pub const NAME: &str = "iOS Keychain";
    const SERVICE: &str = "dev.condx.mobile";
    const ACCOUNT: &str = "identity";
    /// errSecItemNotFound
    const NOT_FOUND: i32 = -25300;

    pub fn read() -> Result<Option<Vec<u8>>, String> {
        match generic_password(PasswordOptions::new_generic_password(SERVICE, ACCOUNT)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.code() == NOT_FOUND => Ok(None),
            Err(e) => Err(format!("Keychain: {e}")),
        }
    }

    pub fn write(bytes: &[u8]) -> Result<(), String> {
        let _ = delete();
        let mut options = PasswordOptions::new_generic_password(SERVICE, ACCOUNT);
        // Solo desbloqueado y solo este dispositivo: no viaja en respaldos a otro teléfono.
        use security_framework::access_control::{ProtectionMode, SecAccessControl};
        let access = SecAccessControl::create_with_protection(Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly), 0).map_err(|e| format!("Keychain: {e}"))?;
        options.set_access_control(access);
        set_generic_password_options(bytes, options).map_err(|e| format!("Keychain: {e}"))
    }

    pub fn delete() -> Result<(), String> {
        match delete_generic_password(SERVICE, ACCOUNT) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == NOT_FOUND => Ok(()),
            Err(e) => Err(format!("Keychain: {e}")),
        }
    }
}

#[cfg(target_os = "android")]
mod platform {
    //! Android Keystore por JNI. nsh no expone su JavaVM, pero Android 11+ la entrega por
    //! `JNI_GetCreatedJavaVMs` (libnativehelper); las clases del framework se encuentran desde un
    //! hilo nativo adjuntado.

    use jni::objects::{JByteArray, JObject, JValue};
    use jni::{JNIEnv, JavaVM};

    pub const NAME: &str = "Android Keystore";
    const ALIAS: &str = "condx-identity";

    fn vm() -> Result<JavaVM, String> {
        type GetVms = unsafe extern "C" fn(*mut *mut jni::sys::JavaVM, jni::sys::jsize, *mut jni::sys::jsize) -> jni::sys::jint;
        unsafe {
            for lib in [c"libnativehelper.so", c"libart.so"] {
                let handle = libc::dlopen(lib.as_ptr(), libc::RTLD_NOW);
                if handle.is_null() {
                    continue;
                }
                let sym = libc::dlsym(handle, c"JNI_GetCreatedJavaVMs".as_ptr());
                if sym.is_null() {
                    continue;
                }
                let get: GetVms = std::mem::transmute(sym);
                let mut vm: *mut jni::sys::JavaVM = std::ptr::null_mut();
                let mut n: jni::sys::jsize = 0;
                if get(&mut vm, 1, &mut n) == 0 && n > 0 && !vm.is_null() {
                    return JavaVM::from_raw(vm).map_err(|e| e.to_string());
                }
            }
        }
        Err("couldn't reach the Java VM".into())
    }

    /// `/data/data/<paquete>/files/identity.bin` (el paquete sale de /proc/self/cmdline).
    fn path() -> Result<std::path::PathBuf, String> {
        let cmdline = std::fs::read("/proc/self/cmdline").map_err(|e| e.to_string())?;
        let package = String::from_utf8_lossy(cmdline.split(|b| *b == 0).next().unwrap_or_default()).to_string();
        let dir = std::path::PathBuf::from(format!("/data/data/{package}/files"));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(dir.join("identity.bin"))
    }

    fn jerr(e: jni::errors::Error) -> String {
        format!("Keystore: {e}")
    }

    /// La llave AES de la identidad, creándola en el Keystore si falta.
    fn key<'a>(env: &mut JNIEnv<'a>) -> Result<JObject<'a>, String> {
        let ks_type = env.new_string("AndroidKeyStore").map_err(jerr)?;
        let ks = env.call_static_method("java/security/KeyStore", "getInstance", "(Ljava/lang/String;)Ljava/security/KeyStore;", &[JValue::Object(&ks_type)]).and_then(|v| v.l()).map_err(jerr)?;
        env.call_method(&ks, "load", "(Ljava/security/KeyStore$LoadStoreParameter;)V", &[JValue::Object(&JObject::null())]).map_err(jerr)?;
        let alias = env.new_string(ALIAS).map_err(jerr)?;
        let has = env.call_method(&ks, "containsAlias", "(Ljava/lang/String;)Z", &[JValue::Object(&alias)]).and_then(|v| v.z()).map_err(jerr)?;
        if !has {
            let aes = env.new_string("AES").map_err(jerr)?;
            let kg = env
                .call_static_method("javax/crypto/KeyGenerator", "getInstance", "(Ljava/lang/String;Ljava/lang/String;)Ljavax/crypto/KeyGenerator;", &[JValue::Object(&aes), JValue::Object(&ks_type)])
                .and_then(|v| v.l())
                .map_err(jerr)?;
            // PURPOSE_ENCRYPT | PURPOSE_DECRYPT = 3
            let builder = env.new_object("android/security/keystore/KeyGenParameterSpec$Builder", "(Ljava/lang/String;I)V", &[JValue::Object(&alias), JValue::Int(3)]).map_err(jerr)?;
            let gcm = env.new_string("GCM").map_err(jerr)?;
            let modes = env.new_object_array(1, "java/lang/String", &gcm).map_err(jerr)?;
            env.call_method(&builder, "setBlockModes", "([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;", &[JValue::Object(&modes)]).map_err(jerr)?;
            let none = env.new_string("NoPadding").map_err(jerr)?;
            let pads = env.new_object_array(1, "java/lang/String", &none).map_err(jerr)?;
            env.call_method(&builder, "setEncryptionPaddings", "([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;", &[JValue::Object(&pads)]).map_err(jerr)?;
            let spec = env.call_method(&builder, "build", "()Landroid/security/keystore/KeyGenParameterSpec;", &[]).and_then(|v| v.l()).map_err(jerr)?;
            env.call_method(&kg, "init", "(Ljava/security/spec/AlgorithmParameterSpec;)V", &[JValue::Object(&spec)]).map_err(jerr)?;
            env.call_method(&kg, "generateKey", "()Ljavax/crypto/SecretKey;", &[]).map_err(jerr)?;
        }
        env.call_method(&ks, "getKey", "(Ljava/lang/String;[C)Ljava/security/Key;", &[JValue::Object(&alias), JValue::Object(&JObject::null())]).and_then(|v| v.l()).map_err(jerr)
    }

    fn cipher<'a>(env: &mut JNIEnv<'a>) -> Result<JObject<'a>, String> {
        let t = env.new_string("AES/GCM/NoPadding").map_err(jerr)?;
        env.call_static_method("javax/crypto/Cipher", "getInstance", "(Ljava/lang/String;)Ljavax/crypto/Cipher;", &[JValue::Object(&t)]).and_then(|v| v.l()).map_err(jerr)
    }

    pub fn write(bytes: &[u8]) -> Result<(), String> {
        let vm = vm()?;
        let mut env = vm.attach_current_thread().map_err(jerr)?;
        let key = key(&mut env)?;
        let c = cipher(&mut env)?;
        // ENCRYPT_MODE = 1; el IV lo elige el Keystore.
        env.call_method(&c, "init", "(ILjava/security/Key;)V", &[JValue::Int(1), JValue::Object(&key)]).map_err(jerr)?;
        let iv: JByteArray = env.call_method(&c, "getIV", "()[B", &[]).and_then(|v| v.l()).map_err(jerr)?.into();
        let input = env.byte_array_from_slice(bytes).map_err(jerr)?;
        let out: JByteArray = env.call_method(&c, "doFinal", "([B)[B", &[JValue::Object(&input)]).and_then(|v| v.l()).map_err(jerr)?.into();
        let iv = env.convert_byte_array(iv).map_err(jerr)?;
        let ct = env.convert_byte_array(out).map_err(jerr)?;
        let mut file = vec![iv.len() as u8];
        file.extend(iv);
        file.extend(ct);
        std::fs::write(path()?, file).map_err(|e| e.to_string())
    }

    pub fn read() -> Result<Option<Vec<u8>>, String> {
        let p = path()?;
        let Ok(file) = std::fs::read(&p) else { return Ok(None) };
        let iv_len = *file.first().ok_or("empty identity file")? as usize;
        let (iv, ct) = file[1..].split_at(iv_len.min(file.len() - 1));
        let vm = vm()?;
        let mut env = vm.attach_current_thread().map_err(jerr)?;
        let key = key(&mut env)?;
        let c = cipher(&mut env)?;
        let iv = env.byte_array_from_slice(iv).map_err(jerr)?;
        let spec = env.new_object("javax/crypto/spec/GCMParameterSpec", "(I[B)V", &[JValue::Int(128), JValue::Object(&iv)]).map_err(jerr)?;
        // DECRYPT_MODE = 2
        env.call_method(&c, "init", "(ILjava/security/Key;Ljava/security/spec/AlgorithmParameterSpec;)V", &[JValue::Int(2), JValue::Object(&key), JValue::Object(&spec)]).map_err(jerr)?;
        let input = env.byte_array_from_slice(ct).map_err(jerr)?;
        let out: JByteArray = env.call_method(&c, "doFinal", "([B)[B", &[JValue::Object(&input)]).and_then(|v| v.l()).map_err(jerr)?.into();
        env.convert_byte_array(out).map(Some).map_err(jerr)
    }

    pub fn delete() -> Result<(), String> {
        let _ = std::fs::remove_file(path()?);
        Ok(())
    }
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod platform {
    //! Escritorio: solo para pruebas y `cargo run`. Nunca el llavero de la Mac.
    pub const NAME: &str = "test file (desktop)";

    fn path() -> std::path::PathBuf {
        std::env::var_os("CONDX_MOBILE_IDENTITY").map(Into::into).unwrap_or_else(|| std::env::temp_dir().join("condx-mobile-identity.json"))
    }

    pub fn read() -> Result<Option<Vec<u8>>, String> {
        Ok(std::fs::read(path()).ok())
    }

    pub fn write(bytes: &[u8]) -> Result<(), String> {
        std::fs::write(path(), bytes).map_err(|e| e.to_string())
    }

    pub fn delete() -> Result<(), String> {
        let _ = std::fs::remove_file(path());
        Ok(())
    }
}

#[cfg(all(test, not(any(target_os = "ios", target_os = "android"))))]
mod tests {
    use super::*;

    #[test]
    fn guarda_lee_y_borra() {
        let dir = std::env::temp_dir().join(format!("condx-mobile-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: la prueba es la única que usa esta variable.
        unsafe { std::env::set_var("CONDX_MOBILE_IDENTITY", dir.join("id.json")) };
        clear().unwrap();
        assert_eq!(load().unwrap(), None);
        let id = Identity { url: "http://x".into(), daemon_id: "mac-1".into(), device_id: "phone-1".into(), name: "iPhone".into(), token: "t".repeat(64), seed: "0".repeat(64) };
        save(&id).unwrap();
        assert_eq!(load().unwrap(), Some(id));
        clear().unwrap();
        assert_eq!(load().unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
