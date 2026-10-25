// El teléfono contra Condx Cloud (backend desechable): ve las aprobaciones, decide con firma, la
// decisión llega al servidor y el daemon la podría verificar. No corre por defecto.

use nsh_app_condx_mobile::sync::{now_ms, verify_decision, Phone};

#[test]
#[ignore = "needs CONDX_CLOUD_URL, CONDX_DAEMON_ID and CONDX_DAEMON_TOKEN"]
fn el_telefono_decide_y_la_firma_llega() {
    let get = |k: &str| std::env::var(k).expect(k);
    let (url, daemon, token) = (get("CONDX_CLOUD_URL"), get("CONDX_DAEMON_ID"), get("CONDX_DAEMON_TOKEN"));
    let mut phone = Phone::open(&url, &token, &daemon).unwrap();
    let list = phone.sync().unwrap();
    let job = list.iter().find(|a| a.status == "waiting-approval" && a.decision.is_none()).expect("a waiting approval").job_id.clone();
    phone.decide(&job, true).unwrap();
    assert_eq!(phone.approvals().iter().find(|a| a.job_id == job).unwrap().decision.as_deref(), Some("approve"));
    for _ in 0..10 {
        phone.sync().unwrap();
        if phone.pending_changes() == 0 {
            break;
        }
    }
    assert_eq!(phone.pending_changes(), 0);
    let row = phone.confirmed(&job).expect("row confirmed by the server");
    let key = phone.key.verifying_key();
    assert_eq!(verify_decision(&daemon, &job, &row, &key, now_ms(), 600_000), Ok("approve".into()));
}
