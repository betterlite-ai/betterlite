use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::{
    CacheError, CacheValue, LocalTtlCache,
    dual::{CacheEvent, Diagnostics, DualLocalCache, EventKind, Ttl},
};
use betterlite_redact::secret::Redactor;
use serde_json::json;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

pub(super) fn run() -> crate::helpers::HelperResult {
    let clock = Arc::new(AtomicU64::new(0.0_f64.to_bits()));
    let source = Arc::clone(&clock);
    let memory = Arc::new(Mutex::new(LocalTtlCache::with_clock(
        Some(2),
        None,
        None,
        1024,
        Arc::new(move || f64::from_bits(source.load(Ordering::Relaxed))),
    )?));
    let events = Arc::new(Mutex::new(Vec::<CacheEvent>::new()));
    let dual = DualLocalCache::new(
        Some(Arc::clone(&memory)),
        Some(4.0_f64),
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics(&events)?,
    );
    dual.set("false", CacheValue::Boolean(false), Ttl::Absent);
    dual.set("zero", CacheValue::Integer(0), Ttl::Explicit(None));
    clock.store(5.0_f64.to_bits(), Ordering::Relaxed);
    let expired = dual.get("false");
    let zero = dual.get("zero");
    clock.store(f64::NAN.to_bits(), Ordering::Relaxed);
    dual.set(
        "failure",
        CacheValue::Text("fixture-secret".to_owned()),
        Ttl::Absent,
    );
    let failure = dual.get("failure");
    let captured = events.lock().map_err(|_poisoned| CacheError::Unavailable)?;
    Ok(json!({"expired": expired, "zero": zero, "failure": failure,
        "set_event": captured.iter().any(|event| event.message.starts_with("set cache:")),
        "get_event": captured.iter().any(|event| event.kind == EventKind::Error)}))
}

fn diagnostics(
    events: &Arc<Mutex<Vec<CacheEvent>>>,
) -> Result<Diagnostics, Box<crate::helpers::HelperError>> {
    let sink = Arc::clone(events);
    let redactor = Redactor::new(16).map_err(|error| {
        Box::new(crate::helpers::HelperError {
            class: "RedactionError".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: error.to_string(),
        })
    })?;
    Ok(Diagnostics {
        redactor: Arc::new(redactor),
        emit: Arc::new(move |event| {
            if let Ok(mut captured) = sink.lock() {
                captured.push(event);
            }
        }),
    })
}
