#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use crate::api::batch_enqueued_refund::record::ReservationRecord;
use crate::api::batch_enqueued_refund::{BatchEnqueuedRefundInput, BatchEnqueuedRefundResult};
use crate::api::batch_input_accounting::error::BatchHttpError;
use crate::api::batch_limit_descriptors::input::BatchLimitPrincipal;
use crate::api::batch_precall::BatchPrecall;
use crate::api::batch_precall::BatchPrecallError;
use crate::api::batch_precall::input::{BatchPrecallCall, BatchPrecallInput, BatchRetrieveCall};
use crate::api::batch_precall::ports::{BatchPrecallSkip, BatchRefundSink};
use crate::api::batch_precall::result::{
    BatchCreateOutcome, BatchDiagnostic, BatchPrecallResult, BatchTerminalResult,
};
use crate::api::request_destination::RequestData;

use self::support::{
    CACHE_KEY, CACHE_NAME, CALL_TYPE, Effect, RETRIEVE_CALL_TYPE, RemoteAnswer, Rig, ScriptedStash,
    StageName, batch_response, break_counter, create_data, create_input, debug_line, descriptor,
    envelope, error_line, failure_input, key_scope, namespaced, namespaced_as, operation,
    plain_principal, principal_with_scope, rejection, reservation, retrieve_data, retrieve_input,
    retrieve_input_with_metadata, seed_counter, stored_counter, stored_record, terminal_input,
};

mod support;

const CACHE_FAILURE_PREFIX: &str = concat!(
    "litellm.proxy.hooks.batch_redis_get.py::async_pre_call_hook(): Exception occu",
    "red - "
);

fn expected_create_effects(enqueued: bool) -> Vec<Effect> {
    let mut effects = vec![
        Effect::Skip {
            has_enqueued_scopes: enqueued,
        },
        Effect::Account {
            file_id: String::from("file-1"),
            provider: String::from("openai"),
            descriptors: Some(vec![descriptor()]),
        },
    ];
    if enqueued {
        effects.push(Effect::Reserve {
            tokens: 120,
            scopes: vec![key_scope()],
        });
    } else {
        effects.push(Effect::Charge {
            tokens: 120,
            descriptors: Some(vec![descriptor()]),
        });
    }
    effects
}

fn expected_create_diagnostics(enqueued: bool) -> Vec<BatchDiagnostic> {
    let success = if enqueued {
        "Batch enqueued-token reservation succeeded"
    } else {
        "Batch rate limit check passed, counters incremented"
    };
    vec![
        debug_line("Batch rate limiter: Handling batch creation rate limiting"),
        debug_line("Counting tokens from batch input file: file-1"),
        debug_line("Batch input file usage - Tokens: 120, Requests: 2"),
        debug_line(success),
    ]
}

// acreate_batch counts the input file, writes the counts onto the request, then reserves enqueued tokens or charges counters
#[rstest]
#[case::charges_counters(false)]
#[case::reserves_enqueued_tokens(true)]
#[tokio::test]
async fn create_precall_reserve(#[case] enqueued: bool) {
    let rig = Rig::new();
    let principal = if enqueued {
        principal_with_scope()
    } else {
        plain_principal()
    };
    let input = create_input(CALL_TYPE, principal, create_data("file-1"));
    let BatchPrecallResult::Create(created) = rig.run(&input).await.unwrap() else {
        panic!("a create call answers with the create result");
    };
    assert_eq!(created.outcome, BatchCreateOutcome::Passed);
    assert_eq!(
        created.data["_batch_token_count"],
        Json::Number(Number::from(120_i64))
    );
    assert_eq!(
        created.data["_batch_request_count"],
        Json::Number(Number::from(2_i64))
    );
    assert_eq!(rig.stages.effects(), expected_create_effects(enqueued));
    assert_eq!(created.diagnostics, expected_create_diagnostics(enqueued));
}

// the hook returns the request untouched for another call type, without a file id, and when the skip decision says so
#[rstest]
#[case::other_call_type("acreate_file", "file-1", false)]
#[case::missing_file_id(CALL_TYPE, "", false)]
#[case::skipped_file(CALL_TYPE, "file-1", true)]
#[tokio::test]
async fn create_precall_passes_through(
    #[case] call_type: &str,
    #[case] file_id: &str,
    #[case] skip: bool,
) {
    let mut rig = Rig::new();
    rig.stages.skip = BatchPrecallSkip {
        skip,
        descriptors: None,
    };
    let input = create_input(call_type, plain_principal(), create_data(file_id));
    let BatchPrecallResult::Create(created) = rig.run(&input).await.unwrap() else {
        panic!("a create call answers with the create result");
    };
    assert_eq!(created.outcome, BatchCreateOutcome::Passed);
    assert_eq!(created.data, create_data(file_id));
    if call_type != CALL_TYPE {
        assert!(rig.stages.effects().is_empty());
        let message = format!(
            "Batch rate limiter: Not handling batch creation rate limiting for call type: {call_type}"
        );
        assert_eq!(created.diagnostics, vec![debug_line(&message)]);
    } else if file_id.is_empty() {
        assert!(rig.stages.effects().is_empty());
        assert_eq!(
            created.diagnostics,
            vec![
                debug_line("Batch rate limiter: Handling batch creation rate limiting"),
                debug_line("No input_file_id in batch request, skipping rate limiting"),
            ]
        );
    } else {
        assert_eq!(
            rig.stages.effects(),
            vec![Effect::Skip {
                has_enqueued_scopes: false
            }]
        );
        assert_eq!(
            created.diagnostics,
            vec![debug_line(
                "Batch rate limiter: Handling batch creation rate limiting"
            )]
        );
    }
}

// a rejection is an answer rather than an HTTP error: the counts stay on the data and the success line is never logged
#[tokio::test]
async fn create_precall_rejects() {
    let mut rig = Rig::new();
    rig.stages.rejection = Some(rejection());
    let input = create_input(CALL_TYPE, principal_with_scope(), create_data("file-1"));
    let BatchPrecallResult::Create(created) = rig.run(&input).await.unwrap() else {
        panic!("a create call answers with the create result");
    };
    assert_eq!(created.outcome, BatchCreateOutcome::Rejected(rejection()));
    assert_eq!(
        created.data["_batch_token_count"],
        Json::Number(Number::from(120_i64))
    );
    assert_eq!(
        created.diagnostics,
        vec![
            debug_line("Batch rate limiter: Handling batch creation rate limiting"),
            debug_line("Counting tokens from batch input file: file-1"),
            debug_line("Batch input file usage - Tokens: 120, Requests: 2"),
        ]
    );
}

// except HTTPException: raise leaves the hook as an HTTP error with the lines it logged before the failure
#[tokio::test]
async fn create_precall_reraises_http() {
    let mut rig = Rig::new();
    rig.stages.fail_at = Some(StageName::Reserve);
    rig.stages.http_status = Some(403);
    let input = create_input(CALL_TYPE, principal_with_scope(), create_data("file-1"));
    let BatchPrecallResult::Create(created) = rig.run(&input).await.unwrap() else {
        panic!("a create call answers with the create result");
    };
    assert_eq!(
        created.outcome,
        BatchCreateOutcome::Raised(BatchHttpError {
            status: 403,
            detail: Json::Text(String::from("the batch file was rejected")),
        })
    );
    assert_eq!(
        created.diagnostics,
        vec![
            debug_line("Batch rate limiter: Handling batch creation rate limiting"),
            debug_line("Counting tokens from batch input file: file-1"),
            debug_line("Batch input file usage - Tokens: 120, Requests: 2"),
        ]
    );
    assert_eq!(rig.stages.effects().len(), 3);
    assert_eq!(
        created.data["_batch_token_count"],
        Json::Number(Number::from(120_i64))
    );
}

// any other failure is logged and the request goes on without the counts
#[tokio::test]
async fn create_precall_swallows_stage_failure() {
    let mut rig = Rig::new();
    rig.stages.fail_at = Some(StageName::Account);
    let input = create_input(CALL_TYPE, plain_principal(), create_data("file-1"));
    let BatchPrecallResult::Create(created) = rig.run(&input).await.unwrap() else {
        panic!("a create call answers with the create result");
    };
    assert_eq!(created.outcome, BatchCreateOutcome::Passed);
    assert_eq!(created.data, create_data("file-1"));
    assert_eq!(
        created.diagnostics,
        vec![
            debug_line("Batch rate limiter: Handling batch creation rate limiting"),
            debug_line("Counting tokens from batch input file: file-1"),
            error_line(String::from(
                "Error in batch rate limiting: the scripted stage Account failed"
            )),
        ]
    );
}

// a failure after the counts were written keeps them on the request the route continues with
#[tokio::test]
async fn create_precall_swallows_settle_failure() {
    let mut rig = Rig::new();
    rig.stages.fail_at = Some(StageName::Reserve);
    let input = create_input(CALL_TYPE, principal_with_scope(), create_data("file-1"));
    let BatchPrecallResult::Create(created) = rig.run(&input).await.unwrap() else {
        panic!("a create call answers with the create result");
    };
    assert_eq!(created.outcome, BatchCreateOutcome::Passed);
    assert_eq!(
        created.data["_batch_token_count"],
        Json::Number(Number::from(120_i64))
    );
    assert_eq!(
        created.data["_batch_request_count"],
        Json::Number(Number::from(2_i64))
    );
    assert_eq!(
        created.diagnostics,
        vec![
            debug_line("Batch rate limiter: Handling batch creation rate limiting"),
            debug_line("Counting tokens from batch input file: file-1"),
            debug_line("Batch input file usage - Tokens: 120, Requests: 2"),
            error_line(String::from(
                "Error in batch rate limiting: the scripted stage Reserve failed"
            )),
        ]
    );
}

// a request field the typed record cannot hold is named, where the source would have carried the value into the limiter
#[rstest]
#[case::input_file_id("input_file_id")]
#[case::custom_llm_provider("custom_llm_provider")]
#[tokio::test]
async fn create_precall_names_wrong_fields(#[case] field: &str) {
    let rig = Rig::new();
    let mut data = create_data("file-1");
    data.insert(field.to_owned(), Json::Number(Number::from(7_i64)));
    let input = create_input(CALL_TYPE, plain_principal(), data);
    let failure = rig.run(&input).await.unwrap_err();
    assert!(matches!(failure, BatchPrecallError::Field(name) if name == field));
}

// a principal field the typed record cannot hold is named, on the scope resolution and on the cache key
#[rstest]
#[case::api_key("api_key")]
#[case::metadata("metadata")]
#[tokio::test]
async fn create_precall_names_wrong_principal_fields(#[case] field: &str) {
    let rig = Rig::new();
    let mut fields = BTreeMap::new();
    fields.insert(field.to_owned(), JsonData::Number(Number::from(7_i64)));
    let input = create_input(
        CALL_TYPE,
        BatchLimitPrincipal(fields),
        create_data("file-1"),
    );
    let failure = rig.run(&input).await.unwrap_err();
    assert!(matches!(failure, BatchPrecallError::Field(name) if name == field));
}

// the provider default is openai and the request's value reaches the file accounting
#[tokio::test]
async fn create_precall_reads_provider() {
    let rig = Rig::new();
    let mut data = create_data("file-1");
    data.insert(
        String::from("custom_llm_provider"),
        Json::Text(String::from("anthropic")),
    );
    let input = create_input(CALL_TYPE, plain_principal(), data);
    let BatchPrecallResult::Create(created) = rig.run(&input).await.unwrap() else {
        panic!("a create call answers with the create result");
    };
    assert_eq!(created.outcome, BatchCreateOutcome::Passed);
    assert_eq!(
        rig.stages.effects(),
        vec![
            Effect::Skip {
                has_enqueued_scopes: false
            },
            Effect::Account {
                file_id: String::from("file-1"),
                provider: String::from("anthropic"),
                descriptors: Some(vec![descriptor()]),
            },
            Effect::Charge {
                tokens: 120,
                descriptors: Some(vec![descriptor()]),
            },
        ]
    );
}

// a skip decision without descriptors leaves the accounting and the charge without a descriptor list
#[tokio::test]
async fn create_precall_charges_without_descriptors() {
    let mut rig = Rig::new();
    rig.stages.skip.descriptors = None;
    let input = create_input(CALL_TYPE, plain_principal(), create_data("file-1"));
    let BatchPrecallResult::Create(created) = rig.run(&input).await.unwrap() else {
        panic!("a create call answers with the create result");
    };
    assert_eq!(created.outcome, BatchCreateOutcome::Passed);
    assert_eq!(
        rig.stages.effects(),
        vec![
            Effect::Skip {
                has_enqueued_scopes: false
            },
            Effect::Account {
                file_id: String::from("file-1"),
                provider: String::from("openai"),
                descriptors: None,
            },
            Effect::Charge {
                tokens: 120,
                descriptors: None,
            },
        ]
    );
}

// async_post_call_failure_hook: refund the stashed reservation, then apply the TPD refunds
#[tokio::test]
async fn create_failure_refund() {
    let mut rig = Rig::new();
    seed_counter(&rig.counters, 500);
    rig.stash = ScriptedStash::holding(reservation(300), vec![operation()]);
    let BatchPrecallResult::CreateFailure(result) = rig.run(&failure_input()).await.unwrap() else {
        panic!("a failed create answers with the refund result");
    };
    assert_eq!(
        result.refunded,
        Some(ReservationRecord::from(&reservation(300)))
    );
    assert_eq!(result.applied, vec![operation()]);
    assert!(result.diagnostics.is_empty());
    assert_eq!(stored_counter(&rig.counters), Some(200));
    assert_eq!(rig.applier.applied(), vec![vec![operation()]]);
    assert_eq!(rig.stash.reservation(), None);
    assert!(rig.stash.operations().is_empty());
}

// nothing reserved and nothing to release: the hook has no work and logs nothing
#[tokio::test]
async fn create_failure_without_reservation() {
    let rig = Rig::new();
    let BatchPrecallResult::CreateFailure(result) = rig.run(&failure_input()).await.unwrap() else {
        panic!("a failed create answers with the refund result");
    };
    assert_eq!(result.refunded, None);
    assert!(result.applied.is_empty());
    assert!(result.diagnostics.is_empty());
}

// a failed TPD apply is logged after the reservation was already refunded
#[tokio::test]
async fn create_failure_reports_applier_error() {
    let mut rig = Rig::new();
    seed_counter(&rig.counters, 500);
    rig.stash = ScriptedStash::holding(reservation(300), vec![operation()]);
    rig.applier.failure = Some(String::from("the counter pipeline is down"));
    let BatchPrecallResult::CreateFailure(result) = rig.run(&failure_input()).await.unwrap() else {
        panic!("a failed create answers with the refund result");
    };
    assert_eq!(
        result.refunded,
        Some(ReservationRecord::from(&reservation(300)))
    );
    assert!(result.applied.is_empty());
    assert_eq!(
        result.diagnostics,
        vec![error_line(String::from(
            "Error releasing TPM reservation on post-call failure: the counter pipeline is down"
        ))]
    );
    assert_eq!(stored_counter(&rig.counters), Some(200));
}

// a failed refund stops the hook before the TPD operations are taken
#[tokio::test]
async fn create_failure_reports_store_error() {
    let mut rig = Rig::new();
    break_counter(&rig.counters);
    rig.stash = ScriptedStash::holding(reservation(300), vec![operation()]);
    let BatchPrecallResult::CreateFailure(result) = rig.run(&failure_input()).await.unwrap() else {
        panic!("a failed create answers with the refund result");
    };
    assert_eq!(result.refunded, None);
    assert!(result.applied.is_empty());
    assert_eq!(
        result.diagnostics,
        vec![error_line(String::from(
            "Error releasing TPM reservation on post-call failure: the local enqueued-token store failed"
        ))]
    );
    assert_eq!(rig.stash.operations(), vec![operation()]);
    assert!(rig.applier.applied().is_empty());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheScenario {
    MemoryHit,
    WarmFromRedis,
    WithinMaxAge,
    AtMaxAge,
    PastMaxAge,
}

fn cached_rig(scenario: CacheScenario) -> Rig {
    let mut rig = Rig::new();
    match scenario {
        CacheScenario::MemoryHit => {
            rig.memory
                .store(CACHE_KEY, envelope(None, batch_response()));
        }
        CacheScenario::WarmFromRedis => {
            rig.remote.scan = RemoteAnswer::Value(vec![String::from(CACHE_KEY)]);
            rig.remote.batch_get = RemoteAnswer::Value(vec![(
                String::from(CACHE_KEY),
                envelope(None, batch_response()),
            )]);
        }
        CacheScenario::WithinMaxAge | CacheScenario::AtMaxAge | CacheScenario::PastMaxAge => {
            rig.memory
                .store(CACHE_KEY, envelope(Some(10.0_f64), batch_response()));
        }
    }
    rig
}

fn expected_cached(scenario: CacheScenario) -> Option<Json> {
    match scenario {
        CacheScenario::MemoryHit | CacheScenario::WarmFromRedis => {
            Some(envelope(None, batch_response()))
        }
        CacheScenario::WithinMaxAge | CacheScenario::AtMaxAge => Some(batch_response()),
        CacheScenario::PastMaxAge => None,
    }
}

fn expected_cache_diagnostics(scenario: CacheScenario) -> Vec<BatchDiagnostic> {
    match scenario {
        CacheScenario::MemoryHit
        | CacheScenario::WithinMaxAge
        | CacheScenario::AtMaxAge
        | CacheScenario::PastMaxAge => Vec::new(),
        CacheScenario::WarmFromRedis => vec![
            debug_line(&format!("cache_key_name: {CACHE_NAME}")),
            debug_line(&format!("redis keys: ['{CACHE_KEY}']")),
        ],
    }
}

fn namespaced_data() -> RequestData {
    let mut data = retrieve_data();
    data.insert(String::from("metadata"), namespaced());
    data
}

// async_get_cache: memory first, the warmed redis key next, and the envelope's age checked against the cache control
#[rstest]
#[case::memory_hit(CacheScenario::MemoryHit)]
#[case::warm_from_redis(CacheScenario::WarmFromRedis)]
#[case::within_max_age(CacheScenario::WithinMaxAge)]
#[case::at_max_age(CacheScenario::AtMaxAge)]
#[case::past_max_age(CacheScenario::PastMaxAge)]
#[tokio::test]
async fn retrieve_cache_hit(#[case] scenario: CacheScenario) {
    let rig = cached_rig(scenario);
    let max_age = match scenario {
        CacheScenario::WithinMaxAge => Some(100),
        CacheScenario::AtMaxAge => Some(90),
        CacheScenario::PastMaxAge => Some(30),
        CacheScenario::MemoryHit | CacheScenario::WarmFromRedis => None,
    };
    let input = retrieve_input(Some(CACHE_KEY), max_age);
    let BatchPrecallResult::Retrieve(retrieved) = rig.run(&input).await.unwrap() else {
        panic!("a retrieve answers with the retrieve result");
    };
    assert_eq!(retrieved.cached, expected_cached(scenario));
    assert_eq!(retrieved.raised, None);
    assert_eq!(retrieved.diagnostics, expected_cache_diagnostics(scenario));
    assert_eq!(retrieved.data, namespaced_data());
    assert!(retrieved.printed.is_empty());
    match scenario {
        CacheScenario::WarmFromRedis => {
            assert_eq!(rig.remote.scan_patterns(), vec![String::from(CACHE_NAME)]);
            assert_eq!(rig.remote.scan_counts(), vec![100]);
            assert_eq!(rig.remote.batch_keys(), vec![vec![String::from(CACHE_KEY)]]);
            assert_eq!(
                rig.memory.pipeline_keys(),
                vec![vec![String::from(CACHE_KEY)]]
            );
            assert_eq!(rig.memory.pipeline_ttls(), vec![60.0_f64]);
            assert!(rig.remote.hits().is_empty());
        }
        CacheScenario::MemoryHit => {
            assert!(rig.remote.scan_patterns().is_empty());
            assert!(rig.remote.hits().is_empty());
        }
        CacheScenario::WithinMaxAge | CacheScenario::AtMaxAge | CacheScenario::PastMaxAge => {}
    }
}

// a cached value over the configured JSON limit is a miss, like every payload betterlite bounds
#[tokio::test]
async fn retrieve_cache_value_over_limit() {
    let mut rig = Rig::new();
    rig.limits.max_json_bytes = 1;
    rig.memory
        .store(CACHE_KEY, envelope(None, batch_response()));
    let input = retrieve_input(Some(CACHE_KEY), None);
    let BatchPrecallResult::Retrieve(retrieved) = rig.run(&input).await.unwrap() else {
        panic!("a retrieve answers with the retrieve result");
    };
    assert_eq!(retrieved.cached, None);
    assert!(retrieved.diagnostics.is_empty());
    assert_eq!(retrieved.data, namespaced_data());
}

// a memory miss reads the backing store through the request's cache key and warms memory for a minute
#[tokio::test]
async fn retrieve_backing_store_hit() {
    let mut rig = Rig::new();
    rig.remote.key = Some(String::from(CACHE_KEY));
    rig.remote.get = RemoteAnswer::Value(Some(envelope(None, batch_response())));
    rig.verbose_stdout = true;
    let input = retrieve_input(None, None);
    let BatchPrecallResult::Retrieve(retrieved) = rig.run(&input).await.unwrap() else {
        panic!("a retrieve answers with the retrieve result");
    };
    assert_eq!(retrieved.cached, Some(envelope(None, batch_response())));
    assert_eq!(rig.remote.hits(), vec![String::from(CACHE_KEY)]);
    assert_eq!(rig.memory.set_keys(), vec![String::from(CACHE_KEY)]);
    assert_eq!(rig.memory.set_ttls(), vec![60.0_f64]);
    assert!(rig.memory.pipeline_keys().is_empty());
    assert!(rig.remote.batch_keys().is_empty());
    let scanned = vec![
        format!("cache_key_name: {CACHE_NAME}"),
        String::from("redis keys: []"),
    ];
    assert_eq!(retrieved.printed, scanned);
    assert_eq!(
        retrieved.diagnostics,
        scanned
            .iter()
            .map(|line| debug_line(line))
            .collect::<Vec<_>>()
    );
    assert_eq!(retrieved.data, namespaced_data());
}

// no cache key from the request and none from the cache means there is nothing to read
#[tokio::test]
async fn retrieve_without_cache_key() {
    let rig = Rig::new();
    let input = retrieve_input(None, None);
    let BatchPrecallResult::Retrieve(retrieved) = rig.run(&input).await.unwrap() else {
        panic!("a retrieve answers with the retrieve result");
    };
    assert_eq!(retrieved.cached, None);
    assert!(rig.remote.hits().is_empty());
    assert_eq!(retrieved.data, namespaced_data());
}

// a caller without an api_key is named None in the cache namespace, like the source's f-string
#[tokio::test]
async fn retrieve_names_the_namespace_without_api_key() {
    let rig = Rig::new();
    let input = BatchPrecallInput {
        call: BatchPrecallCall::Retrieve(BatchRetrieveCall {
            call_type: String::from(RETRIEVE_CALL_TYPE),
            principal: BatchLimitPrincipal(BTreeMap::new()),
            data: retrieve_data(),
            cache_key: None,
            cache_control: None,
        }),
    };
    let BatchPrecallResult::Retrieve(retrieved) = rig.run(&input).await.unwrap() else {
        panic!("a retrieve answers with the retrieve result");
    };
    assert_eq!(retrieved.cached, None);
    let mut expected = retrieve_data();
    expected.insert(
        String::from("metadata"),
        namespaced_as("litellm:None:aretrieve_batch"),
    );
    assert_eq!(retrieved.data, expected);
}

// without litellm.cache the hook only sets the namespace
#[tokio::test]
async fn retrieve_without_cache() {
    let rig = Rig::new();
    let input = retrieve_input(Some(CACHE_KEY), None);
    let result = rig.run_without_remote(&input).await.unwrap();
    let BatchPrecallResult::Retrieve(retrieved) = result else {
        panic!("a retrieve answers with the retrieve result");
    };
    assert_eq!(retrieved.cached, None);
    assert!(retrieved.diagnostics.is_empty());
    assert_eq!(retrieved.data, namespaced_data());
}

// a cache that is not Redis is not scanned, and the namespace is still set
#[tokio::test]
async fn retrieve_without_redis() {
    let mut rig = Rig::new();
    rig.remote.redis = false;
    let input = retrieve_input(Some(CACHE_KEY), None);
    let BatchPrecallResult::Retrieve(retrieved) = rig.run(&input).await.unwrap() else {
        panic!("a retrieve answers with the retrieve result");
    };
    assert_eq!(retrieved.cached, None);
    assert!(retrieved.diagnostics.is_empty());
    assert!(rig.remote.scan_patterns().is_empty());
    assert_eq!(retrieved.data, namespaced_data());
}

// metadata is set after the warm-up, and a request without a metadata object is a logged failure, not a hard error
#[rstest]
#[case::missing(None, "the request metadata is missing")]
#[case::not_an_object(
    Some(Json::Text(String::from("batch-1"))),
    "the request metadata is not an object"
)]
#[tokio::test]
async fn retrieve_reports_bad_metadata(#[case] metadata: Option<Json>, #[case] reason: &str) {
    let rig = Rig::new();
    let input = retrieve_input_with_metadata(metadata);
    let BatchPrecallResult::Retrieve(retrieved) = rig.run(&input).await.unwrap() else {
        panic!("a retrieve answers with the retrieve result");
    };
    assert_eq!(retrieved.cached, None);
    assert_eq!(
        retrieved.diagnostics,
        vec![
            debug_line(&format!("cache_key_name: {CACHE_NAME}")),
            debug_line("redis keys: []"),
            error_line(format!("{CACHE_FAILURE_PREFIX}{reason}")),
        ]
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheFailure {
    ScanLogged,
    ScanRaised,
    MemoryRead,
    RemoteRead,
    WarmWrite,
    ReadWarm,
}

fn failing_rig(failure: CacheFailure) -> (Rig, BatchPrecallInput) {
    let mut rig = Rig::new();
    let input = match failure {
        CacheFailure::ScanLogged => {
            rig.remote.scan = RemoteAnswer::Failed(String::from("redis is down"));
            retrieve_input(Some(CACHE_KEY), None)
        }
        CacheFailure::ScanRaised => {
            rig.remote.scan = RemoteAnswer::Http {
                status: 400,
                detail: Json::Text(String::from("the scan was rejected")),
            };
            // a readable value, so the cached answer stays empty only because the raise skips the read
            rig.remote.get = RemoteAnswer::Value(Some(envelope(None, batch_response())));
            retrieve_input(Some(CACHE_KEY), None)
        }
        CacheFailure::MemoryRead => {
            rig.memory
                .store(CACHE_KEY, envelope(None, batch_response()));
            rig.memory.get_failure = Some(String::from("memory read failed"));
            retrieve_input(Some(CACHE_KEY), None)
        }
        CacheFailure::RemoteRead => {
            rig.remote.get = RemoteAnswer::Failed(String::from("redis read failed"));
            retrieve_input(Some(CACHE_KEY), None)
        }
        CacheFailure::WarmWrite => {
            rig.remote.scan = RemoteAnswer::Value(vec![String::from(CACHE_KEY)]);
            rig.remote.batch_get = RemoteAnswer::Value(vec![(
                String::from(CACHE_KEY),
                envelope(None, batch_response()),
            )]);
            rig.memory.set_many_failure = Some(String::from("memory write failed"));
            retrieve_input(Some(CACHE_KEY), None)
        }
        CacheFailure::ReadWarm => {
            rig.remote.key = Some(String::from(CACHE_KEY));
            rig.remote.get = RemoteAnswer::Value(Some(envelope(None, batch_response())));
            rig.memory.set_failure = Some(String::from("memory write failed"));
            retrieve_input(None, None)
        }
    };
    (rig, input)
}

fn expected_failure_diagnostics(failure: CacheFailure) -> Vec<BatchDiagnostic> {
    match failure {
        CacheFailure::ScanLogged => vec![
            debug_line(&format!("cache_key_name: {CACHE_NAME}")),
            error_line(format!("{CACHE_FAILURE_PREFIX}redis is down")),
        ],
        CacheFailure::ScanRaised => vec![debug_line(&format!("cache_key_name: {CACHE_NAME}"))],
        CacheFailure::MemoryRead => Vec::new(),
        CacheFailure::RemoteRead | CacheFailure::ReadWarm => vec![
            debug_line(&format!("cache_key_name: {CACHE_NAME}")),
            debug_line("redis keys: []"),
        ],
        CacheFailure::WarmWrite => vec![
            debug_line(&format!("cache_key_name: {CACHE_NAME}")),
            debug_line(&format!("redis keys: ['{CACHE_KEY}']")),
            error_line(format!("{CACHE_FAILURE_PREFIX}memory write failed")),
        ],
    }
}

fn expected_raise(failure: CacheFailure) -> Option<BatchHttpError> {
    match failure {
        CacheFailure::ScanRaised => Some(BatchHttpError {
            status: 400,
            detail: Json::Text(String::from("the scan was rejected")),
        }),
        CacheFailure::ScanLogged
        | CacheFailure::MemoryRead
        | CacheFailure::RemoteRead
        | CacheFailure::WarmWrite
        | CacheFailure::ReadWarm => None,
    }
}

fn expected_failure_data(failure: CacheFailure) -> RequestData {
    match failure {
        CacheFailure::MemoryRead | CacheFailure::RemoteRead | CacheFailure::ReadWarm => {
            namespaced_data()
        }
        CacheFailure::ScanLogged | CacheFailure::ScanRaised | CacheFailure::WarmWrite => {
            retrieve_data()
        }
    }
}

// the async cache hooks never block a retrieve: every failure but an HTTPException becomes a log line, and the namespace is set only after a clean warm-up
#[rstest]
#[case::scan_logged(CacheFailure::ScanLogged)]
#[case::scan_raised(CacheFailure::ScanRaised)]
#[case::memory_read(CacheFailure::MemoryRead)]
#[case::remote_read(CacheFailure::RemoteRead)]
#[case::warm_write(CacheFailure::WarmWrite)]
#[case::read_warm(CacheFailure::ReadWarm)]
#[tokio::test]
async fn redis_cache_error_policy(#[case] failure: CacheFailure) {
    let (rig, input) = failing_rig(failure);
    let BatchPrecallResult::Retrieve(retrieved) = rig.run(&input).await.unwrap() else {
        panic!("a retrieve answers with the retrieve result");
    };
    assert_eq!(retrieved.cached, None);
    assert_eq!(retrieved.raised, expected_raise(failure));
    assert_eq!(retrieved.diagnostics, expected_failure_diagnostics(failure));
    assert_eq!(retrieved.data, expected_failure_data(failure));
    assert!(retrieved.printed.is_empty());
}

async fn terminal_result(rig: &Rig, status: &str) -> BatchTerminalResult {
    let input = terminal_input("batch-1", status);
    let result = rig.run(&input).await.unwrap();
    let BatchPrecallResult::TerminalRetrieve(outcome) = result else {
        panic!("a terminal retrieve answers with the terminal result");
    };
    outcome
}

// _handle_batch_enqueued_post_call: save a pending reservation under the batch id, then refund it once the status is terminal
#[tokio::test]
async fn terminal_retrieve_refund() {
    let mut rig = Rig::new();
    seed_counter(&rig.counters, 500);
    rig.stash = ScriptedStash::holding(reservation(300), Vec::new());
    let saved = terminal_result(&rig, "in_progress").await;
    assert_eq!(saved.batch_id, "batch-1");
    assert!(saved.saved);
    assert!(!saved.refunded);
    assert!(saved.diagnostics.is_empty());
    assert_eq!(stored_counter(&rig.counters), Some(500));
    assert!(stored_record(&rig.counters, "batch-1").is_some());
    assert_eq!(rig.stash.reservation(), None);
    let refunded = terminal_result(&rig, "completed").await;
    assert_eq!(refunded.batch_id, "batch-1");
    assert!(!refunded.saved);
    assert!(refunded.refunded);
    assert_eq!(stored_counter(&rig.counters), Some(200));
    assert!(stored_record(&rig.counters, "batch-1").is_none());
    let again = terminal_result(&rig, "completed").await;
    assert!(!again.refunded);
    assert_eq!(stored_counter(&rig.counters), Some(200));
}

// a terminal status on the first retrieve saves and refunds in one call, and the status is read case-insensitively
#[tokio::test]
async fn terminal_retrieve_refunds_in_one_call() {
    let mut rig = Rig::new();
    seed_counter(&rig.counters, 500);
    rig.stash = ScriptedStash::holding(reservation(300), Vec::new());
    let outcome = terminal_result(&rig, "FAILED").await;
    assert_eq!(outcome.batch_id, "batch-1");
    assert!(outcome.saved);
    assert!(outcome.refunded);
    assert_eq!(stored_counter(&rig.counters), Some(200));
    assert!(stored_record(&rig.counters, "batch-1").is_none());
}

#[derive(Debug, thiserror::Error)]
#[error("the refund store is down")]
struct RefundStoreError;

struct FailingRefunds;

impl BatchRefundSink for FailingRefunds {
    type Error = RefundStoreError;

    async fn handle(
        &self,
        _input: &BatchEnqueuedRefundInput,
    ) -> Result<BatchEnqueuedRefundResult, RefundStoreError> {
        Err(RefundStoreError)
    }
}

// the post-call hook never fails the response: a store error is one error line and an outcome that saved and refunded nothing
#[tokio::test]
async fn terminal_retrieve_reports_store_error() {
    let mut rig = Rig::new();
    rig.stash = ScriptedStash::holding(reservation(300), Vec::new());
    let refunds = FailingRefunds;
    let service = BatchPrecall {
        stages: &rig.stages,
        stash: &rig.stash,
        refunds: &refunds,
        applier: &rig.applier,
        memory: &rig.memory,
        remote: Some(&rig.remote),
        verbose_stdout: rig.verbose_stdout,
        clock: Arc::clone(&rig.clock),
        limits: rig.limits,
    };
    let result = service
        .run(&terminal_input("batch-1", "completed"))
        .await
        .unwrap();
    let BatchPrecallResult::TerminalRetrieve(outcome) = result else {
        panic!("a terminal retrieve answers with the terminal result");
    };
    assert_eq!(outcome.batch_id, "batch-1");
    assert!(!outcome.saved);
    assert!(!outcome.refunded);
    assert_eq!(
        outcome.diagnostics,
        vec![error_line(String::from(
            "Error in batch enqueued-token post-call hook: the refund store is down"
        ))]
    );
    assert_eq!(rig.stash.reservation(), None);
}
