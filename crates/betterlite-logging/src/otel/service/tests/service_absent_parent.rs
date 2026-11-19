use super::fixtures::{database_env, payload, service_call};
use super::support::Rig;
use crate::otel::model::payloads::service::ServiceType;
use crate::otel::service::hooks::{async_service_failure_hook, async_service_success_hook};

#[rstest::rstest]
#[case::success(false)]
#[case::failure(true)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(#[case] failure: bool) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let payload = payload(ServiceType::Postgres, "query", None, None);
    let call = service_call(&payload, None, None);
    if failure {
        async_service_failure_hook(&lifecycle, &call, Some("boom"), &database_env());
    } else {
        async_service_success_hook(&lifecycle, &call, &database_env());
    }
    assert!(rig.spans().is_empty(), "no parent, no service span");
}
