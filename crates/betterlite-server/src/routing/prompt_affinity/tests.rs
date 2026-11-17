#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_config::sections::prompt_affinity as section;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::routing::prompt_affinity::settings;

use super::{DualCachePins, PinCache};

fn configured(prompt_caching: bool, maximum_base64_length: Option<&str>) -> section::Settings {
    section::Settings {
        prompt_caching,
        maximum_base64_length: maximum_base64_length.map(str::to_owned),
    }
}

fn pin(model_id: &str) -> Json {
    Json::Object(IndexMap::from([(
        "model_id".to_owned(),
        Json::Text(model_id.to_owned()),
    )]))
}

fn clock_at(handle: Arc<Mutex<f64>>) -> Arc<dyn Fn() -> f64 + Send + Sync> {
    Arc::new(move || *handle.lock().unwrap())
}

fn cache_at(now: Arc<Mutex<f64>>) -> Arc<DualCache<LocalMemoryOps>> {
    let memory = LocalMemoryOps::new(1024, clock_at(now)).unwrap();
    Arc::new(DualCache::new(
        Arc::new(memory),
        None,
        DualCacheSettings {
            default_in_memory_ttl: None,
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: 1000,
            redis_delete_chunk_size: 1000,
        },
    ))
}

fn message_uri(payload: &str) -> Json {
    let source = format!(
        r#"{{"role":"user","content":[{{"type":"image_url","image_url":{{"url":"data:image/png;base64,{payload}"}}}}]}}"#
    );
    serde_json::from_str(&source).unwrap()
}

#[test]
fn the_check_is_off_unless_the_config_names_it() {
    assert!(!settings(&configured(false, None)).unwrap().enabled);
    assert!(settings(&configured(true, None)).unwrap().enabled);
}

#[test]
fn the_configured_base64_limit_decides_which_payloads_shrink() {
    let payload = "A".repeat(12);
    let message = message_uri(&payload);
    let default = settings(&configured(true, None)).unwrap().truncator;
    let short = settings(&configured(true, Some("8"))).unwrap().truncator;
    assert_eq!(default.truncate_messages(&message).unwrap(), message);
    assert_ne!(short.truncate_messages(&message).unwrap(), message);
}

#[test]
fn a_malformed_base64_limit_is_an_error() {
    assert!(settings(&configured(true, Some("many"))).is_err());
}

#[tokio::test]
async fn a_pin_is_readable_until_its_ttl_expires() {
    let now = Arc::new(Mutex::new(0.0_f64));
    let pins: Arc<dyn PinCache> = Arc::new(DualCachePins {
        cache: cache_at(Arc::clone(&now)),
    });
    let key = "deployment:abc:prompt_caching".to_owned();
    pins.store(&key, &pin("dep-a")).await;
    let keys = vec![key];
    assert_eq!(pins.read(&keys).await, [Some(pin("dep-a"))]);
    *now.lock().unwrap() = 299.0_f64;
    assert_eq!(pins.read(&keys).await, [Some(pin("dep-a"))]);
    *now.lock().unwrap() = 301.0_f64;
    assert_eq!(pins.read(&keys).await, [None]);
}
