//! Escanear el QR de la Mac (Configuración → Condx Cloud → Vincular un teléfono).
//!
//! nsh 0.2 no trae cámara ni deja pedir permisos o declarar intents en el manifiesto. En Android se
//! abre la cámara del sistema (`ACTION_IMAGE_CAPTURE`) con la foto hacia una fila propia de
//! MediaStore: así no hace falta el permiso CAMERA (la foto la toma la app de cámara) y la app la lee
//! al volver (también si Android cerró la app mientras tanto). El QR se decodifica aquí, en Rust
//! (rxing y, si no, rqrr), y la foto se borra. Si la cámara del teléfono no devuelve la foto, la
//! suya (o Lens) lee el QR y copia el texto: «Paste the QR's text» lo toma del portapapeles.
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

fn scaled(img: &image::GrayImage, max: u32) -> Option<image::GrayImage> {
    let (w, h) = img.dimensions();
    if w.max(h) <= max {
        return None;
    }
    let k = max as f32 / w.max(h) as f32;
    Some(image::imageops::resize(img, (w as f32 * k) as u32, (h as f32 * k) as u32, image::imageops::FilterType::Triangle))
}

/// ZXing (rxing): el que mejor lee fotos de una pantalla (moiré, desenfoque, perspectiva).
fn with_rxing(img: &image::GrayImage) -> Option<String> {
    let mut hints = rxing::DecodeHints::default().with(rxing::DecodeHintValue::TryHarder(true)).with(rxing::DecodeHintValue::PossibleFormats([rxing::BarcodeFormat::QR_CODE].into_iter().collect()));
    rxing::helpers::detect_in_luma_with_hints(img.as_raw().clone(), img.width(), img.height(), None, &mut hints).ok().map(|r| r.getText().to_string())
}

fn with_rqrr(img: &image::GrayImage) -> Option<String> {
    let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(img.width() as usize, img.height() as usize, |x, y| img.get_pixel(x as u32, y as u32).0[0]);
    prepared.detect_grids().into_iter().find_map(|g| g.decode().ok().map(|(_, t)| t))
}

/// Lee el QR de una foto (JPEG o PNG). Una foto del teléfono tiene 12 MP o más: se busca en varias
/// escalas (reducida es más rápido y suaviza el moiré; grande sirve si el QR salió pequeño), primero
/// con rxing y, si no, con rqrr. En 40 fotos sintéticas de la pantalla, rxing leyó 39 y rqrr 34.
pub fn decode(bytes: &[u8]) -> Result<String, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("couldn't read the photo ({e}); try “Paste the QR's text”"))?.to_luma8();
    let sizes: Vec<image::GrayImage> = [1000u32, 1600, 2400].iter().filter_map(|m| scaled(&img, *m)).collect();
    let all = || sizes.iter().chain(std::iter::once(&img));
    if let Some(t) = all().find_map(with_rxing).or_else(|| all().find_map(with_rqrr)) {
        return Ok(t);
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
    /// decodificar una foto a medio escribir). También va a un archivo: Android puede cerrar la app
    /// mientras la cámara está abierta, y al volver el escaneo sigue.
    static PENDING: Mutex<Option<(String, usize)>> = Mutex::new(None);

    fn pending_file() -> Option<std::path::PathBuf> {
        let cmdline = std::fs::read("/proc/self/cmdline").ok()?;
        let package = String::from_utf8_lossy(cmdline.split(|b| *b == 0).next().unwrap_or_default()).to_string();
        Some(std::path::PathBuf::from(format!("/data/data/{package}/files/scan-pending.txt")))
    }

    fn load() -> Option<(String, usize)> {
        let mut p = PENDING.lock().unwrap();
        if p.is_none() {
            *p = pending_file().and_then(|f| std::fs::read_to_string(f).ok()).map(|u| u.trim().to_string()).filter(|u| u.starts_with("content://")).map(|u| (u, 0));
        }
        p.clone()
    }

    pub fn pending() -> bool {
        load().is_some()
    }

    /// El texto del portapapeles (la cámara del teléfono o Lens leen el QR y lo copian).
    pub fn clipboard() -> Result<String, String> {
        with_env(|env| {
            let ctx = context(env)?;
            let name = env.new_string("clipboard").map_err(jerr)?;
            let cm = env.call_method(&ctx, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&name)]).and_then(|v| v.l()).map_err(jerr)?;
            let clip = env.call_method(&cm, "getPrimaryClip", "()Landroid/content/ClipData;", &[]).and_then(|v| v.l()).map_err(jerr)?;
            if clip.is_null() {
                return Err("the clipboard is empty: copy the QR's text first".into());
            }
            let item = env.call_method(&clip, "getItemAt", "(I)Landroid/content/ClipData$Item;", &[JValue::Int(0)]).and_then(|v| v.l()).map_err(jerr)?;
            let text = env.call_method(&item, "coerceToText", "(Landroid/content/Context;)Ljava/lang/CharSequence;", &[JValue::Object(&ctx)]).and_then(|v| v.l()).map_err(jerr)?;
            let s = env.call_method(&text, "toString", "()Ljava/lang/String;", &[]).and_then(|v| v.l()).map_err(jerr)?;
            Ok(String::from(env.get_string(&s.into()).map_err(jerr)?))
        })
    }

    fn jerr(e: jni::errors::Error) -> String {
        format!("camera: {e}")
    }

    /// El error de un paso, con la excepción de Java si la hubo (clase y mensaje), y al log.
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
        log::error!("scan: {step}: {detail}");
        format!("{step}: {detail}")
    }

    /// `Ok(valor)` o devuelve el error del paso con la excepción de Java.
    macro_rules! j {
        ($env:ident, $step:expr, $call:expr) => {{
            let r = $call;
            match r {
                Ok(v) => v,
                Err(e) => return Err(jx($env, $step, e)),
            }
        }};
    }

    fn context<'a>(env: &mut JNIEnv<'a>) -> Result<JObject<'a>, String> {
        let app = env.call_static_method("android/app/ActivityThread", "currentApplication", "()Landroid/app/Application;", &[]).and_then(|v| v.l());
        match app {
            Ok(a) if !a.is_null() => Ok(a),
            other => {
                if let Err(e) = other {
                    let _ = jx(env, "app context", e);
                }
                let at = j!(env, "app context", env.call_static_method("android/app/ActivityThread", "currentActivityThread", "()Landroid/app/ActivityThread;", &[]).and_then(|v| v.l()));
                Ok(j!(env, "app context", env.call_method(&at, "getApplication", "()Landroid/app/Application;", &[]).and_then(|v| v.l())))
            }
        }
    }

    /// Una fila de MediaStore para la foto: la tabla de siempre o, si falla, la del volumen principal.
    fn new_row<'a>(env: &mut JNIEnv<'a>, res: &JObject<'a>) -> Result<JObject<'a>, String> {
        let values = j!(env, "photo row", env.new_object("android/content/ContentValues", "()V", &[]));
        for (k, v) in [("_display_name", "condx-pairing.jpg"), ("mime_type", "image/jpeg")] {
            let (k, v) = (j!(env, "photo row", env.new_string(k)), j!(env, "photo row", env.new_string(v)));
            j!(env, "photo row", env.call_method(&values, "put", "(Ljava/lang/String;Ljava/lang/String;)V", &[JValue::Object(&k), JValue::Object(&v)]));
        }
        let mut last = String::from("photo row: no table");
        for vol in [None, Some("external_primary"), Some("external")] {
            let table = match vol {
                None => env.get_static_field("android/provider/MediaStore$Images$Media", "EXTERNAL_CONTENT_URI", "Landroid/net/Uri;").and_then(|v| v.l()),
                Some(v) => match env.new_string(v) {
                    Ok(name) => env.call_static_method("android/provider/MediaStore$Images$Media", "getContentUri", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&name)]).and_then(|v| v.l()),
                    Err(e) => Err(e),
                },
            };
            let table = match table {
                Ok(t) => t,
                Err(e) => {
                    last = jx(env, "photo row", e);
                    continue;
                }
            };
            match env.call_method(res, "insert", "(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;", &[JValue::Object(&table), JValue::Object(&values)]).and_then(|v| v.l()) {
                Ok(uri) if !uri.is_null() => return Ok(uri),
                Ok(_) => last = "photo row: MediaStore gave no row".into(),
                Err(e) => last = jx(env, "photo row", e),
            }
        }
        Err(last)
    }

    /// Abre la cámara sola (sin foto de vuelta): su lector de QR muestra el texto para copiarlo.
    pub fn open_camera_app() -> Result<(), String> {
        with_env(|env| {
            let ctx = context(env)?;
            let action = j!(env, "camera", env.new_string("android.media.action.STILL_IMAGE_CAMERA"));
            let intent = j!(env, "camera", env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)]));
            j!(env, "camera", env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(0x1000_0000)]));
            j!(env, "open the camera", env.call_method(&ctx, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)]));
            Ok(())
        })
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
            let ctx = context(env)?;
            let res = j!(env, "content resolver", env.call_method(&ctx, "getContentResolver", "()Landroid/content/ContentResolver;", &[]).and_then(|v| v.l()));
            let uri = new_row(env, &res)?;
            let drop_row = |env: &mut JNIEnv<'_>| {
                let _ = env.call_method(&res, "delete", "(Landroid/net/Uri;Ljava/lang/String;[Ljava/lang/String;)I", &[JValue::Object(&uri), JValue::Object(&JObject::null()), JValue::Object(&JObject::null())]);
                let _ = env.exception_clear();
            };
            let action = j!(env, "camera intent", env.new_string("android.media.action.IMAGE_CAPTURE"));
            let intent = j!(env, "camera intent", env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)]));
            let extra = j!(env, "camera intent", env.new_string("output"));
            j!(env, "camera intent", env.call_method(&intent, "putExtra", "(Ljava/lang/String;Landroid/os/Parcelable;)Landroid/content/Intent;", &[JValue::Object(&extra), JValue::Object(&uri)]));
            // NEW_TASK (se lanza desde la Application) y permiso de escribir y leer esa fila.
            j!(env, "camera intent", env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(0x1000_0000 | 0x2 | 0x1)]));
            if let Err(e) = env.call_method(&ctx, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)]) {
                let msg = jx(env, "open the camera", e);
                drop_row(env);
                return Err(msg);
            }
            let s: jni::objects::JString = j!(env, "photo row", env.call_method(&uri, "toString", "()Ljava/lang/String;", &[]).and_then(|v| v.l())).into();
            let text = j!(env, "photo row", env.get_string(&s).map(String::from));
            Ok(text)
        })?;
        if let Some(f) = pending_file() {
            let _ = std::fs::write(f, &uri);
        }
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
        let Some((uri, last)) = load() else { return Poll::Failed("no scan in progress".into()) };
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
        let _ = load();
        if let Some(f) = pending_file() {
            let _ = std::fs::remove_file(f);
        }
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
    pub fn pending() -> bool {
        false
    }
    pub fn clipboard() -> Result<String, String> {
        Err("paste the QR's text in the field".into())
    }
    pub fn open_camera_app() -> Result<(), String> {
        Err("no camera here".into())
    }
}

pub use platform::{cancel, clipboard, open_camera_app, pending, poll, start};

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

    /// Una foto sintética de la pantalla de la Mac (perspectiva, giro, moiré, ruido, desenfoque)
    /// que rqrr solo no leía.
    #[test]
    fn lee_una_foto_dificil_de_la_pantalla() {
        let photo = include_bytes!("../tests/fixtures/qr-screen-photo.jpg");
        assert_eq!(decode(photo).unwrap(), r#"{"condx":1,"url":"https://condx-cloud.serlismaldonado.com","code":"RXDU-W2QX-JUKT"}"#);
    }

    #[test]
    fn sin_qr_lo_dice() {
        let mut out = std::io::Cursor::new(Vec::new());
        image::GrayImage::from_pixel(300, 300, image::Luma([128])).write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        assert!(decode(&out.into_inner()).unwrap_err().contains("no QR"));
        assert!(decode(b"not an image").is_err());
    }
}
