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

/// Un entorno de la Mac (`env:`): si su Convex responde.
#[derive(Debug, Clone, PartialEq)]
pub struct Env {
    pub name: String,
    pub kind: String,
    pub project: String,
    pub deployment: String,
    /// `None`: todavía no se revisó.
    pub reachable: Option<bool>,
}

/// Un trabajo reciente de la Mac (`jobs:`).
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: String,
    pub title: String,
    pub recipe: String,
    pub env: String,
    pub env_kind: String,
    pub status: String,
    pub requested_by: String,
    pub created_at: i64,
    /// Cuándo empezó a correr (después de la aprobación, si pedía una).
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub error: Option<String>,
    pub commit: Option<String>,
    /// La cola del registro (solo de los que corren o fallaron).
    pub log: Option<String>,
}

/// Una receta que se puede pedir desde el teléfono: en qué tipos de entorno y sus argumentos
/// (nombre, tipo, valor por defecto).
#[derive(Debug, Clone, PartialEq)]
pub struct RecipeInfo {
    pub name: String,
    pub title: String,
    pub kinds: Vec<String>,
    pub args: Vec<(String, String, Option<String>)>,
}

/// Un flujo encendido de la Mac.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowInfo {
    pub id: String,
    pub name: String,
    pub project: String,
}

/// Lo último que este teléfono pidió correr: si espera y qué contestó la Mac.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunState {
    pub waiting: bool,
    pub result: Option<String>,
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

/// Un hilo (una corrida de un flujo con sus agentes, o un chat) como lo ve el teléfono.
#[derive(Debug, Clone, PartialEq)]
pub struct Thread {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    pub updated_at: i64,
    /// `[{kind, author, to, at, status, job, text}]`: los últimos mensajes, recortados.
    pub items: Vec<Value>,
    /// `[{node, name}]`: a quién se le puede escribir con `@node`.
    pub participants: Vec<Value>,
    pub waiting_answer: bool,
    /// En qué proyecto y entorno trabaja (el entorno, si es de un incidente o un flujo).
    pub project: Option<String>,
    pub env: Option<String>,
    pub result: Option<String>,
    pub waiting: bool,
    /// Lo que este teléfono mandó y todavía no contestó el daemon (se muestra al instante).
    pub sending: Option<String>,
    /// Lo mandado ya salió del teléfono (está en la nube) y nadie lo tomó todavía: ✓.
    pub delivered: bool,
    /// El último mensaje de este teléfono ya lo tomó un agente (la Mac o el sitio en la nube): ✓✓.
    pub read: bool,
}

/// Un adjunto subido: lo que el mensaje dice de él (el daemon lo baja con `id`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub mime: String,
    /// `image`, `audio` o `file`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
}

/// Dónde responden las rutas HTTP del deployment (como `site_url` del daemon).
pub fn site_url(url: &str) -> String {
    let url = url.trim_end_matches('/');
    if let Some(base) = url.strip_suffix(".convex.cloud") {
        format!("{base}.convex.site")
    } else if let Some((host, port)) = url.strip_prefix("http://").and_then(|r| r.rsplit_once(':')).filter(|(h, _)| *h == "127.0.0.1" || *h == "localhost" || *h == "10.0.2.2") {
        format!("http://{host}:{}", port.parse::<u16>().map(|p| p + 1).unwrap_or(3211))
    } else {
        url.to_string()
    }
}

/// Lo que firma el teléfono para escribir en un hilo (mismo texto en el daemon; otro prefijo que el
/// de los incidentes).
pub fn thread_message(daemon_id: &str, thread_id: &str, request_id: &str, at: i64, arg: &str) -> String {
    format!("condx-thread:v1|{daemon_id}|{thread_id}|message|{request_id}|{at}|{arg}")
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
    threads_scope: String,
    /// Los entornos y los trabajos recientes de la Mac (solo lectura).
    env_scope: String,
    jobs_scope: String,
    age: Option<age::x25519::Identity>,
    /// El deployment y el token del teléfono: para subir adjuntos a `/files`.
    url: String,
    token: String,
    /// El timbre en vivo (se abre al sincronizar con la app a la vista; se cierra de fondo) y cuándo
    /// sonó por HTTP por última vez: aunque el socket diga que anda, uno cada minuto por las dudas.
    bell: Option<crate::bell::Bell>,
    rung_at: i64,
}

/// Cada cuánto suena por HTTP aunque el timbre en vivo ande (un socket caído que no se notó).
const SAFETY_RING_MS: i64 = 60_000;

pub type Shared = Arc<Mutex<Phone>>;

impl Phone {
    /// Con la identidad guardada (Keychain/Keystore): la llave, el token y a qué daemon pertenece.
    pub fn open(identity: &crate::secure::Identity) -> Result<Self, String> {
        // En disco: abre con lo último que se vio (la lista sale al instante) y sincroniza solo lo que
        // cambió; lo pendiente de mandar sobrevive a un cierre. Si no se puede, en memoria.
        let file = crate::secure::data_dir().map(|d| d.join(format!("cloud-{}-{}.sqlite3", identity.daemon_id, identity.device_id)));
        let (replica, outbox) = match file.as_deref().map(sqlite::open) {
            Some(Ok(pair)) => pair,
            _ => sqlite::open_in_memory().map_err(|e| e.to_string())?,
        };
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
        let threads_scope = format!("threads:{}", identity.daemon_id);
        engine.enable_scope(&threads_scope, None).map_err(|e| e.to_string())?;
        let env_scope = format!("env:{}", identity.daemon_id);
        engine.enable_scope(&env_scope, None).map_err(|e| e.to_string())?;
        let jobs_scope = format!("jobs:{}", identity.daemon_id);
        engine.enable_scope(&jobs_scope, None).map_err(|e| e.to_string())?;
        let age = identity.age.as_deref().and_then(|a| a.parse::<age::x25519::Identity>().ok());
        let seed: [u8; 32] = unhex(&identity.seed).and_then(|b| b.try_into().ok()).ok_or("bad key in storage")?;
        Ok(Self { daemon_id: identity.daemon_id.clone(), key: SigningKey::from_bytes(&seed), engine, transport: ConvexHttpTransport::new(&identity.url, &identity.token), scope, incidents_scope, keys_scope, threads_scope, env_scope, jobs_scope, age, url: identity.url.clone(), token: identity.token.clone(), bell: None, rung_at: 0 })
    }

    /// Timbre + sincronizar. Devuelve las aprobaciones (pendientes primero).
    pub fn sync(&mut self) -> Result<Vec<Approval>, String> {
        let scopes = [self.scope.clone(), self.incidents_scope.clone(), self.keys_scope.clone(), self.threads_scope.clone(), self.env_scope.clone(), self.jobs_scope.clone()];
        let bell = self.bell.get_or_insert_with(|| crate::bell::Bell::start(&self.url, &self.token, &scopes, PROTOCOL_VERSION));
        let now = now_ms();
        // Con el timbre en vivo, solo lo que sonó; sin él (o de vez en cuando), el de HTTP.
        let rung = if bell.is_live() && now - self.rung_at < SAFETY_RING_MS {
            match bell.take() {
                Some(v) if v["status"] == "upgradeRequired" => Some(Doorbell::UpgradeRequired),
                Some(v) => Some(Doorbell::Versions(serde_json::from_value(v["versions"].clone()).map_err(|e| format!("unexpected doorbell: {e}"))?)),
                None => None,
            }
        } else {
            bell.take();
            self.rung_at = now;
            Some(self.transport.doorbell(&scopes, PROTOCOL_VERSION).map_err(|e| e.to_string())?)
        };
        match rung {
            Some(Doorbell::Versions(v)) => self.engine.on_doorbell(&v, now).map_err(|e| e.to_string())?,
            Some(Doorbell::UpgradeRequired) => return Err("Condx Cloud needs a newer app".into()),
            None => {}
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

    pub fn threads(&self) -> Vec<Thread> {
        let s = |v: &Values, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let json_list = |v: &Values, k: &str| v.get(k).and_then(Value::as_str).and_then(|t| serde_json::from_str::<Vec<Value>>(t).ok()).unwrap_or_default();
        let mine = hex(self.key.verifying_key().as_bytes());
        let mut list: Vec<Thread> = self
            .engine
            .rows(&self.threads_scope)
            .into_iter()
            // `_mac` y `_new` no son hilos: la llave de la Mac y donde se piden hilos nuevos.
            // Tampoco el resumen del sitio en la nube (`site-overview`).
            .filter(|r| !r.row.id.starts_with('_') && r.row.id != "site-overview")
            .map(|r| {
                let v = &r.row.values;
                let own = v.get("deviceKey").and_then(Value::as_str) == Some(mine.as_str());
                let request_id = v.get("requestId").and_then(Value::as_str).map(str::to_string);
                // Lo contestó la Mac (`resultFor`) o, con la Mac apagada, el sitio en la nube (`siteFor`).
                let by_site = request_id.is_some() && v.get("siteFor").and_then(Value::as_str) == request_id.as_deref();
                let answered = by_site || (request_id.is_some() && v.get("resultFor").and_then(Value::as_str) == request_id.as_deref());
                // Lo que contestó el sitio va en el hilo junto con lo de la Mac, en orden.
                let mut items = json_list(v, "items");
                let site_items = json_list(v, "siteItems");
                let site_at = site_items.iter().filter_map(|i| i["at"].as_i64()).max().unwrap_or(0);
                items.extend(site_items);
                items.sort_by_key(|i| i["at"].as_i64().unwrap_or(0));
                Thread {
                    id: r.row.id.clone(),
                    title: s(v, "title"),
                    kind: s(v, "kind"),
                    status: s(v, "status"),
                    updated_at: v.get("updatedAt").and_then(Value::as_i64).unwrap_or(0).max(site_at),
                    items,
                    participants: json_list(v, "participants"),
                    waiting_answer: v.get("waitingAnswer").and_then(Value::as_bool).unwrap_or(false),
                    project: v.get("project").and_then(Value::as_str).map(str::to_string),
                    env: v.get("env").and_then(Value::as_str).map(str::to_string),
                    result: if by_site { Some("answered by the cloud site".to_string()).filter(|_| own) } else { v.get("result").and_then(Value::as_str).filter(|_| own || answered).map(str::to_string) },
                    waiting: own && (r.pending || !answered),
                    sending: v.get("requestArg").and_then(Value::as_str).filter(|_| own && (r.pending || !answered)).map(str::to_string),
                    delivered: own && !r.pending && !answered,
                    read: own && answered,
                }
            })
            .collect();
        list.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        list
    }

    /// Escribe en un hilo: sale firmado en el próximo sync; el daemon verifica y decide a quién va.
    pub fn thread_send(&mut self, thread_id: &str, text: &str) -> Result<(), String> {
        self.thread_send_with(thread_id, text, &[])
    }

    /// De fondo: se cierra el timbre en vivo (sin socket abierto); al volver se abre de nuevo.
    pub fn pause(&mut self) {
        self.bell = None;
    }

    /// Si el timbre en vivo sonó (hay algo nuevo en la nube) y si anda.
    pub fn bell(&self) -> (bool, bool) {
        self.bell.as_ref().map(|b| (b.rang(), b.is_live())).unwrap_or((false, false))
    }

    /// La fila donde se pide un hilo nuevo, como un hilo vacío («New thread»): lo que se mandó desde
    /// aquí y qué contestó el daemon («thread <id>» con el hilo creado).
    pub fn new_thread(&self) -> Thread {
        let mine = hex(self.key.verifying_key().as_bytes());
        let row = self.engine.rows(&self.threads_scope).into_iter().find(|r| r.row.id == "_new");
        let (mut waiting, mut result, mut sending, mut delivered) = (false, None, None, false);
        if let Some(r) = &row {
            let v = &r.row.values;
            let own = v.get("deviceKey").and_then(Value::as_str) == Some(mine.as_str());
            let request_id = v.get("requestId").and_then(Value::as_str);
            // Un hilo nuevo con la Mac apagada lo crea el sitio en la nube (`siteFor`, `siteResult`).
            let by_site = request_id.is_some() && v.get("siteFor").and_then(Value::as_str) == request_id;
            let answered = by_site || (request_id.is_some() && v.get("resultFor").and_then(Value::as_str) == request_id);
            waiting = own && (r.pending || !answered);
            result = v.get(if by_site { "siteResult" } else { "result" }).and_then(Value::as_str).filter(|_| own && answered).map(str::to_string);
            sending = v.get("requestArg").and_then(Value::as_str).filter(|_| waiting).map(str::to_string);
            delivered = own && !r.pending && !answered;
        }
        Thread { id: "_new".into(), title: crate::i18n::t("New thread"), kind: "chat".into(), status: String::new(), updated_at: 0, items: vec![], participants: vec![], waiting_answer: false, project: None, env: None, result, waiting, sending, delivered, read: false }
    }

    /// El catálogo de agentes que publica el daemon (fila `_agents`): (id, nombre, marca del proveedor).
    pub fn catalog(&self) -> Vec<(String, String, String)> {
        let Some(r) = self.engine.rows(&self.threads_scope).into_iter().find(|r| r.row.id == "_agents") else { return vec![] };
        let list: Vec<Value> = r.row.values.get("participants").and_then(Value::as_str).and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
        list.iter().filter_map(|a| Some((a["id"].as_str()?.to_string(), a["name"].as_str()?.to_string(), a["logo"].as_str().unwrap_or("").to_string()))).collect()
    }

    /// La última versión de la app del teléfono que publicó el daemon (fila `_apk`): versión, su
    /// número (versionCode) y el enlace de descarga.
    pub fn latest_apk(&self) -> Option<(String, i64, String)> {
        let r = self.engine.rows(&self.threads_scope).into_iter().find(|r| r.row.id == "_apk")?;
        let v = &r.row.values;
        Some((v.get("version")?.as_str()?.to_string(), v.get("versionCode")?.as_i64()?, v.get("url")?.as_str()?.to_string()))
    }

    /// Los entornos, por proyecto y nombre.
    pub fn envs(&self) -> Vec<Env> {
        let s = |v: &Values, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let mut list: Vec<Env> = self
            .engine
            .rows(&self.env_scope)
            .into_iter()
            .map(|r| {
                let v = &r.row.values;
                Env { name: s(v, "name"), kind: s(v, "kind"), project: s(v, "project"), deployment: s(v, "deployment"), reachable: v.get("reachable").and_then(Value::as_bool) }
            })
            .collect();
        list.sort_by(|a, b| (a.project.clone(), a.name.clone()).cmp(&(b.project.clone(), b.name.clone())));
        list
    }

    /// Los trabajos recientes, el más nuevo primero.
    pub fn jobs(&self) -> Vec<Job> {
        let s = |v: &Values, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let o = |v: &Values, k: &str| v.get(k).and_then(Value::as_str).filter(|x| !x.is_empty()).map(str::to_string);
        let mut list: Vec<Job> = self
            .engine
            .rows(&self.jobs_scope)
            .into_iter()
            .map(|r| {
                let v = &r.row.values;
                Job {
                    id: r.row.id.clone(),
                    title: s(v, "title"),
                    recipe: s(v, "recipe"),
                    env: s(v, "env"),
                    env_kind: s(v, "envKind"),
                    status: s(v, "status"),
                    requested_by: s(v, "requestedBy"),
                    created_at: v.get("createdAt").and_then(Value::as_i64).unwrap_or(0),
                    started_at: v.get("startedAt").and_then(Value::as_i64),
                    ended_at: v.get("endedAt").and_then(Value::as_i64),
                    error: o(v, "error"),
                    commit: o(v, "commit"),
                    log: o(v, "log"),
                }
            })
            .collect();
        list.sort_by_key(|j| std::cmp::Reverse(j.created_at));
        list
    }

    /// Lo que se puede pedir correr (la fila `_catalog` que publica la Mac).
    pub fn run_catalog(&self) -> (Vec<RecipeInfo>, Vec<FlowInfo>) {
        let Some(v) = self.engine.rows(&self.threads_scope).into_iter().find(|r| r.row.id == "_catalog").and_then(|r| r.row.values.get("catalog").and_then(Value::as_str).and_then(|t| serde_json::from_str::<Value>(t).ok())) else {
            return (vec![], vec![]);
        };
        let st = |x: &Value| x.as_str().unwrap_or_default().to_string();
        let recipes = v["recipes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| RecipeInfo {
                name: st(&r["name"]),
                title: st(&r["title"]),
                kinds: r["kinds"].as_array().into_iter().flatten().map(st).collect(),
                args: r["args"].as_array().into_iter().flatten().map(|a| (st(&a["name"]), st(&a["type"]), a["default"].as_str().map(str::to_string))).collect(),
            })
            .collect();
        let flows = v["flows"].as_array().into_iter().flatten().map(|f| FlowInfo { id: st(&f["id"]), name: st(&f["name"]), project: st(&f["project"]) }).collect();
        (recipes, flows)
    }

    /// Pide correr una receta (`{recipe, env, args}`) o un flujo (`{flow}`): firmado en `_run`; la Mac
    /// lo verifica y aplica las mismas aprobaciones que si se pidiera allá.
    pub fn run(&mut self, ask: &Value) -> Result<(), String> {
        let at = now_ms();
        let request_id = random_hex(8);
        let arg = ask.to_string();
        let signature = self.key.sign(request_message(&self.daemon_id, "_run", "run", &request_id, at, &arg).as_bytes());
        let mut set = Values::new();
        set.insert("request".into(), json!("run"));
        set.insert("requestArg".into(), json!(arg));
        set.insert("requestId".into(), json!(request_id));
        set.insert("requestedAt".into(), json!(at));
        set.insert("deviceKey".into(), json!(hex(self.key.verifying_key().as_bytes())));
        set.insert("signature".into(), json!(hex(&signature.to_bytes())));
        self.engine.commit(vec![OpDraft::patch(&self.threads_scope, "_run", set)], at).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Cómo va lo último que este teléfono pidió correr.
    pub fn run_state(&self) -> RunState {
        let mine = hex(self.key.verifying_key().as_bytes());
        let Some(r) = self.engine.rows(&self.threads_scope).into_iter().find(|r| r.row.id == "_run") else { return RunState::default() };
        let v = &r.row.values;
        if v.get("deviceKey").and_then(Value::as_str) != Some(mine.as_str()) {
            return RunState::default();
        }
        let request_id = v.get("requestId").and_then(Value::as_str);
        // Lo contesta la Mac (`resultFor`) o, con la Mac apagada, el sitio en la nube (`siteFor`).
        let by_mac = request_id.is_some() && v.get("resultFor").and_then(Value::as_str) == request_id;
        let by_site = request_id.is_some() && v.get("siteFor").and_then(Value::as_str) == request_id;
        let result = if by_mac { v.get("result") } else if by_site { v.get("siteResult") } else { None };
        RunState { waiting: r.pending || !(by_mac || by_site), result: result.and_then(Value::as_str).map(str::to_string) }
    }

    /// El resumen de cada proyecto (la fila `_overview` que publica la Mac): git, deployer, servicios,
    /// lo que pide atención y los detalles de cada entorno. `Null` si la Mac todavía no lo publicó.
    pub fn overview(&self) -> Value {
        self.overview_of("_overview")
    }

    /// El resumen del sitio en la nube (`site-overview`): sus servicios locales, con la Mac apagada.
    pub fn site_overview(&self) -> Value {
        self.overview_of("site-overview")
    }

    fn overview_of(&self, row: &str) -> Value {
        self.engine
            .rows(&self.threads_scope)
            .into_iter()
            .find(|r| r.row.id == row)
            .and_then(|r| r.row.values.get("overview").and_then(Value::as_str).and_then(|t| serde_json::from_str::<Value>(t).ok()))
            .unwrap_or(Value::Null)
    }

    /// El modelo de cada agente y los de su proveedor (fila `_agents`): (id, modelo, modelos).
    pub fn agent_models(&self) -> Vec<(String, Option<String>, Vec<String>)> {
        let Some(r) = self.engine.rows(&self.threads_scope).into_iter().find(|r| r.row.id == "_agents") else { return vec![] };
        let list: Vec<Value> = r.row.values.get("participants").and_then(Value::as_str).and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
        list.iter()
            .filter_map(|a| Some((a["id"].as_str()?.to_string(), a["model"].as_str().map(str::to_string), a["models"].as_array().map(|l| l.iter().filter_map(|m| m.as_str().map(str::to_string)).collect()).unwrap_or_default())))
            .collect()
    }

    /// El modelo predeterminado y los del proveedor predeterminado (para un hilo nuevo o sin mención).
    pub fn default_models(&self) -> (String, Vec<String>) {
        let v = self
            .engine
            .rows(&self.threads_scope)
            .into_iter()
            .find(|r| r.row.id == "_settings")
            .and_then(|r| r.row.values.get("settings").and_then(Value::as_str).and_then(|t| serde_json::from_str::<Value>(t).ok()))
            .unwrap_or(Value::Null);
        let models: Vec<String> = v["defaultModels"].as_array().map(|l| l.iter().filter_map(|m| m.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let model = v["defaultModel"].as_str().map(str::to_string).filter(|m| !m.is_empty()).or_else(|| models.first().cloned()).unwrap_or_default();
        (model, models)
    }

    /// En qué idioma contestan los agentes (la configuración que publica la Mac en `_settings`).
    pub fn agent_language(&self) -> String {
        self.engine
            .rows(&self.threads_scope)
            .into_iter()
            .find(|r| r.row.id == "_settings")
            .and_then(|r| r.row.values.get("settings").and_then(Value::as_str).and_then(|t| serde_json::from_str::<Value>(t).ok()))
            .and_then(|v| v["language"].as_str().map(str::to_string))
            .unwrap_or_default()
    }

    /// El último latido de la Mac (`aliveAt` en la fila `_mac`, cada 5 min mientras está encendida).
    pub fn mac_alive_at(&self) -> Option<i64> {
        self.engine.rows(&self.threads_scope).into_iter().find(|r| r.row.id == "_mac")?.row.values.get("aliveAt")?.as_i64()
    }

    /// La llave age de la Mac (la publica el daemon en la fila `_mac` de los hilos).
    pub fn mac_recipient(&self) -> Option<age::x25519::Recipient> {
        self.engine.rows(&self.threads_scope).into_iter().find(|r| r.row.id == "_mac")?.row.values.get("recipient")?.as_str()?.parse().ok()
    }

    /// Cifra el archivo para la Mac (la nube no puede leerlo) y lo sube a `/files`; devuelve su id.
    pub fn upload(&self, bytes: &[u8]) -> Result<String, String> {
        let mac = self.mac_recipient().ok_or("the Mac hasn't published its key yet: update Condx on the Mac")?;
        let encryptor = age::Encryptor::with_recipients(std::iter::once(&mac as &dyn age::Recipient)).map_err(|e| e.to_string())?;
        let mut sealed = vec![];
        let mut w = encryptor.wrap_output(&mut sealed).map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut w, bytes).map_err(|e| e.to_string())?;
        w.finish().map_err(|e| e.to_string())?;
        let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(120)).build();
        let url = format!("{}/files", site_url(&self.url));
        match agent.post(&url).set("Authorization", &format!("Bearer {}", self.token)).set("Content-Type", "application/octet-stream").send_bytes(&sealed) {
            Ok(r) => r.into_json::<Value>().map_err(|e| e.to_string())?["id"].as_str().map(str::to_string).ok_or_else(|| "no id from Condx Cloud".into()),
            Err(ureq::Error::Status(413, _)) => Err("the file is too big (10 MB at most)".into()),
            Err(ureq::Error::Status(code, r)) => Err(format!("Condx Cloud answered {code}: {}", r.into_string().unwrap_or_default().chars().take(120).collect::<String>())),
            Err(e) => Err(format!("couldn't upload: {e}")),
        }
    }

    /// Un mensaje con adjuntos ya subidos (`upload`): van firmados con el texto («texto␟archivos»).
    pub fn thread_send_with(&mut self, thread_id: &str, text: &str, files: &[Attachment]) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() && files.is_empty() {
            return Err("write something".into());
        }
        let at = now_ms();
        let request_id = random_hex(8);
        let files_json = (!files.is_empty()).then(|| serde_json::to_string(files).unwrap_or_default());
        let signed = match &files_json {
            Some(f) => format!("{text}\u{1f}{f}"),
            None => text.to_string(),
        };
        let signature = self.key.sign(thread_message(&self.daemon_id, thread_id, &request_id, at, &signed).as_bytes());
        let mut set = Values::new();
        set.insert("request".into(), json!("message"));
        set.insert("requestArg".into(), json!(text));
        set.insert("requestFiles".into(), files_json.map(Value::String).unwrap_or(Value::Null));
        set.insert("requestId".into(), json!(request_id));
        set.insert("requestedAt".into(), json!(at));
        set.insert("deviceKey".into(), json!(hex(self.key.verifying_key().as_bytes())));
        set.insert("signature".into(), json!(hex(&signature.to_bytes())));
        self.engine.commit(vec![OpDraft::patch(&self.threads_scope, thread_id, set)], at).map_err(|e| e.to_string())?;
        Ok(())
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
            // Las deploy keys de cloud dev y las variables de las apps van solo al sitio en la nube.
            .filter(|r| !matches!(r.row.values.get("kind").and_then(Value::as_str), Some("convex-deploy" | "app-env")))
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

    /// Descarta los cambios que la nube rechazó (quedan en recuperación) y vuelve a sincronizar.
    pub fn discard_rejected(&mut self) -> usize {
        let ids: Vec<String> = self.engine.recovery_items().iter().map(|r| r.group_id.clone()).collect();
        for id in &ids {
            let _ = self.engine.discard_recovery(id);
        }
        ids.len()
    }

    /// El estado del motor de sincronización (para ver por qué algo no sale).
    pub fn sync_debug(&self) -> String {
        format!("{:?}", self.engine.sync_status())
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
