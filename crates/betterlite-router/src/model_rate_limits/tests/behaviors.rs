use std::sync::Arc;

use crate::model_rate_limits::error::RateLimitChannel;
use crate::model_rate_limits::service::SuccessUsage;

use super::support::{
    FakeCache, FakeIo, HookCalls, IoLimitsSeen, RemoteMode, exceeded, request, rpm_key,
    run_case_table, service, spec, tpm_key, with_info, with_param,
};

pub(super) fn run(name: &str) {
    run_case_table(name, &CASES);
}

const CASES: [(&str, fn()); 17] = [
    ("limits_disabled_allows", || {
        super::runtime().block_on(limits_disabled_allows());
    }),
    ("tpm_below_limit_allows", || {
        super::runtime().block_on(tpm_below_limit_allows());
    }),
    ("tpm_at_limit_rejects", || {
        super::runtime().block_on(tpm_at_limit_rejects());
    }),
    ("tpm_remote_higher_value_rejects", || {
        super::runtime().block_on(tpm_remote_higher_value_rejects());
    }),
    ("tpm_local_at_limit_skips_remote", || {
        super::runtime().block_on(tpm_local_at_limit_skips_remote());
    }),
    ("breaker_open_falls_back_to_local", || {
        super::runtime().block_on(breaker_open_falls_back_to_local());
    }),
    ("remote_failure_allows_without_rpm", || {
        super::runtime().block_on(remote_failure_allows_without_rpm());
    }),
    ("rpm_increments_first_and_rejects_over_limit", || {
        super::runtime().block_on(rpm_increments_first_and_rejects_over_limit());
    }),
    ("success_event_counts_tpm_usage", || {
        super::runtime().block_on(success_event_counts_tpm_usage());
    }),
    ("success_event_skips_absent_fields", || {
        super::runtime().block_on(success_event_skips_absent_fields());
    }),
    ("failure_event_refunds", || {
        super::runtime().block_on(failure_event_refunds());
    }),
    ("io_reservation_before_the_checks", || {
        super::runtime().block_on(io_reservation_before_the_checks());
    }),
    ("coexist_warning_logs_once", || {
        super::runtime().block_on(coexist_warning_logs_once());
    }),
    ("sync_facade_counts_local_only", || {
        super::runtime().block_on(sync_facade_counts_local_only());
    }),
    ("sync_success_event_counts_local_only", || {
        super::runtime().block_on(sync_success_event_counts_local_only());
    }),
    ("sync_failure_event_refunds", || {
        super::runtime().block_on(sync_failure_event_refunds());
    }),
    ("rpm_rejection_carries_error_fields", || {
        super::runtime().block_on(rpm_rejection_carries_error_fields());
    }),
];

async fn limits_disabled_allows() {
    let cache = Arc::new(FakeCache::new());
    let check = service(Arc::clone(&cache), None);
    let deployment = spec("gpt-4o", "a", "openai/gpt-4o");
    check
        .async_pre_call_check(&deployment, &mut request())
        .await
        .unwrap();
    assert!(cache.local.lock().unwrap().is_empty());
}

async fn tpm_below_limit_allows() {
    let cache = Arc::new(FakeCache::new().seed_local(&tpm_key("openai/gpt-4o"), 4));
    let check = service(Arc::clone(&cache), None);
    let deployment = with_param(spec("gpt-4o", "a", "openai/gpt-4o"), "tpm", 5);
    check
        .async_pre_call_check(&deployment, &mut request())
        .await
        .unwrap();
    assert_eq!(
        cache.remote_reads.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

async fn tpm_at_limit_rejects() {
    let cache = Arc::new(
        FakeCache::new()
            .seed_local(&tpm_key("openai/gpt-4o"), 5)
            .seed_remote(&tpm_key("openai/gpt-4o"), 100),
    );
    let check = service(Arc::clone(&cache), None);
    let deployment = with_param(spec("gpt-4o", "a", "openai/gpt-4o"), "tpm", 5);
    let error = check
        .async_pre_call_check(&deployment, &mut request())
        .await
        .unwrap_err();
    assert_eq!(exceeded(error).channel, RateLimitChannel::Tpm);
    assert_eq!(
        cache.remote_reads.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

async fn tpm_remote_higher_value_rejects() {
    let cache = Arc::new(
        FakeCache::new()
            .seed_local(&tpm_key("openai/gpt-4o"), 1)
            .seed_remote(&tpm_key("openai/gpt-4o"), 5),
    );
    let check = service(Arc::clone(&cache), None);
    let deployment = with_param(spec("gpt-4o", "a", "openai/gpt-4o"), "tpm", 5);
    let error = check
        .async_pre_call_check(&deployment, &mut request())
        .await
        .unwrap_err();
    assert_eq!(exceeded(error).current, 5);
}

async fn tpm_local_at_limit_skips_remote() {
    let cache = Arc::new(FakeCache::new().seed_local(&tpm_key("openai/gpt-4o"), 5));
    let check = service(Arc::clone(&cache), None);
    let deployment = with_param(spec("gpt-4o", "a", "openai/gpt-4o"), "tpm", 5);
    check
        .async_pre_call_check(&deployment, &mut request())
        .await
        .unwrap_err();
    assert_eq!(
        cache.remote_reads.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

async fn breaker_open_falls_back_to_local() {
    let cache = Arc::new(
        FakeCache::new()
            .mode(RemoteMode::Breaker)
            .seed_local(&tpm_key("openai/gpt-4o"), 1),
    );
    let check = service(Arc::clone(&cache), None);
    let deployment = with_param(spec("gpt-4o", "a", "openai/gpt-4o"), "tpm", 5);
    check
        .async_pre_call_check(&deployment, &mut request())
        .await
        .unwrap();
    assert_eq!(
        cache.remote_reads.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

async fn remote_failure_allows_without_rpm() {
    let cache = Arc::new(
        FakeCache::new()
            .mode(RemoteMode::Down)
            .seed_local(&tpm_key("openai/gpt-4o"), 1),
    );
    let check = service(Arc::clone(&cache), None);
    let deployment = with_param(
        with_param(spec("gpt-4o", "a", "openai/gpt-4o"), "tpm", 5),
        "rpm",
        1,
    );
    check
        .async_pre_call_check(&deployment, &mut request())
        .await
        .unwrap();
    assert_eq!(cache.local_value(&rpm_key("openai/gpt-4o")), None);
}

async fn rpm_increments_first_and_rejects_over_limit() {
    let cache = Arc::new(FakeCache::new());
    let check = service(Arc::clone(&cache), None);
    let deployment = with_param(spec("gpt-4o", "a", "openai/gpt-4o"), "rpm", 2);
    for _attempt in 0..2 {
        check
            .async_pre_call_check(&deployment, &mut request())
            .await
            .unwrap();
    }
    let error = check
        .async_pre_call_check(&deployment, &mut request())
        .await
        .unwrap_err();
    assert_eq!(exceeded(error).current, 3);
    assert_eq!(cache.local_value(&rpm_key("openai/gpt-4o")), Some(3));
    assert_eq!(cache.remote_value(&rpm_key("openai/gpt-4o")), Some(3));
}

async fn success_event_counts_tpm_usage() {
    let cache = Arc::new(FakeCache::new());
    let check = service(Arc::clone(&cache), None);
    check
        .async_log_success_event(&SuccessUsage {
            model_id: Some("a"),
            model: Some("openai/gpt-4o"),
            total_tokens: 120,
        })
        .await;
    assert_eq!(cache.local_value(&tpm_key("openai/gpt-4o")), Some(120));
    assert_eq!(cache.remote_value(&tpm_key("openai/gpt-4o")), Some(120));
}

async fn success_event_skips_absent_fields() {
    let cache = Arc::new(FakeCache::new());
    let check = service(Arc::clone(&cache), None);
    for usage in [
        SuccessUsage {
            model_id: None,
            model: Some("openai/gpt-4o"),
            total_tokens: 10,
        },
        SuccessUsage {
            model_id: Some("a"),
            model: None,
            total_tokens: 10,
        },
        SuccessUsage {
            model_id: Some("a"),
            model: Some(""),
            total_tokens: 10,
        },
        SuccessUsage {
            model_id: Some("a"),
            model: Some("openai/gpt-4o"),
            total_tokens: 0,
        },
    ] {
        check.async_log_success_event(&usage).await;
    }
    assert!(cache.local.lock().unwrap().is_empty());
}

async fn failure_event_refunds() {
    let io = Arc::new(FakeIo::new());
    let cache = Arc::new(FakeCache::new());
    let check = service(Arc::clone(&cache), Some(Arc::clone(&io)));
    check.async_log_failure_event(&mut request()).await;
    assert_eq!(io.calls.lock().unwrap().refunds, 1);
}

async fn io_reservation_before_the_checks() {
    let io = Arc::new(FakeIo::new());
    let cache = Arc::new(
        FakeCache::new()
            .seed_local(&rpm_key("openai/gpt-4o"), 1)
            .seed_remote(&rpm_key("openai/gpt-4o"), 1),
    );
    let check = service(Arc::clone(&cache), Some(Arc::clone(&io)));
    let deployment = with_param(
        with_info(
            spec("gpt-4o", "a", "openai/gpt-4o"),
            "itpm",
            betterlite_models::decisions::Json::Number(10.into()),
        ),
        "rpm",
        1,
    );
    let mut body = request();
    let error = check
        .async_pre_call_check(&deployment, &mut body)
        .await
        .unwrap_err();
    assert_eq!(exceeded(error).channel, RateLimitChannel::Rpm);
    assert_eq!(
        io.calls.lock().unwrap().reserved,
        vec![IoLimitsSeen {
            itpm: Some(10),
            otpm: None
        }]
    );
    assert_eq!(io.calls.lock().unwrap().refunds, 1);
    assert_eq!(
        body.get("reserved"),
        Some(&betterlite_models::decisions::Json::Bool(true))
    );
}

async fn coexist_warning_logs_once() {
    let check = service(Arc::new(FakeCache::new()), None);
    let deployment = with_param(
        with_info(
            spec("gpt-4o", "a", "openai/gpt-4o"),
            "itpm",
            betterlite_models::decisions::Json::Number(10.into()),
        ),
        "tpm",
        100,
    );
    assert!(check.logs_coexist_warning(&deployment).unwrap());
    assert!(!check.logs_coexist_warning(&deployment).unwrap());
}

async fn sync_facade_counts_local_only() {
    let cache = Arc::new(FakeCache::new());
    let check = service(Arc::clone(&cache), None);
    let deployment = with_param(spec("gpt-4o", "a", "openai/gpt-4o"), "rpm", 1);
    check.pre_call_check(&deployment).unwrap();
    let error = check.pre_call_check(&deployment).unwrap_err();
    assert_eq!(exceeded(error).current, 2);
    assert_eq!(cache.local_value(&rpm_key("openai/gpt-4o")), Some(2));
    assert_eq!(cache.remote_value(&rpm_key("openai/gpt-4o")), None);
}

async fn sync_success_event_counts_local_only() {
    let cache = Arc::new(FakeCache::new());
    let check = service(Arc::clone(&cache), None);
    check.log_success_event(&SuccessUsage {
        model_id: Some("a"),
        model: Some("openai/gpt-4o"),
        total_tokens: 7,
    });
    assert_eq!(cache.local_value(&tpm_key("openai/gpt-4o")), Some(7));
    assert_eq!(cache.remote_value(&tpm_key("openai/gpt-4o")), None);
}

async fn sync_failure_event_refunds() {
    let io = Arc::new(FakeIo::new());
    let check = service(Arc::new(FakeCache::new()), Some(io.clone()));
    check.log_failure_event(&mut request());
    assert_eq!(
        io.calls.lock().unwrap(),
        HookCalls {
            reserved: vec![],
            refunds: 1
        }
    );
}

async fn rpm_rejection_carries_error_fields() {
    let cache = Arc::new(FakeCache::new().seed_local(&rpm_key("openai/gpt-4o"), 1));
    let check = service(Arc::clone(&cache), None);
    let deployment = with_param(spec("gpt-4o", "a", "openai/gpt-4o"), "rpm", 1);
    let error = check
        .async_pre_call_check(&deployment, &mut request())
        .await
        .unwrap_err();
    let exceeded = exceeded(error);
    assert_eq!(exceeded.channel, RateLimitChannel::Rpm);
    assert_eq!(exceeded.limit, 1);
    assert_eq!(exceeded.current, 2);
    assert_eq!(exceeded.model_id, "a");
    assert_eq!(exceeded.model, "openai/gpt-4o");
    assert_eq!(exceeded.model_group, "gpt-4o");
    assert_eq!(exceeded.num_retries, Some(0));
    assert_eq!(
        exceeded.to_string(),
        "Model rate limit exceeded. RPM limit=1, current usage=2"
    );
    assert_eq!(
        exceeded.content(),
        "Deployment over user-defined ratelimit. rpm limit=1. current usage=2. id=a, model_group=gpt-4o"
    );
}
