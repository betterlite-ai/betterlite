#![cfg(test)]

mod support;

use std::collections::BTreeSet;
use std::sync::PoisonError;

use crate::limiter_records::{
    CacheCounterValue, PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY,
    ParallelSlotAcquisition, RateLimitDescriptor, RateLimitResponse, RateLimitStatus,
    RateLimitType, ReservationBackend, ReservationWindowIdentity,
};
use crate::project_admission::ProjectAdmissionError;

use self::support::{
    Effect, Fixture, GAUGE, LogCapture, PROJECT, START, StashSnapshot, data, limit, request,
    scopes, tokens_key, window,
};

const ITPM: &str = PROJECT_ITPM_DESCRIPTOR_KEY;
const OTPM: &str = PROJECT_OTPM_DESCRIPTOR_KEY;
const CHAT: &str = r#"{"messages":[{"role":"user","content":"hello"}],"litellm_call_id":"call-1"}"#;

fn status(
    key: &str,
    value: &str,
    code: &str,
    limit_tokens: i64,
    remaining: i64,
) -> RateLimitStatus {
    RateLimitStatus {
        code: code.to_owned(),
        current_limit: limit_tokens,
        limit_remaining: remaining,
        rate_limit_type: RateLimitType::Tokens,
        descriptor_key: key.to_owned(),
        descriptor_value: Some(value.to_owned()),
    }
}

fn local_window(key: &str, value: &str) -> ReservationWindowIdentity {
    ReservationWindowIdentity {
        counter_key: tokens_key(key, value),
        window_start: START.to_string(),
        backend: ReservationBackend::Local,
    }
}

fn stored_windows(
    fixture: &Fixture,
) -> (
    BTreeSet<ReservationWindowIdentity>,
    BTreeSet<ReservationWindowIdentity>,
) {
    let stash = fixture.slot.get().expect("the admission claimed the stash");
    let state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    (
        state.itpm_reserved_windows.clone(),
        state.otpm_reserved_windows.clone(),
    )
}

fn stored_response(fixture: &Fixture) -> Option<RateLimitResponse> {
    let stash = fixture.slot.get().expect("the admission claimed the stash");
    let state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    state.rate_limit_response.clone()
}

fn seeded(
    windows: &[[(String, CacheCounterValue); 2]],
    extra: &[(String, CacheCounterValue)],
) -> Vec<(String, CacheCounterValue)> {
    let mut entries = windows.iter().flatten().cloned().collect::<Vec<_>>();
    entries.extend(extra.iter().cloned());
    entries
}

fn slot_seeded(fixture: &Fixture) {
    let stash = fixture.slot.get_or_create();
    stash
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .parallel_slot = Some(ParallelSlotAcquisition {
        slot_id: "slot-1".to_owned(),
        counter_keys: vec![GAUGE.to_owned()],
    });
}

#[tokio::test]
async fn without_project_io_descriptors_the_stash_is_claimed_and_nothing_is_reserved() {
    let fixture = Fixture::new(&[], Some(42));
    let descriptors = [limit("model_per_key", "key1:gpt-4", 100)];
    let mut body = data(CHAT);

    fixture
        .admit(&request(&descriptors, &[], 0), &mut body)
        .await
        .unwrap();

    assert_eq!(fixture.stash().owner.as_deref(), Some("call-1"));
    assert!(fixture.recorder.effects().is_empty());
    assert!(fixture.counter.models().is_empty());
    assert_eq!(body, data(CHAT));
}

#[tokio::test]
async fn a_call_id_that_is_not_text_leaves_the_claimed_stash_unowned() {
    let fixture = Fixture::new(&[], Some(42));
    let mut body = data(r#"{"messages":[],"litellm_call_id":7}"#);

    fixture
        .admit(&request(&[], &[], 0), &mut body)
        .await
        .unwrap();

    assert_eq!(fixture.stash().owner, None);
}

#[tokio::test]
async fn an_itpm_reservation_stores_its_tokens_scopes_windows_and_response() {
    let fixture = Fixture::new(&seeded(&[window(ITPM, PROJECT, 100)], &[]), Some(42));
    let descriptors = [limit(ITPM, PROJECT, 1000)];
    let mut body = data(CHAT);

    fixture
        .admit(&request(&descriptors, &[], 0), &mut body)
        .await
        .unwrap();

    assert_eq!(
        fixture.stash(),
        StashSnapshot {
            owner: Some("call-1".to_owned()),
            itpm: (42, scopes(&[(ITPM, PROJECT)])),
            otpm: (0, BTreeSet::new()),
            released: false,
            slot_held: false,
        }
    );
    let windows = BTreeSet::from([local_window(ITPM, PROJECT)]);
    assert_eq!(stored_windows(&fixture), (windows.clone(), BTreeSet::new()));
    assert_eq!(
        stored_response(&fixture),
        Some(RateLimitResponse {
            overall_code: "OK".to_owned(),
            statuses: vec![status(ITPM, PROJECT, "OK", 1000, 858)],
            reservation_windows: Some(windows),
        })
    );
    assert_eq!(
        fixture.recorder.effects(),
        vec![
            Effect::Read(format!("{{{ITPM}:{PROJECT}}}:window")),
            Effect::Read(tokens_key(ITPM, PROJECT)),
            Effect::Write(tokens_key(ITPM, PROJECT), CacheCounterValue::Int(142), 60),
        ]
    );
    assert_eq!(fixture.counter.models(), vec!["gpt-4".to_owned()]);
    assert_eq!(body, data(CHAT));
}

#[tokio::test]
async fn the_smallest_otpm_limit_sets_the_output_estimate_and_the_written_cap() {
    let other = "p2:gpt-4";
    let seeded_windows = [
        window(ITPM, PROJECT, 0),
        window(OTPM, PROJECT, 0),
        window(OTPM, other, 0),
    ];
    let fixture = Fixture::new(&seeded(&seeded_windows, &[]), Some(42));
    let descriptors = [
        limit(ITPM, PROJECT, 100_000),
        limit(OTPM, PROJECT, 1000),
        limit(OTPM, other, 400),
    ];
    let mut body = data(CHAT);

    fixture
        .admit(&request(&descriptors, &[], 0), &mut body)
        .await
        .unwrap();

    let snapshot = fixture.stash();
    assert_eq!(snapshot.itpm, (42, scopes(&[(ITPM, PROJECT)])));
    assert_eq!(
        snapshot.otpm,
        (100, scopes(&[(OTPM, PROJECT), (OTPM, other)]))
    );
    assert_eq!(
        stored_windows(&fixture),
        (
            BTreeSet::from([local_window(ITPM, PROJECT)]),
            BTreeSet::from([local_window(OTPM, PROJECT), local_window(OTPM, other)]),
        )
    );
    assert_eq!(
        body,
        data(
            r#"{"messages":[{"role":"user","content":"hello"}],"litellm_call_id":"call-1","max_tokens":100}"#
        )
    );
}

#[tokio::test]
async fn an_explicit_output_cap_is_reserved_as_given_and_left_in_place() {
    let fixture = Fixture::new(&seeded(&[window(OTPM, PROJECT, 0)], &[]), Some(42));
    let descriptors = [limit(OTPM, PROJECT, 400)];
    let text = r#"{"messages":[{"role":"user","content":"hello"}],"max_tokens":30}"#;
    let mut body = data(text);

    fixture
        .admit(&request(&descriptors, &[], 0), &mut body)
        .await
        .unwrap();

    assert_eq!(fixture.stash().otpm, (30, scopes(&[(OTPM, PROJECT)])));
    assert_eq!(fixture.stash().itpm, (0, BTreeSet::new()));
    assert_eq!(body, data(text));
}

#[tokio::test]
async fn an_explicit_zero_output_cap_reserves_no_output_and_keeps_earlier_otpm_fields() {
    let fixture = Fixture::new(
        &seeded(&[window(ITPM, PROJECT, 0), window(OTPM, PROJECT, 0)], &[]),
        Some(42),
    );
    let earlier = scopes(&[(OTPM, "earlier")]);
    {
        let stash = fixture.slot.get_or_create();
        let mut state = stash.state.lock().unwrap();
        state.otpm_reserved_tokens = 9;
        state.otpm_reserved_scopes = earlier.clone();
    }
    let descriptors = [limit(ITPM, PROJECT, 1000), limit(OTPM, PROJECT, 400)];
    let mut body = data(r#"{"messages":[{"role":"user","content":"hello"}],"max_tokens":0}"#);

    fixture
        .admit(&request(&descriptors, &[], 0), &mut body)
        .await
        .unwrap();

    assert_eq!(fixture.stash().itpm, (42, scopes(&[(ITPM, PROJECT)])));
    assert_eq!(fixture.stash().otpm, (9, earlier));
}

#[tokio::test]
async fn an_otpm_only_reservation_keeps_earlier_itpm_fields() {
    let fixture = Fixture::new(&seeded(&[window(OTPM, PROJECT, 0)], &[]), Some(42));
    let earlier = scopes(&[(ITPM, "earlier")]);
    {
        let stash = fixture.slot.get_or_create();
        let mut state = stash.state.lock().unwrap();
        state.itpm_reserved_tokens = 5;
        state.itpm_reserved_scopes = earlier.clone();
    }
    let descriptors = [limit(OTPM, PROJECT, 400)];
    let mut body = data(CHAT);

    fixture
        .admit(&request(&descriptors, &[], 0), &mut body)
        .await
        .unwrap();

    assert_eq!(fixture.stash().itpm, (5, earlier));
    assert_eq!(fixture.stash().otpm, (100, scopes(&[(OTPM, PROJECT)])));
}

#[tokio::test]
async fn empty_estimates_reserve_one_token_in_each_bucket() {
    let fixture = Fixture::new(
        &seeded(&[window(ITPM, PROJECT, 0), window(OTPM, PROJECT, 0)], &[]),
        Some(0),
    );
    let descriptors = [limit(ITPM, PROJECT, 1000), limit(OTPM, PROJECT, 400)];
    let mut body = data(r#"{"messages":[]}"#);

    fixture
        .admit(&request(&descriptors, &[], 0), &mut body)
        .await
        .unwrap();

    assert_eq!(fixture.stash().itpm.0, 1);
    assert_eq!(fixture.stash().otpm.0, 1);
    assert_eq!(body, data(r#"{"messages":[],"max_tokens":100}"#));
}

fn over_limit_fixture() -> Fixture {
    let fixture = Fixture::new(
        &seeded(
            &[window(ITPM, PROJECT, 90)],
            &[(GAUGE.to_owned(), CacheCounterValue::Int(2))],
        ),
        Some(50),
    );
    slot_seeded(&fixture);
    fixture
}

fn over_limit_descriptors() -> [RateLimitDescriptor; 3] {
    [
        limit("model_per_key", "key1:gpt-4", 10_000),
        limit(ITPM, PROJECT, 100),
        limit(OTPM, PROJECT, 400),
    ]
}

fn rejection_message(result: Result<(), ProjectAdmissionError>) -> String {
    let Err(ProjectAdmissionError::RateLimited(error)) = result else {
        panic!("the over-limit reservation raises the 429");
    };
    assert_eq!(error.model, "gpt-4");
    error.message
}

#[tokio::test]
async fn an_over_limit_reservation_refunds_the_tpm_reservation_and_releases_the_slot() {
    let fixture = over_limit_fixture();
    let descriptors = over_limit_descriptors();
    let tpm_scopes = [("model_per_key".to_owned(), "key1:gpt-4".to_owned())];
    let mut body = data(CHAT);

    let result = fixture
        .admit(&request(&descriptors, &tpm_scopes, 200), &mut body)
        .await;

    assert_eq!(
        rejection_message(result),
        "Rate limit exceeded for model_per_project_itpm: p1:gpt-4. Limit type: tokens. Current limit: 100, Remaining: 10. Limit resets at: 2023-11-14 22:14:20 UTC"
    );
    assert_eq!(
        fixture.recorder.effects(),
        vec![
            Effect::Read(format!("{{{ITPM}:{PROJECT}}}:window")),
            Effect::Read(tokens_key(ITPM, PROJECT)),
            Effect::Pipeline(vec![(
                tokens_key("model_per_key", "key1:gpt-4"),
                -200.0_f64,
                Some(60)
            )]),
            Effect::Read(GAUGE.to_owned()),
            Effect::Write(GAUGE.to_owned(), CacheCounterValue::Int(1), 3600),
        ]
    );
    assert_eq!(
        fixture.stash(),
        StashSnapshot {
            owner: Some("call-1".to_owned()),
            itpm: (0, BTreeSet::new()),
            otpm: (0, BTreeSet::new()),
            released: true,
            slot_held: false,
        }
    );
    let stash = fixture.slot.get().unwrap();
    assert_eq!(stash.state.lock().unwrap().rate_limit_response, None);
    let capped = r#"{"messages":[{"role":"user","content":"hello"}],"litellm_call_id":"call-1","max_tokens":100}"#;
    assert_eq!(body, data(capped));
}

#[tokio::test]
async fn an_over_limit_reservation_without_a_tpm_reservation_refunds_nothing() {
    let fixture = over_limit_fixture();
    let descriptors = over_limit_descriptors();
    let tpm_scopes = [("model_per_key".to_owned(), "key1:gpt-4".to_owned())];
    let mut body = data(CHAT);

    let result = fixture
        .admit(&request(&descriptors, &tpm_scopes, 0), &mut body)
        .await;

    assert!(
        rejection_message(result).starts_with("Rate limit exceeded for model_per_project_itpm")
    );
    assert_eq!(
        fixture.recorder.effects(),
        vec![
            Effect::Read(format!("{{{ITPM}:{PROJECT}}}:window")),
            Effect::Read(tokens_key(ITPM, PROJECT)),
            Effect::Read(GAUGE.to_owned()),
            Effect::Write(GAUGE.to_owned(), CacheCounterValue::Int(1), 3600),
        ]
    );
    let snapshot = fixture.stash();
    assert!(!snapshot.released);
    assert!(!snapshot.slot_held);
}

#[tokio::test]
async fn io_statuses_extend_the_stored_rate_limit_response() {
    let fixture = Fixture::new(&seeded(&[window(ITPM, PROJECT, 100)], &[]), Some(42));
    let rpm = RateLimitStatus {
        code: "OK".to_owned(),
        current_limit: 10,
        limit_remaining: 9,
        rate_limit_type: RateLimitType::Requests,
        descriptor_key: "api_key".to_owned(),
        descriptor_value: Some("k1".to_owned()),
    };
    fixture
        .slot
        .get_or_create()
        .state
        .lock()
        .unwrap()
        .rate_limit_response = Some(RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses: vec![rpm.clone()],
        reservation_windows: None,
    });
    let descriptors = [limit(ITPM, PROJECT, 1000)];
    let mut body = data(CHAT);

    fixture
        .admit(&request(&descriptors, &[], 0), &mut body)
        .await
        .unwrap();

    let stash = fixture.slot.get().unwrap();
    assert_eq!(
        stash.state.lock().unwrap().rate_limit_response,
        Some(RateLimitResponse {
            overall_code: "OK".to_owned(),
            statuses: vec![rpm, status(ITPM, PROJECT, "OK", 1000, 858)],
            reservation_windows: None,
        })
    );
}

#[tokio::test]
async fn a_reservation_logs_the_reserved_amounts_and_the_model() {
    let capture = LogCapture::start();
    let fixture = Fixture::new(&seeded(&[window(ITPM, PROJECT, 100)], &[]), Some(42));
    let descriptors = [limit(ITPM, PROJECT, 1000)];
    let mut body = data(CHAT);

    fixture
        .admit(&request(&descriptors, &[], 0), &mut body)
        .await
        .unwrap();

    let text = capture.text();
    assert!(text.contains("ITPM/OTPM tokens reserved"), "{text}");
    assert!(text.contains("itpm=42"), "{text}");
    assert!(text.contains("otpm=0"), "{text}");
    assert!(text.contains(r#"model="gpt-4""#), "{text}");
}
