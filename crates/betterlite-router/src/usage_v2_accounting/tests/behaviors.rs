use rust_decimal::Decimal;

use super::support::{
    Harness, MINUTE_MILLIS, MINUTE_MILLIS_END, NEXT_MINUTE_RPM_KEY, RPM_KEY, TPM_KEY, at,
    deployment, payload, rpm, success_event,
};
use crate::router_observations::usage::DeploymentUsage;
use crate::usage_v2_accounting::success::{HiddenParams, StandardLoggingObject, SuccessEvent};

pub(super) async fn run(name: &str) {
    match name {
        "rpm_allows_exactly_the_configured_requests" => {
            rpm_allows_exactly_the_configured_requests().await;
        }
        "rpm_zero_limit_counts_the_first_request" => {
            rpm_zero_limit_counts_the_first_request().await;
        }
        "rpm_without_a_limit_never_rejects" => rpm_without_a_limit_never_rejects().await,
        "rpm_limit_prefers_the_deployment_value" => rpm_limit_prefers_the_deployment_value().await,
        "rpm_limit_falls_back_in_order" => rpm_limit_falls_back_in_order().await,
        "rpm_window_follows_the_minute" => rpm_window_follows_the_minute().await,
        "pre_call_keeps_counting_when_the_store_is_down" => {
            pre_call_keeps_counting_when_the_store_is_down().await;
        }
        "success_counts_each_callback_once" => success_counts_each_callback_once().await,
        "success_skips_batch_retrieves" => success_skips_batch_retrieves().await,
        "success_skips_an_incomplete_payload" => success_skips_an_incomplete_payload().await,
        "success_sync_and_async_share_the_counter" => {
            success_sync_and_async_share_the_counter().await;
        }
        "success_storage_error_is_swallowed" => success_storage_error_is_swallowed().await,
        other => panic!("unknown case {other}"),
    }
}

async fn rpm_allows_exactly_the_configured_requests() {
    let harness = Harness::new();
    let facts = deployment("dep-a", rpm(Some(2)));
    assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
    assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
    let error = harness
        .accounting
        .pre_call(&facts, at(0))
        .await
        .expect_err("the third request is past the limit");
    assert_eq!(error.limit, 2);
    assert_eq!(error.usage, Decimal::from(2_i64));
    assert_eq!(harness.local(RPM_KEY), Some(2));
}

async fn rpm_zero_limit_counts_the_first_request() {
    let harness = Harness::new();
    let facts = deployment("dep-a", rpm(Some(0)));
    let over = harness
        .accounting
        .pre_call(&facts, at(0))
        .await
        .expect_err("an rpm of zero admits no request");
    assert_eq!(over.usage, Decimal::ONE);
    assert_eq!(harness.local(RPM_KEY), Some(1));
    let at_limit = harness
        .accounting
        .pre_call(&facts, at(0))
        .await
        .expect_err("the local count is already at the limit");
    assert_eq!(at_limit.usage, Decimal::ONE);
    assert_eq!(at_limit.limit, 0);
    assert_eq!(harness.local(RPM_KEY), Some(1));
}

async fn rpm_without_a_limit_never_rejects() {
    let harness = Harness::new();
    let facts = deployment("dep-a", DeploymentUsage::default());
    for _call in 0_u8..3_u8 {
        assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
    }
    assert_eq!(harness.local(RPM_KEY), Some(3));
}

async fn rpm_limit_prefers_the_deployment_value() {
    let harness = Harness::new();
    let limits = DeploymentUsage {
        tpm: None,
        rpm: Some(1),
        params_tpm: None,
        params_rpm: Some(5),
        model_info_tpm: None,
        model_info_rpm: Some(9),
        io_token_limits: false,
    };
    let facts = deployment("dep-a", limits);
    assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
    let error = harness
        .accounting
        .pre_call(&facts, at(0))
        .await
        .expect_err("the deployment rpm wins over the other levels");
    assert_eq!(error.limit, 1);
}

async fn rpm_limit_falls_back_in_order() {
    let harness = Harness::new();
    let levels: [(&'static str, Option<i64>, Option<i64>); 2] =
        [("dep-a", Some(1), Some(9)), ("dep-b", None, Some(1))];
    for (model_id, params_rpm, model_info_rpm) in levels {
        let facts = deployment(
            model_id,
            DeploymentUsage {
                tpm: None,
                rpm: None,
                params_tpm: None,
                params_rpm,
                model_info_tpm: None,
                model_info_rpm,
                io_token_limits: false,
            },
        );
        assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
        let error = harness
            .accounting
            .pre_call(&facts, at(0))
            .await
            .expect_err("the next configured rpm level is the one in force");
        assert_eq!(error.limit, 1);
    }
}

async fn rpm_window_follows_the_minute() {
    let harness = Harness::new();
    let facts = deployment("dep-a", DeploymentUsage::default());
    assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
    assert_eq!(
        harness
            .accounting
            .pre_call(&facts, at(MINUTE_MILLIS_END))
            .await,
        Ok(())
    );
    assert_eq!(
        harness.accounting.pre_call(&facts, at(MINUTE_MILLIS)).await,
        Ok(())
    );
    assert_eq!(harness.local(RPM_KEY), Some(2));
    assert_eq!(harness.local(NEXT_MINUTE_RPM_KEY), Some(1));
}

async fn pre_call_keeps_counting_when_the_store_is_down() {
    let harness = Harness::new();
    let facts = deployment("dep-a", rpm(Some(2)));
    harness.store.set_down(true);
    assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
    assert!(!harness.accounting.counters.sync_with_redis().await);
    assert_eq!(harness.store.value(RPM_KEY), Decimal::ZERO);
    assert_eq!(harness.local(RPM_KEY), Some(1));
    assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
    harness.store.set_down(false);
    assert!(harness.accounting.counters.sync_with_redis().await);
    assert_eq!(harness.store.value(RPM_KEY), Decimal::ONE);
}

async fn success_counts_each_callback_once() {
    let harness = Harness::new();
    let event = success_event(payload(
        Some("gpt"),
        Some("dep-a"),
        Some("openai/gpt-4o-mini"),
        7,
    ));
    assert_eq!(
        harness
            .accounting
            .log_success(&event, at(0))
            .await
            .as_deref(),
        Some(TPM_KEY)
    );
    assert_eq!(
        harness
            .accounting
            .log_success(&event, at(0))
            .await
            .as_deref(),
        Some(TPM_KEY)
    );
    assert_eq!(harness.local(TPM_KEY), Some(14));
}

async fn success_skips_batch_retrieves() {
    let harness = Harness::new();
    let mut event = success_event(payload(
        Some("gpt"),
        Some("dep-a"),
        Some("openai/gpt-4o-mini"),
        7,
    ));
    for call_type in ["retrieve_batch", "aretrieve_batch"] {
        event.call_type = Some(call_type);
        assert_eq!(harness.accounting.log_success(&event, at(0)).await, None);
        assert_eq!(harness.accounting.log_success_now(&event, at(0)), None);
    }
    assert_eq!(harness.local(TPM_KEY), None);
}

async fn success_skips_an_incomplete_payload() {
    let harness = Harness::new();
    let without_payload = SuccessEvent {
        call_type: Some("acompletion"),
        standard_logging_object: None,
    };
    assert_eq!(
        harness
            .accounting
            .log_success(&without_payload, at(0))
            .await,
        None
    );
    for event in [
        success_event(payload(None, Some("dep-a"), Some("openai/gpt-4o-mini"), 7)),
        success_event(payload(Some("gpt"), None, Some("openai/gpt-4o-mini"), 7)),
        success_event(payload(Some("gpt"), Some("dep-a"), None, 7)),
    ] {
        assert_eq!(harness.accounting.log_success(&event, at(0)).await, None);
        assert_eq!(harness.accounting.log_success_now(&event, at(0)), None);
    }
    assert_eq!(harness.local(TPM_KEY), None);
}

async fn success_sync_and_async_share_the_counter() {
    let harness = Harness::new();
    let sync_event = success_event(payload(
        Some("gpt"),
        Some("dep-a"),
        Some("openai/gpt-4o-mini"),
        7,
    ));
    assert_eq!(
        harness
            .accounting
            .log_success_now(&sync_event, at(0))
            .as_deref(),
        Some(TPM_KEY)
    );
    let async_event = success_event(payload(
        Some("gpt"),
        Some("dep-a"),
        Some("openai/gpt-4o-mini"),
        3,
    ));
    let key = harness.accounting.log_success(&async_event, at(0)).await;
    assert_eq!(key.as_deref(), Some(TPM_KEY));
    assert_eq!(harness.local(TPM_KEY), Some(10));
    assert_eq!(harness.store.value(TPM_KEY), Decimal::ZERO);
    assert!(harness.accounting.counters.sync_with_redis().await);
    assert_eq!(harness.store.value(TPM_KEY), Decimal::from(10_i64));
}

async fn success_storage_error_is_swallowed() {
    let harness = Harness::new();
    let long_model = "m".repeat(1_100);
    let event = SuccessEvent {
        call_type: Some("acompletion"),
        standard_logging_object: Some(StandardLoggingObject {
            model_group: Some("gpt"),
            hidden_params: HiddenParams {
                litellm_model_name: Some(&long_model),
            },
            model_id: Some("dep-a"),
            total_tokens: 7,
        }),
    };
    assert_eq!(harness.accounting.log_success(&event, at(0)).await, None);
    assert!(harness.accounting.counters.get_tracked_keys().is_empty());
}
