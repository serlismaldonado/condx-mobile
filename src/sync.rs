//! Lo que el teléfono hace fuera de la interfaz: el motor local-first contra Condx Cloud y la firma
//! de las decisiones. Se prueba sin pantalla (`cargo test`).

use std::sync::{Arc, Mutex};

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use localfirst_core::http::{ConvexHttpTransport, Doorbell};
use localfirst_core::{sqlite, sync_once, Engine, EngineConfig, OpDraft, Values, PROTOCOL_VERSION};
use serde_json::{json, Value};

pub type CloudEngine = Engine<sqlite::SqliteReplicaStore, sqlite::SqliteOutboxStore>;

/// Una aprobación pendiente como la ve el teléfono.
#[derive(Debug, Clone, PartialEq)]
pub struct Approval {
    pub job_id: String,
    pub recipe: String,
    pub title: String,
    pub env: String,
    /// `devbox`, `throwaway`, `cloud-dev` o `cloud-prod`.
    pub env_kind: String,
    pub status: String,
    pub level: String,
    pub requested_by: String,
    /// Cuándo se pidió (ms; 0 si no se sabe).
    pub created_at: i64,
    pub decision: Option<String>,
    /// Qué hizo el daemon con la decisión: `approved`, `rejected: …`, `needs-mac`.
    pub phone_result: Option<String>,
    pub pending: bool,
}

/// Un proveedor cuya clave el daemon cifró para este teléfono.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderKey {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub model: String,
    pub key_hint: String,
    /// Se descifra con la identidad de este teléfono.
    pub opens: bool,
}

/// Un incidente como lo ve el teléfono.
#[derive(Debug, Clone, PartialEq)]
pub struct Incident {
    pub id: String,
    pub title: String,
    pub source: String,
    pub env: String,
    pub status: String,
    pub count: i64,
    pub last_seen: i64,
    pub error: String,
    pub log: String,
    /// `{runId, mode, status, template, model, answer, fix: {branch, commit, files, stat, changes, discarded}}`.
    pub agent: Value,
    /// Los flujos encendidos del proyecto, para desplegar un arreglo: (id, nombre).
    pub workflows: Vec<(String, String)>,
    /// Qué hizo el daemon con el último pedido de este teléfono.
    pub result: Option<String>,
    /// Hay un pedido propio sin respuesta todavía.
    pub waiting: bool,
}

/// Lo que firma el teléfono para pedir algo sobre un incidente (mismo texto en el daemon).
pub fn request_message(daemon_id: &str, incident_id: &str, request: &str, request_id: &str, at: i64, arg: &str) -> String {
    format!("condx-incident:v1|{daemon_id}|{incident_id}|{request}|{request_id}|{at}|{arg}")
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Lo que firma el teléfono: versión, daemon, trabajo, decisión y hora. El daemon arma el mismo texto
/// y verifica con la llave pública del teléfono vinculado.
pub fn decision_message(daemon_id: &str, job_id: &str, decision: &str, decided_at: i64) -> String {
    format!("condx-approval:v1|{daemon_id}|{job_id}|{decision}|{decided_at}")
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    (0..text.len()).step_by(2).map(|i| text.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok())).collect()
}

/// Lo que haría el daemon con una decisión que llega: firma de la llave que conoce, para ese
/// trabajo, y reciente (`max_age_ms`).
pub fn verify_decision(daemon_id: &str, job_id: &str, row: &Values, device_key: &VerifyingKey, now: i64, max_age_ms: i64) -> Result<String, String> {
    let decision = row.get("decision").and_then(Value::as_str).ok_or("no decision")?;
    let at = row.get("decidedAt").and_then(Value::as_i64).ok_or("no time")?;
    if row.get("deviceKey").and_then(Value::as_str) != Some(hex(device_key.as_bytes()).as_str()) {
        return Err("unknown device".into());
    }
    if now - at > max_age_ms || at > now + 60_000 {
        return Err("stale decision".into());
    }
    let sig = row.get("signature").and_then(Value::as_str).and_then(unhex).ok_or("no signature")?;
    let sig = ed25519_dalek::Signature::from_slice(&sig).map_err(|_| "bad signature")?;
    device_key.verify(decision_message(daemon_id, job_id, decision, at).as_bytes(), &sig).map_err(|_| "signature doesn't match".to_string())?;
    Ok(decision.to_string())
}

/// Llama a una función pública de Condx Cloud (vincular).
fn call(url: &str, kind: &str, path: &str, args: Value) -> Result<Value, String> {
    let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(20)).build();
    let reply: Value = match agent.post(&format!("{}/api/{kind}", url.trim_end_matches('/'))).send_json(json!({ "path": path, "args": args, "format": "json" })) {
        Ok(r) => r.into_json().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(_, r)) => r.into_json().unwrap_or(Value::Null),
        Err(e) => return Err(format!("couldn't reach Condx Cloud: {e}")),
    };
    if reply["status"] == "success" {
        return Ok(reply["value"].clone());
    }
    Err(reply["errorData"]["message"].as_str().map(str::to_string).unwrap_or_else(|| reply["errorMessage"].as_str().unwrap_or("error").lines().next().unwrap_or("error").to_string()))
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut buf);
    hex(&buf)
}

/// Lo que trae el QR de la Mac: `{"condx":1,"url":…,"code":…}`. También acepta solo el código si
/// se pasa la URL aparte.
pub fn parse_pairing(text: &str, url: &str) -> Result<(String, String), String> {
    let text = text.trim();
    // El enlace del QR (`https://<sitio>/pair#d=<deployment>&c=<código>`).
    if let Some((_, frag)) = text.split_once("/pair#").filter(|_| text.starts_with("http")) {
        let decode = |s: &str| {
            let b = s.as_bytes();
            let mut out = Vec::with_capacity(b.len());
            let mut i = 0;
            while i < b.len() {
                match (b[i], b.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())) {
                    (b'%', Some(v)) => {
                        out.push(v);
                        i += 3;
                    }
                    (c, _) => {
                        out.push(c);
                        i += 1;
                    }
                }
            }
            String::from_utf8_lossy(&out).to_string()
        };
        let get = |k: &str| frag.split('&').find_map(|kv| kv.strip_prefix(k).and_then(|v| v.strip_prefix('=')).map(decode));
        let url = get("d").ok_or("the QR's link has no deployment URL")?;
        let code = get("c").ok_or("the QR's link has no code")?;
        return Ok((url, code));
    }
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        let url = v["url"].as_str().ok_or("the QR has no deployment URL")?;
        let code = v["code"].as_str().ok_or("the QR has no code")?;
        return Ok((url.to_string(), code.to_string()));
    }
    if url.trim().is_empty() {
        return Err("type the deployment URL too (it's in the Mac's Settings)".into());
    }
    Ok((url.trim().to_string(), text.to_string()))
}

/// Reclama el código: genera la llave y el token del teléfono y queda pendiente de confirmación en
/// la Mac. Devuelve la identidad a guardar (todavía sin permisos).
pub fn pair(url: &str, code: &str, name: &str) -> Result<crate::secure::Identity, String> {
    use age::secrecy::ExposeSecret;
    let key = SigningKey::generate(&mut rand_core::OsRng);
    let token = random_hex(32);
    // Para recibir las claves de los proveedores cifradas (solo este teléfono las abre).
    let enc = age::x25519::Identity::generate();
    let recipient = enc.to_public().to_string();
    let v = call(url, "mutation", "pairing:claim", json!({ "code": code, "publicKey": hex(key.verifying_key().as_bytes()), "token": token, "name": name, "encryptionKey": recipient }))?;
    Ok(crate::secure::Identity {
        url: url.trim_end_matches('/').to_string(),
        daemon_id: v["daemonId"].as_str().ok_or("unexpected answer")?.to_string(),
        device_id: v["deviceId"].as_str().ok_or("unexpected answer")?.to_string(),
        name: name.to_string(),
        token,
        seed: hex(key.as_bytes()),
        age: Some(enc.to_string().expose_secret().to_string()),
    })
}

/// `pending`, `active` o `revoked` (según el backend; la confianza real la tiene la Mac).
pub fn device_status(identity: &crate::secure::Identity) -> Result<String, String> {
    let v = call(&identity.url, "query", "pairing:me", json!({ "token": identity.token }))?;
    Ok(v["status"].as_str().unwrap_or("unknown").to_string())
}

/// Huella corta de la llave del teléfono (la misma que muestra la Mac).
pub fn fingerprint_of(identity: &crate::secure::Identity) -> String {
    let seed: Option<[u8; 32]> = unhex(&identity.seed).and_then(|b| b.try_into().ok());
    let pk = seed.map(|s| hex(SigningKey::from_bytes(&s).verifying_key().as_bytes())).unwrap_or_default();
    pk.chars().take(12).collect::<Vec<_>>().chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join(" ")
}

pub struct Phone {
    pub daemon_id: String,
    pub key: SigningKey,
    engine: CloudEngine,
    transport: ConvexHttpTransport,
    scope: String,
    /// Los incidentes del daemon (`incidents:<daemon>`).
    incidents_scope: String,
    /// Las claves de los proveedores cifradas para este teléfono (`keys:<daemon>`) y su identidad.
    keys_scope: String,
    age: Option<age::x25519::Identity>,
}

pub type Shared = Arc<Mutex<Phone>>;

impl Phone {
    /// Con la identidad guardada (Keychain/Keystore): la llave, el token y a qué daemon pertenece.
    pub fn open(identity: &crate::secure::Identity) -> Result<Self, String> {
        let (replica, outbox) = sqlite::open_in_memory().map_err(|e| e.to_string())?;
        let mut n = 0u64;
        let mut engine = Engine::open(replica, outbox, EngineConfig { backoff_base_ms: 200, backoff_max_ms: 5000, ..Default::default() }, move || {
            n += 1;
            format!("phone-{}-{n}", now_ms())
        })
        .map_err(|e| e.to_string())?;
        let scope = format!("approvals:{}", identity.daemon_id);
        engine.enable_scope(&scope, None).map_err(|e| e.to_string())?;
        let incidents_scope = format!("incidents:{}", identity.daemon_id);
        engine.enable_scope(&incidents_scope, None).map_err(|e| e.to_string())?;
        let keys_scope = format!("keys:{}", identity.daemon_id);
        engine.enable_scope(&keys_scope, None).map_err(|e| e.to_string())?;
        let age = identity.age.as_deref().and_then(|a| a.parse::<age::x25519::Identity>().ok());
        let seed: [u8; 32] = unhex(&identity.seed).and_then(|b| b.try_into().ok()).ok_or("bad key in storage")?;
        Ok(Self { daemon_id: identity.daemon_id.clone(), key: SigningKey::from_bytes(&seed), engine, transport: ConvexHttpTransport::new(&identity.url, &identity.token), scope, incidents_scope, keys_scope, age })
    }

    /// Timbre + sincronizar. Devuelve las aprobaciones (pendientes primero).
    pub fn sync(&mut self) -> Result<Vec<Approval>, String> {
        match self.transport.doorbell(&[self.scope.clone(), self.incidents_scope.clone(), self.keys_scope.clone()], PROTOCOL_VERSION).map_err(|e| e.to_string())? {
            Doorbell::Versions(v) => self.engine.on_doorbell(&v, now_ms()).map_err(|e| e.to_string())?,
            Doorbell::UpgradeRequired => return Err("Condx Cloud needs a newer app".into()),
        }
        block_on(sync_once(&mut self.engine, &self.transport, now_ms, 50)).map_err(|e| e.to_string())?;
        Ok(self.approvals())
    }

    pub fn approvals(&self) -> Vec<Approval> {
        let s = |v: &Values, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        // La fila es del daemon: otro teléfono pudo dejar su decisión. Solo cuenta la de esta llave.
        let mine = hex(self.key.verifying_key().as_bytes());
        let mut list: Vec<(i64, Approval)> = self
            .engine
            .rows(&self.scope)
            .into_iter()
            .map(|r| {
                let own = r.row.values.get("deviceKey").and_then(Value::as_str) == Some(mine.as_str());
                let opt = |k: &str| if own { r.row.values.get(k).and_then(Value::as_str).map(str::to_string) } else { None };
                let created_at = r.row.values.get("createdAt").and_then(Value::as_i64).unwrap_or(0);
                (created_at, Approval {
                job_id: r.row.id.clone(),
                recipe: s(&r.row.values, "recipe"),
                title: s(&r.row.values, "title"),
                env: s(&r.row.values, "env"),
                env_kind: s(&r.row.values, "envKind"),
                status: s(&r.row.values, "status"),
                level: s(&r.row.values, "approval"),
                requested_by: s(&r.row.values, "requestedBy"),
                created_at,
                decision: opt("decision"),
                phone_result: opt("phoneResult"),
                pending: r.pending,
            })
            })
            .collect();
        // Pendientes primero; dentro de cada grupo, lo más reciente arriba.
        list.sort_by_key(|(at, a)| (a.status != "waiting-approval", std::cmp::Reverse(*at), a.job_id.clone()));
        list.into_iter().map(|(_, a)| a).collect()
    }

    /// Decide sobre un trabajo: la decisión firmada entra a la outbox (y sale en el próximo sync).
    pub fn decide(&mut self, job_id: &str, approve: bool, confirm: Option<String>) -> Result<(), String> {
        let decision = if approve { "approve" } else { "reject" };
        let at = now_ms();
        let signature = self.key.sign(decision_message(&self.daemon_id, job_id, decision, at).as_bytes());
        let mut set = Values::new();
        set.insert("decision".into(), json!(decision));
        set.insert("decidedAt".into(), json!(at));
        set.insert("deviceKey".into(), json!(hex(self.key.verifying_key().as_bytes())));
        set.insert("signature".into(), json!(hex(&signature.to_bytes())));
        if let Some(text) = confirm {
            set.insert("confirmText".into(), json!(text));
        }
        self.engine.commit(vec![OpDraft::patch(&self.scope, job_id, set)], at).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Los incidentes: abiertos primero, el más reciente arriba.
    pub fn incidents(&self) -> Vec<Incident> {
        let s = |v: &Values, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let mine = hex(self.key.verifying_key().as_bytes());
        let mut list: Vec<Incident> = self
            .engine
            .rows(&self.incidents_scope)
            .into_iter()
            .map(|r| {
                let v = &r.row.values;
                // El pedido propio que sigue sin respuesta del daemon.
                let own = v.get("deviceKey").and_then(Value::as_str) == Some(mine.as_str());
                let request_id = v.get("requestId").and_then(Value::as_str).map(str::to_string);
                let answered = request_id.is_some() && v.get("resultFor").and_then(Value::as_str) == request_id.as_deref();
                Incident {
                    id: r.row.id.clone(),
                    title: s(v, "title"),
                    source: s(v, "source"),
                    env: s(v, "env"),
                    status: s(v, "status"),
                    count: v.get("count").and_then(Value::as_i64).unwrap_or(1),
                    last_seen: v.get("lastSeen").and_then(Value::as_i64).unwrap_or(0),
                    error: s(v, "error"),
                    log: s(v, "log"),
                    agent: v.get("agent").cloned().unwrap_or(Value::Null),
                    workflows: v.get("workflows").and_then(Value::as_array).into_iter().flatten().filter_map(|w| Some((w["id"].as_str()?.to_string(), w["name"].as_str()?.to_string()))).collect(),
                    result: v.get("result").and_then(Value::as_str).filter(|_| own || answered).map(str::to_string),
                    waiting: own && (r.pending || !answered),
                }
            })
            .collect();
        list.sort_by_key(|i| (i.status != "open", std::cmp::Reverse(i.last_seen)));
        list
    }

    /// Para pruebas: a cuántos destinatarios está cifrada la clave de `id` y el texto cifrado, como los
    /// confirmó el servidor.
    pub fn confirmed_key_recipients(&self, id: &str) -> Option<usize> {
        self.engine.confirmed_row(&self.keys_scope, id).and_then(|r| r.values.get("recipients").and_then(Value::as_u64)).map(|n| n as usize)
    }

    pub fn confirmed_key_sealed(&self, id: &str) -> Option<String> {
        self.engine.confirmed_row(&self.keys_scope, id).and_then(|r| r.values.get("sealed").and_then(Value::as_str).map(str::to_string))
    }

    /// ¿Este teléfono puede recibir claves? (los vinculados antes de A6 no tienen identidad de cifrado).
    pub fn can_receive_keys(&self) -> bool {
        self.age.is_some()
    }

    /// Los proveedores cuyas claves llegaron cifradas para este teléfono, y si se abren aquí.
    pub fn keys(&self) -> Vec<ProviderKey> {
        let s = |v: &Values, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        self.engine
            .rows(&self.keys_scope)
            .into_iter()
            .map(|r| {
                let v = &r.row.values;
                ProviderKey { id: r.row.id.clone(), kind: s(v, "kind"), label: s(v, "label"), model: s(v, "model"), key_hint: s(v, "keyHint"), opens: self.key(&r.row.id).is_ok() }
            })
            .collect()
    }

    /// La clave de un proveedor, descifrada en el teléfono (para correr un agente desde aquí; nunca se
    /// muestra entera).
    pub fn key(&self, id: &str) -> Result<String, String> {
        use std::io::Read;
        let identity = self.age.as_ref().ok_or("this phone has no encryption identity: pair it again")?;
        let row = self.engine.rows(&self.keys_scope).into_iter().find(|r| r.row.id == id).ok_or("no key for that provider")?;
        let sealed = row.row.values.get("sealed").and_then(Value::as_str).ok_or("no key")?;
        let reader = age::armor::ArmoredReader::new(sealed.as_bytes());
        let decryptor = age::Decryptor::new(reader).map_err(|e| e.to_string())?;
        let mut out = String::new();
        decryptor.decrypt(std::iter::once(identity as &dyn age::Identity)).map_err(|_| "it isn't sealed for this phone".to_string())?.read_to_string(&mut out).map_err(|e| e.to_string())?;
        Ok(out)
    }

    /// Pide algo sobre un incidente (diagnosticar, arreglar, ignorar, resolver, desplegar con un
    /// flujo, descartar el arreglo, o un mensaje al agente). Va firmado; el daemon decide.
    pub fn request(&mut self, incident_id: &str, request: &str, arg: &str) -> Result<(), String> {
        let at = now_ms();
        let request_id = random_hex(8);
        let signature = self.key.sign(request_message(&self.daemon_id, incident_id, request, &request_id, at, arg).as_bytes());
        let mut set = Values::new();
        set.insert("request".into(), json!(request));
        set.insert("requestArg".into(), if arg.is_empty() { Value::Null } else { json!(arg) });
        set.insert("requestId".into(), json!(request_id));
        set.insert("requestedAt".into(), json!(at));
        set.insert("deviceKey".into(), json!(hex(self.key.verifying_key().as_bytes())));
        set.insert("signature".into(), json!(hex(&signature.to_bytes())));
        self.engine.commit(vec![OpDraft::patch(&self.incidents_scope, incident_id, set)], at).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// La fila tal como la confirmó el servidor (para comprobar lo que llegó).
    pub fn confirmed(&self, job_id: &str) -> Option<Values> {
        self.engine.confirmed_row(&self.scope, job_id).map(|r| r.values.clone())
    }


    /// Lo que no llegó (rechazos y conflictos del servidor), para mostrarlo.
    pub fn recovery(&self) -> Vec<String> {
        self.engine.recovery_items().iter().map(|r| format!("{:?} {}: {}", r.kind, r.group.ops.first().map(|o| o.row_id.as_str()).unwrap_or("?"), r.error.as_ref().map(|e| e.message.as_str()).unwrap_or_else(|| if r.conflicts.is_some() { "conflict" } else { "" }))).collect()
    }

    pub fn pending_changes(&self) -> usize {
        self.engine.pending_groups().len()
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(v) = future.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_firma_se_verifica_y_no_se_puede_falsificar() {
        let key = SigningKey::generate(&mut rand_core::OsRng);
        let now = 1_000_000;
        let sig = key.sign(decision_message("mac-1", "j1", "approve", now).as_bytes());
        let row = |decision: &str, at: i64, sig: &ed25519_dalek::Signature| -> Values {
            let mut v = Values::new();
            v.insert("decision".into(), json!(decision));
            v.insert("decidedAt".into(), json!(at));
            v.insert("deviceKey".into(), json!(hex(key.verifying_key().as_bytes())));
            v.insert("signature".into(), json!(hex(&sig.to_bytes())));
            v
        };
        let vk = key.verifying_key();
        assert_eq!(verify_decision("mac-1", "j1", &row("approve", now, &sig), &vk, now + 1000, 600_000), Ok("approve".into()));
        // Cambiar la decisión, el trabajo o el daemon invalida la firma.
        assert!(verify_decision("mac-1", "j1", &row("reject", now, &sig), &vk, now, 600_000).is_err());
        assert!(verify_decision("mac-1", "j2", &row("approve", now, &sig), &vk, now, 600_000).is_err());
        assert!(verify_decision("mac-2", "j1", &row("approve", now, &sig), &vk, now, 600_000).is_err());
        // Vieja: se descarta aunque la firma sea buena.
        assert_eq!(verify_decision("mac-1", "j1", &row("approve", now, &sig), &vk, now + 700_000, 600_000), Err("stale decision".into()));
        // Otra llave (otro teléfono): desconocido.
        let other = SigningKey::generate(&mut rand_core::OsRng).verifying_key();
        assert!(verify_decision("mac-1", "j1", &row("approve", now, &sig), &other, now, 600_000).is_err());
    }
}
