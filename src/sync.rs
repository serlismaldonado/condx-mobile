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
    pub status: String,
    pub level: String,
    pub requested_by: String,
    pub decision: Option<String>,
    pub pending: bool,
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

pub struct Phone {
    pub daemon_id: String,
    pub key: SigningKey,
    engine: CloudEngine,
    transport: ConvexHttpTransport,
    scope: String,
}

pub type Shared = Arc<Mutex<Phone>>;

impl Phone {
    pub fn open(url: &str, token: &str, daemon_id: &str) -> Result<Self, String> {
        let (replica, outbox) = sqlite::open_in_memory().map_err(|e| e.to_string())?;
        let mut n = 0u64;
        let mut engine = Engine::open(replica, outbox, EngineConfig { backoff_base_ms: 200, backoff_max_ms: 5000, ..Default::default() }, move || {
            n += 1;
            format!("phone-{}-{n}", now_ms())
        })
        .map_err(|e| e.to_string())?;
        let scope = format!("approvals:{daemon_id}");
        engine.enable_scope(&scope, None).map_err(|e| e.to_string())?;
        Ok(Self { daemon_id: daemon_id.into(), key: SigningKey::generate(&mut rand_core::OsRng), engine, transport: ConvexHttpTransport::new(url, token), scope })
    }

    /// Timbre + sincronizar. Devuelve las aprobaciones (pendientes primero).
    pub fn sync(&mut self) -> Result<Vec<Approval>, String> {
        match self.transport.doorbell(std::slice::from_ref(&self.scope), PROTOCOL_VERSION).map_err(|e| e.to_string())? {
            Doorbell::Versions(v) => self.engine.on_doorbell(&v, now_ms()).map_err(|e| e.to_string())?,
            Doorbell::UpgradeRequired => return Err("Condx Cloud needs a newer app".into()),
        }
        block_on(sync_once(&mut self.engine, &self.transport, now_ms, 50)).map_err(|e| e.to_string())?;
        Ok(self.approvals())
    }

    pub fn approvals(&self) -> Vec<Approval> {
        let s = |v: &Values, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let mut list: Vec<Approval> = self
            .engine
            .rows(&self.scope)
            .into_iter()
            .map(|r| Approval {
                job_id: r.row.id.clone(),
                recipe: s(&r.row.values, "recipe"),
                title: s(&r.row.values, "title"),
                env: s(&r.row.values, "env"),
                status: s(&r.row.values, "status"),
                level: s(&r.row.values, "approval"),
                requested_by: s(&r.row.values, "requestedBy"),
                decision: r.row.values.get("decision").and_then(Value::as_str).map(str::to_string),
                pending: r.pending,
            })
            .collect();
        list.sort_by_key(|a| (a.status != "waiting-approval", a.job_id.clone()));
        list
    }

    /// Decide sobre un trabajo: la decisión firmada entra a la outbox (y sale en el próximo sync).
    pub fn decide(&mut self, job_id: &str, approve: bool) -> Result<(), String> {
        let decision = if approve { "approve" } else { "reject" };
        let at = now_ms();
        let signature = self.key.sign(decision_message(&self.daemon_id, job_id, decision, at).as_bytes());
        let mut set = Values::new();
        set.insert("decision".into(), json!(decision));
        set.insert("decidedAt".into(), json!(at));
        set.insert("deviceKey".into(), json!(hex(self.key.verifying_key().as_bytes())));
        set.insert("signature".into(), json!(hex(&signature.to_bytes())));
        self.engine.commit(vec![OpDraft::patch(&self.scope, job_id, set)], at).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// La fila tal como la confirmó el servidor (para comprobar lo que llegó).
    pub fn confirmed(&self, job_id: &str) -> Option<Values> {
        self.engine.confirmed_row(&self.scope, job_id).map(|r| r.values.clone())
    }

    pub fn fingerprint(&self) -> String {
        hex(&self.key.verifying_key().as_bytes()[..6])
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
