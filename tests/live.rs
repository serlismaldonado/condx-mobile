// De punta a punta sin pantalla: teléfono ⇄ Condx Cloud ⇄ daemon (desechables). No corre por defecto:
//
//   CONDX_DAEMON_URL=http://127.0.0.1:4799 CONDX_APP_TOKEN=… cargo test --test live -- --ignored
//
// 1. La Mac abre un código; el teléfono lo reclama con su llave y su token (queda pendiente).
// 2. La Mac ve el mismo nombre y la misma huella, y confirma.
// 3. El teléfono aprueba un trabajo «simple»: el daemon verifica la firma y lo aprueba.
// 4. Un teléfono desvinculado ya no entra.

use nsh_app_condx_mobile::sync::{self, Phone};
use serde_json::{json, Value};

fn daemon(method: &str, path: &str, body: Option<Value>) -> Value {
    let url = std::env::var("CONDX_DAEMON_URL").expect("CONDX_DAEMON_URL");
    let token = std::env::var("CONDX_APP_TOKEN").expect("CONDX_APP_TOKEN");
    let req = ureq::request(method, &format!("{url}{path}")).set("Authorization", &format!("Bearer {token}"));
    let res = match body {
        Some(b) => req.send_json(b),
        None => req.call(),
    };
    match res {
        Ok(r) => r.into_json().unwrap(),
        Err(ureq::Error::Status(code, r)) => panic!("{method} {path} → {code}: {}", r.into_string().unwrap_or_default()),
        Err(e) => panic!("{method} {path}: {e}"),
    }
}

fn wait_for(mut ok: impl FnMut() -> bool, what: &str) {
    for _ in 0..120 {
        if ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    panic!("timed out waiting for {what}");
}

#[test]
#[ignore = "needs a throwaway condxd linked to a throwaway Condx Cloud (CONDX_DAEMON_URL, CONDX_APP_TOKEN)"]
fn vincular_aprobar_y_desvincular() {
    // 1. Código y reclamo.
    let open = daemon("POST", "/api/cloud/pair", Some(json!({})));
    let (url, code) = sync::parse_pairing(open["qr"].as_str().unwrap(), "").unwrap();
    let id = sync::pair(&url, &code, "Test phone").unwrap();
    assert_eq!(sync::device_status(&id).unwrap(), "pending");
    // Pendiente: todavía no lee nada.
    assert!(Phone::open(&id).unwrap().sync().is_err());

    // 2. La Mac ve el mismo teléfono y la misma huella; confirma.
    let status = daemon("GET", "/api/cloud/pair", None);
    assert_eq!(status["status"], "claimed");
    assert_eq!(status["device"]["name"], "Test phone");
    assert_eq!(status["device"]["fingerprint"], sync::fingerprint_of(&id));
    let device_id = status["device"]["deviceId"].as_str().unwrap().to_string();
    daemon("POST", "/api/cloud/pair/confirm", Some(json!({ "deviceId": device_id })));
    assert_eq!(sync::device_status(&id).unwrap(), "active");
    let devices = daemon("GET", "/api/cloud/devices", None);
    assert!(devices.as_array().unwrap().iter().any(|d| d["deviceId"] == device_id.as_str()));

    // 3. Un trabajo que espera aprobación («simple»), y el teléfono lo aprueba.
    let job = daemon("POST", "/api/jobs", Some(json!({ "recipe": "workflow-approval", "env": "ci-proj-local", "args": { "message": "From the phone test" } })));
    let job_id = job["id"].as_str().unwrap().to_string();
    let mut phone = Phone::open(&id).unwrap();
    wait_for(|| phone.sync().unwrap().iter().any(|a| a.job_id == job_id), "the job to reach the phone");
    phone.decide(&job_id, true, None).unwrap();
    wait_for(|| phone.sync().is_ok() && phone.pending_changes() == 0, "the decision to reach the cloud");
    wait_for(|| daemon("GET", &format!("/api/jobs/{job_id}"), None)["status"] != "waiting-approval", "the daemon to apply it");
    let after = daemon("GET", &format!("/api/jobs/{job_id}"), None);
    assert_eq!(after["approvedBy"], "phone:Test phone", "{after}");
    wait_for(|| phone.sync().unwrap().iter().any(|a| a.job_id == job_id && a.phone_result.as_deref() == Some("approved")), "phoneResult on the phone");

    // 4. Desvincular: el teléfono ya no entra.
    daemon("DELETE", &format!("/api/cloud/devices/{device_id}"), None);
    assert_eq!(sync::device_status(&id).unwrap(), "revoked");
    assert!(Phone::open(&id).unwrap().sync().is_err());
}

/// Vincula un teléfono nuevo y lo confirma desde la "Mac" (la API de la app).
fn paired_phone(name: &str) -> (Phone, String) {
    let open = daemon("POST", "/api/cloud/pair", Some(json!({})));
    let (url, code) = sync::parse_pairing(open["qr"].as_str().unwrap(), "").unwrap();
    let id = sync::pair(&url, &code, name).unwrap();
    let status = daemon("GET", "/api/cloud/pair", None);
    let device_id = status["device"]["deviceId"].as_str().unwrap().to_string();
    daemon("POST", "/api/cloud/pair/confirm", Some(json!({ "deviceId": device_id })));
    (Phone::open(&id).unwrap(), device_id)
}

fn job_status(id: &str) -> Value {
    daemon("GET", &format!("/api/jobs/{id}"), None)
}

fn phone_result(phone: &mut Phone, job: &str) -> Option<String> {
    phone.sync().ok()?.into_iter().find(|a| a.job_id == job)?.phone_result
}

#[test]
#[ignore = "needs a throwaway condxd linked to a throwaway Condx Cloud with a fake-prod environment"]
fn escribir_el_nombre_y_produccion_con_touch_id() {
    let (mut phone, device_id) = paired_phone("Levels phone");

    // «typed»: sin el nombre, la Mac no lo aplica; con el nombre, sí.
    let typed = daemon("POST", "/api/jobs", Some(json!({ "recipe": "workflow-approval-strict", "env": "ci-proj-local", "args": { "message": "typed" } })))["id"].as_str().unwrap().to_string();
    wait_for(|| phone.sync().unwrap().iter().any(|a| a.job_id == typed), "the typed job on the phone");
    phone.decide(&typed, true, None).unwrap();
    wait_for(|| phone_result(&mut phone, &typed).is_some(), "the typed result");
    assert!(phone_result(&mut phone, &typed).unwrap().starts_with("rejected: Type"), "{:?}", phone_result(&mut phone, &typed));
    assert_eq!(job_status(&typed)["status"], "waiting-approval");

    let typed2 = daemon("POST", "/api/jobs", Some(json!({ "recipe": "workflow-approval-strict", "env": "ci-proj-local", "args": { "message": "typed 2" } })))["id"].as_str().unwrap().to_string();
    wait_for(|| phone.sync().unwrap().iter().any(|a| a.job_id == typed2), "the second typed job");
    phone.decide(&typed2, true, Some("ci-proj-local".into())).unwrap();
    wait_for(|| phone.sync().is_ok() && job_status(&typed2)["status"] != "waiting-approval", "the typed approval");
    assert_eq!(job_status(&typed2)["approvedBy"], "phone:Levels phone");

    // Producción: el teléfono solo propone. El trabajo sigue esperando hasta Touch ID en la Mac.
    let prod = daemon("POST", "/api/jobs", Some(json!({ "recipe": "workflow-approval", "env": "fake-prod", "args": { "message": "prod" } })))["id"].as_str().unwrap().to_string();
    wait_for(|| phone.sync().unwrap().iter().any(|a| a.job_id == prod), "the prod job on the phone");
    phone.decide(&prod, true, None).unwrap();
    wait_for(|| phone_result(&mut phone, &prod).as_deref() == Some("needs-mac"), "needs-mac");
    std::thread::sleep(std::time::Duration::from_secs(4));
    assert_eq!(job_status(&prod)["status"], "waiting-approval", "production must not be approved from the phone");
    let cloud = daemon("GET", "/api/cloud", None);
    assert!(cloud["status"]["data"]["phoneProposals"].as_array().unwrap().iter().any(|j| j == prod.as_str()));
    // La app confirma con Touch ID (aquí, la llamada que hace la app tras LocalAuthentication).
    daemon("POST", &format!("/api/approvals/{prod}"), Some(json!({ "approve": true, "touchId": true, "confirm": "fake-prod" })));
    assert_ne!(job_status(&prod)["status"], "waiting-approval");

    // Rechazar desde el teléfono siempre se puede (también en producción).
    let prod2 = daemon("POST", "/api/jobs", Some(json!({ "recipe": "workflow-approval", "env": "fake-prod", "args": { "message": "prod 2" } })))["id"].as_str().unwrap().to_string();
    wait_for(|| phone.sync().unwrap().iter().any(|a| a.job_id == prod2), "the second prod job");
    phone.decide(&prod2, false, None).unwrap();
    // El teléfono envía al sincronizar: se sincroniza mientras se espera al daemon.
    wait_for(|| phone.sync().is_ok() && job_status(&prod2)["status"] == "rejected", "the rejection");
    assert!(phone.recovery().is_empty(), "{:?}", phone.recovery());

    daemon("DELETE", &format!("/api/cloud/devices/{device_id}"), None);
}

/// Espera un incidente en el teléfono que cumpla `ok`.
fn wait_incident(phone: &mut Phone, what: &str, ok: impl Fn(&sync::Incident) -> bool) -> sync::Incident {
    for _ in 0..120 {
        let _ = phone.sync();
        if let Some(i) = phone.incidents().into_iter().find(|i| ok(i)) {
            return i;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    panic!("timed out waiting for {what}");
}

#[test]
#[ignore = "needs a throwaway condxd (fake agent, an enabled workflow) linked to a throwaway Condx Cloud and an open incident titled “Unit tests failed…”"]
fn incidentes_desde_el_telefono() {
    let (mut phone, device_id) = paired_phone("Incidents phone");
    let inc = wait_incident(&mut phone, "the incident", |i| i.title.starts_with("Unit tests failed") && i.status == "open");
    let id = inc.id.clone();

    // Diagnosticar: el daemon arranca el agente (pedido firmado) y su respuesta llega al teléfono.
    phone.request(&id, "diagnose", "").unwrap();
    let i = wait_incident(&mut phone, "the diagnosis", |i| i.id == id && i.agent["mode"] == "diagnose" && i.agent["status"] == "done");
    assert!(i.result.as_deref().unwrap_or_default().starts_with("started run-"), "{:?}", i.result);
    assert!(i.agent["answer"].as_str().unwrap_or_default().contains("## Cause"));

    // Preparar un arreglo: queda en su rama, con sus archivos.
    phone.request(&id, "fix", "").unwrap();
    let i = wait_incident(&mut phone, "the fix", |i| i.id == id && i.agent["mode"] == "fix" && i.agent["fix"]["changes"] == true);
    assert_eq!(i.agent["fix"]["files"], serde_json::json!(["lib/dates.js"]));
    let (workflow, _) = i.workflows.first().cloned().expect("an enabled workflow");

    // Desplegar con un flujo: el daemon lo arranca con el commit del arreglo (el flujo pide sus aprobaciones).
    phone.request(&id, "deploy", &workflow).unwrap();
    let i = wait_incident(&mut phone, "the deploy", |i| i.id == id && i.result.as_deref().is_some_and(|r| r.contains("workflow")) && !i.waiting);
    assert!(i.result.as_deref().unwrap().contains("started"), "{:?}", i.result);

    // Un mensaje al agente.
    phone.request(&id, "message", "Explain the change in one line").unwrap();
    let i = wait_incident(&mut phone, "the message", |i| i.id == id && !i.waiting && i.result.is_some());
    assert_eq!(i.result.as_deref(), Some("sent to the agent"));
    // Espera a que el agente termine antes de descartar.
    wait_incident(&mut phone, "the agent to finish", |i| i.id == id && i.agent["status"] == "done");

    // Descartar el arreglo, ignorar y resolver.
    phone.request(&id, "discard", "").unwrap();
    let i = wait_incident(&mut phone, "the discard", |i| i.id == id && i.agent["fix"]["discarded"] == true);
    assert_eq!(i.result.as_deref(), Some("fix discarded"));
    phone.request(&id, "ignore", "").unwrap();
    wait_incident(&mut phone, "ignored", |i| i.id == id && i.status == "ignored");
    phone.request(&id, "resolve", "").unwrap();
    wait_incident(&mut phone, "resolved", |i| i.id == id && i.status == "resolved");

    // Un teléfono desvinculado ya no pide nada.
    daemon("DELETE", &format!("/api/cloud/devices/{device_id}"), None);
    assert!(phone.sync().is_err());
}

#[test]
#[ignore = "needs a throwaway condxd linked to a throwaway Condx Cloud, with a provider whose key is sk-fake-not-real-A6"]
fn claves_cifradas_en_el_telefono() {
    let (mut a, device_a) = paired_phone("Keys phone A");
    let (mut b, _device_b) = paired_phone("Keys phone B");
    let opens = |p: &mut Phone, n: usize| {
        for _ in 0..120 {
            let _ = p.sync();
            let ks = p.keys();
            if ks.iter().any(|k| k.id == "fake") && p.confirmed_key_recipients("fake") == Some(n) {
                return p.key("fake");
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        panic!("timed out waiting for a key sealed for {n} recipients");
    };
    // La Mac y los dos teléfonos.
    assert_eq!(opens(&mut a, 3).unwrap(), "sk-fake-not-real-A6");
    assert_eq!(opens(&mut b, 3).unwrap(), "sk-fake-not-real-A6");
    assert!(a.can_receive_keys());
    // En lo que llega solo está cifrada.
    let sealed = a.confirmed_key_sealed("fake").unwrap();
    assert!(sealed.contains("BEGIN AGE ENCRYPTED FILE") && !sealed.contains("sk-fake-not-real-A6"));

    // Quitar el teléfono A: el daemon vuelve a cifrar para la Mac y B; B la sigue abriendo.
    daemon("DELETE", &format!("/api/cloud/devices/{device_a}"), None);
    assert_eq!(opens(&mut b, 2).unwrap(), "sk-fake-not-real-A6");
    assert!(a.sync().is_err(), "a removed phone doesn't sync anymore");
}
