//! Escanear el QR de la Mac (Configuración → Condx Cloud → Vincular un teléfono).
//!
//! nsh 0.2 no trae cámara ni deja pedir permisos o declarar intents en el manifiesto. En Android se
//! abre la cámara del sistema (`ACTION_IMAGE_CAPTURE`) con la foto hacia una fila propia de
//! MediaStore: así no hace falta el permiso CAMERA (la foto la toma la app de cámara) y la app la lee
//! al volver. El QR se decodifica aquí, en Rust (`rqrr`), y la foto se borra.
//! En iOS falta: la cámara pide `NSCameraUsageDescription` en el Info.plist, que nsh 0.2 no deja
//! declarar; ahí sigue pegar el texto del QR.

/// Hay cámara del sistema a la que pedirle la foto.
pub fn supported() -> bool {
    cfg!(target_os = "android")
}

/// En qué va un escaneo.
#[derive(Debug, Clone, PartialEq)]
pub enum Poll {
    /// Todavía no hay foto (la cámara está abierta).
    Waiting,
    /// El texto del QR.
    Done(String),
    Failed(String),
}

/// Lee el QR de una foto (JPEG o PNG). Prueba primero reducida (más rápido, y en fotos grandes
/// rqrr encuentra mejor los patrones) y después más grande.
pub fn decode(bytes: &[u8]) -> Result<String, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("couldn't read the photo: {e}"))?.to_luma8();
    for max in [1200u32, 2400] {
        let (w, h) = img.dimensions();
        let scaled = if w.max(h) > max {
            let k = max as f32 / w.max(h) as f32;
            image::imageops::resize(&img, (w as f32 * k) as u32, (h as f32 * k) as u32, image::imageops::FilterType::Triangle)
        } else {
            img.clone()
        };
        let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(scaled.width() as usize, scaled.height() as usize, |x, y| scaled.get_pixel(x as u32, y as u32).0[0]);
        for grid in prepared.detect_grids() {
            if let Ok((_, text)) = grid.decode() {
                return Ok(text);
            }
        }
        if w.max(h) <= max {
            break;
        }
    }
    Err("there's no QR in the photo: take it again, closer and in focus".into())
}

#[cfg(target_os = "android")]
mod platform {
    use std::sync::Mutex;

    use jni::objects::{JObject, JValue};
    use jni::JNIEnv;

    use super::Poll;

    /// La fila de MediaStore que espera la foto y cuántos bytes tenía la vez anterior (para no
    /// decodificar una foto a medio escribir).
    static PENDING: Mutex<Option<(String, usize)>> = Mutex::new(None);

    fn jerr(e: jni::errors::Error) -> String {
        format!("camera: {e}")
    }

    fn context<'a>(env: &mut JNIEnv<'a>) -> Result<JObject<'a>, String> {
        env.call_static_method("android/app/ActivityThread", "currentApplication", "()Landroid/app/Application;", &[]).and_then(|v| v.l()).map_err(jerr)
    }

    fn resolver<'a>(env: &mut JNIEnv<'a>) -> Result<JObject<'a>, String> {
        let ctx = context(env)?;
        env.call_method(&ctx, "getContentResolver", "()Landroid/content/ContentResolver;", &[]).and_then(|v| v.l()).map_err(jerr)
    }

    fn parse_uri<'a>(env: &mut JNIEnv<'a>, uri: &str) -> Result<JObject<'a>, String> {
        let s = env.new_string(uri).map_err(jerr)?;
        env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&s)]).and_then(|v| v.l()).map_err(jerr)
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

    pub fn start() -> Result<(), String> {
        cancel();
        let uri = with_env(|env| {
            let values = env.new_object("android/content/ContentValues", "()V", &[]).map_err(jerr)?;
            for (k, v) in [("_display_name", "condx-pairing.jpg"), ("mime_type", "image/jpeg")] {
                let (k, v) = (env.new_string(k).map_err(jerr)?, env.new_string(v).map_err(jerr)?);
                env.call_method(&values, "put", "(Ljava/lang/String;Ljava/lang/String;)V", &[JValue::Object(&k), JValue::Object(&v)]).map_err(jerr)?;
            }
            let table = env.get_static_field("android/provider/MediaStore$Images$Media", "EXTERNAL_CONTENT_URI", "Landroid/net/Uri;").and_then(|v| v.l()).map_err(jerr)?;
            let res = resolver(env)?;
            let uri = env.call_method(&res, "insert", "(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;", &[JValue::Object(&table), JValue::Object(&values)]).and_then(|v| v.l()).map_err(jerr)?;
            if uri.is_null() {
                return Err("couldn't make room for the photo".into());
            }
            let action = env.new_string("android.media.action.IMAGE_CAPTURE").map_err(jerr)?;
            let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)]).map_err(jerr)?;
            let extra = env.new_string("output").map_err(jerr)?;
            env.call_method(&intent, "putExtra", "(Ljava/lang/String;Landroid/os/Parcelable;)Landroid/content/Intent;", &[JValue::Object(&extra), JValue::Object(&uri)]).map_err(jerr)?;
            // NEW_TASK (se lanza desde la Application) y permiso de escribir y leer esa fila.
            env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(0x1000_0000 | 0x2 | 0x1)]).map_err(jerr)?;
            let ctx = context(env)?;
            if env.call_method(&ctx, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)]).is_err() {
                let _ = env.exception_clear();
                let _ = env.call_method(&res, "delete", "(Landroid/net/Uri;Ljava/lang/String;[Ljava/lang/String;)I", &[JValue::Object(&uri), JValue::Object(&JObject::null()), JValue::Object(&JObject::null())]);
                return Err("this phone has no camera app to take the photo".into());
            }
            let s = env.call_method(&uri, "toString", "()Ljava/lang/String;", &[]).and_then(|v| v.l()).map_err(jerr)?;
            Ok(String::from(env.get_string(&s.into()).map_err(jerr)?))
        })?;
        *PENDING.lock().unwrap() = Some((uri, 0));
        Ok(())
    }

    /// Los bytes de la foto (vacío si la cámara todavía no la escribió).
    fn read(uri: &str) -> Result<Vec<u8>, String> {
        with_env(|env| {
            let res = resolver(env)?;
            let u = parse_uri(env, uri)?;
            let Ok(stream) = env.call_method(&res, "openInputStream", "(Landroid/net/Uri;)Ljava/io/InputStream;", &[JValue::Object(&u)]).and_then(|v| v.l()) else {
                let _ = env.exception_clear();
                return Ok(vec![]);
            };
            let buf = env.new_byte_array(64 * 1024).map_err(jerr)?;
            let mut out = Vec::new();
            loop {
                let n = env.call_method(&stream, "read", "([B)I", &[JValue::Object(&buf)]).and_then(|v| v.i()).map_err(jerr)?;
                if n < 0 {
                    break;
                }
                let mut chunk = vec![0i8; n as usize];
                env.get_byte_array_region(&buf, 0, &mut chunk).map_err(jerr)?;
                out.extend(chunk.into_iter().map(|b| b as u8));
            }
            let _ = env.call_method(&stream, "close", "()V", &[]);
            Ok(out)
        })
    }

    pub fn poll() -> Poll {
        let Some((uri, last)) = PENDING.lock().unwrap().clone() else { return Poll::Failed("no scan in progress".into()) };
        let bytes = match read(&uri) {
            Ok(b) => b,
            Err(e) => return Poll::Failed(e),
        };
        // Espera dos lecturas iguales: la cámara pudo no terminar de escribir.
        if bytes.is_empty() || bytes.len() != last {
            if let Some(p) = PENDING.lock().unwrap().as_mut() {
                p.1 = bytes.len();
            }
            return Poll::Waiting;
        }
        let out = super::decode(&bytes);
        cancel();
        match out {
            Ok(t) => Poll::Done(t),
            Err(e) => Poll::Failed(e),
        }
    }

    /// Borra la fila (con o sin foto).
    pub fn cancel() {
        let Some((uri, _)) = PENDING.lock().unwrap().take() else { return };
        let _ = with_env(|env| {
            let res = resolver(env)?;
            let u = parse_uri(env, &uri)?;
            env.call_method(&res, "delete", "(Landroid/net/Uri;Ljava/lang/String;[Ljava/lang/String;)I", &[JValue::Object(&u), JValue::Object(&JObject::null()), JValue::Object(&JObject::null())]).map_err(jerr)?;
            Ok(())
        });
    }
}

#[cfg(not(target_os = "android"))]
mod platform {
    use super::Poll;

    pub fn start() -> Result<(), String> {
        Err("scanning isn't available here yet: paste the QR's text".into())
    }
    pub fn poll() -> Poll {
        Poll::Failed("scanning isn't available here".into())
    }
    pub fn cancel() {}
}

pub use platform::{cancel, poll, start};

#[cfg(test)]
mod tests {
    use super::*;

    /// Un QR como el de la Mac, dibujado con margen y escala, en JPEG.
    fn photo(text: &str, scale: u32, size: u32) -> Vec<u8> {
        let code = qrcode::QrCode::new(text.as_bytes()).unwrap();
        let w = code.width() as u32;
        let colors = code.to_colors();
        let mut img = image::GrayImage::from_pixel(size, size, image::Luma([235]));
        let off = (size - w * scale) / 2;
        for y in 0..w * scale {
            for x in 0..w * scale {
                if colors[((y / scale) * w + x / scale) as usize] == qrcode::Color::Dark {
                    img.put_pixel(off + x, off + y, image::Luma([20]));
                }
            }
        }
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    #[test]
    fn lee_el_qr_de_la_mac_en_una_foto() {
        let text = r#"{"condx":1,"url":"https://condx-cloud.serlismaldonado.com","code":"RXDU-W2QX-JUKT"}"#;
        // Pequeña y una foto grande (se reduce antes de buscar).
        assert_eq!(decode(&photo(text, 6, 400)).unwrap(), text);
        assert_eq!(decode(&photo(text, 40, 3000)).unwrap(), text);
        assert_eq!(crate::sync::parse_pairing(&decode(&photo(text, 6, 400)).unwrap(), "").unwrap().0, "https://condx-cloud.serlismaldonado.com");
    }

    #[test]
    fn sin_qr_lo_dice() {
        let mut out = std::io::Cursor::new(Vec::new());
        image::GrayImage::from_pixel(300, 300, image::Luma([128])).write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        assert!(decode(&out.into_inner()).unwrap_err().contains("no QR"));
        assert!(decode(b"not an image").is_err());
    }
}
