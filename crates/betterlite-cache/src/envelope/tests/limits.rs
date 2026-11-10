#![cfg(test)]

use std::sync::Arc;

use rstest::rstest;

use super::support_tests::{
    FakeBackend, NOW, control, empty_request, envelope, fake_cache, key_settings, reply_response,
};
use crate::envelope::facade::EnvelopeSettings;
use crate::envelope::request::EnvelopeRequest;
use crate::types::cache_type::CacheMode;
use crate::types::control::DynamicCacheControl;

fn request_with(control: DynamicCacheControl) -> EnvelopeRequest {
    let mut request = empty_request();
    request.cache_key = Some("key-1".to_owned());
    request.control = control;
    request
}

#[test]
fn sync_read_uses_s_maxage_before_s_max_age() {
    let cached = envelope(Some(NOW - 50.0_f64), reply_response());
    let fake = Arc::new(FakeBackend::new(Some(cached)));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    assert_eq!(
        cache.get_cache(None, &mut request_with(control(None, Some(40), Some(60)))),
        None
    );
    assert_eq!(
        cache.get_cache(None, &mut request_with(control(None, Some(60), Some(40)))),
        Some(reply_response())
    );
}

#[tokio::test]
async fn async_read_uses_s_max_age_before_s_maxage() {
    let cached = envelope(Some(NOW - 50.0_f64), reply_response());
    let fake = Arc::new(FakeBackend::new(Some(cached)));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    assert_eq!(
        cache
            .async_get_cache(None, &mut request_with(control(None, Some(40), Some(60))))
            .await,
        Some(reply_response())
    );
    assert_eq!(
        cache
            .async_get_cache(None, &mut request_with(control(None, Some(60), Some(40))))
            .await,
        None
    );
}

#[rstest]
#[case::settings_float(Some(600.5_f64), None, Some(600.5_f64))]
#[case::control_overrides(Some(600.5_f64), Some(30), Some(30.0_f64))]
#[case::zero_control(Some(600.5_f64), Some(0), Some(0.0_f64))]
#[case::absent(None, None, None)]
fn add_cache_writes_the_resolved_ttl(
    #[case] settings_ttl: Option<f64>,
    #[case] control_ttl: Option<i64>,
    #[case] expected: Option<f64>,
) {
    let fake = Arc::new(FakeBackend::new(None));
    let settings = EnvelopeSettings {
        ttl: settings_ttl,
        redis_flush_size: None,
    };
    let cache = fake_cache(&fake, key_settings(CacheMode::DefaultOn), settings);
    cache.add_cache(
        &mut request_with(control(control_ttl, None, None)),
        &reply_response(),
    );
    assert_eq!(fake.sets()[0].2, expected);
}
