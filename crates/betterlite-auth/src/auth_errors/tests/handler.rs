use std::time::Duration;

use betterlite_db::database_faults::ErrorType;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_models::decisions::Json;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use super::support::{
    CLIENT_IP, Event, Fixture, OUTAGE_ALLOWED, RESPONSE_ROUTE, Recorder, Reply, budget, database,
    http, identity_with, object, rejected, seeded, settings,
};
use crate::auth_errors::conversion::AuthFailure;
use crate::auth_errors::handler::{DB_UNAVAILABLE_FALLBACK_USER_ID, HandlerError};
use crate::auth_errors::ports::{AuthLog, HookError, Level, Logger};
use crate::resolved_key::auth::{ApiKeySecret, user_api_key_has_admin_view};
use crate::resolved_key::credential::safe_hash_api_key;
use crate::user_roles::UserRole;

#[tokio::test]
async fn outage_connection_only_service_hook_then_warning_then_restricted_identity() {
    let fixture = Fixture::new();
    let ports = Recorder::replying(Reply::Original);
    let outage = database(ErrorType::Connect, "refused");
    let identity = fixture
        .handle(&ports, OUTAGE_ALLOWED, outage)
        .await
        .unwrap();
    let service = Event::Service {
        call_type: "get_key_object",
        duration: Duration::ZERO,
    };
    let events = ports.events();
    assert_eq!((events.len(), events.first()), (2, Some(&service)));
    assert!(matches!(
        events.get(1),
        Some(Event::Log(AuthLog {
            logger: Logger::Proxy,
            level: Level::Warning,
            traceback: false,
            ..
        }))
    ));
    let token = identity
        .view
        .token
        .token
        .as_ref()
        .map(|value| value.expose_secret().to_owned());
    let fields = (
        identity.view.token.user_id.as_deref(),
        identity.view.token.key_name.as_deref(),
        token.as_deref(),
        identity.extra.request_route.as_deref(),
    );
    let failed = Some("failed-to-connect-to-db");
    let expected = (
        Some(DB_UNAVAILABLE_FALLBACK_USER_ID),
        failed,
        failed,
        Some(RESPONSE_ROUTE),
    );
    assert_eq!(fields, expected);
    assert_eq!(identity.extra.user_role, Some(UserRole::InternalUser));
    assert!(!user_api_key_has_admin_view(&identity));
}

#[tokio::test]
async fn outage_bypass_requires_a_connection_error() {
    let fixture = Fixture::new();
    let ports = Recorder::replying(Reply::Original);
    let data_error = database(ErrorType::UniqueViolation, "dup");
    let outcome = fixture.handle(&ports, OUTAGE_ALLOWED, data_error).await;
    assert_eq!(rejected(outcome).failure.exception().code, "401");
    let services = ports
        .events()
        .into_iter()
        .filter(|event| matches!(event, Event::Service { .. }));
    assert_eq!(services.count(), 0);
    assert_eq!(ports.hook_calls().len(), 1);
}

#[tokio::test]
async fn outage_disabled_no_bypass() {
    let fixture = Fixture::new();
    let ports = Recorder::replying(Reply::Original);
    let outcome = fixture
        .handle(
            &ports,
            settings(false),
            database(ErrorType::Connect, "refused"),
        )
        .await;
    assert_eq!(rejected(outcome).failure.exception().code, "503");
    let events = ports.events();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::Service { .. }))
    );
    assert_eq!(ports.hook_calls().len(), 1);
    assert_eq!(
        ports.logs().first().map(|entry| entry.level),
        Some(Level::Error)
    );
}

#[tokio::test]
async fn missing_scope_headers_no_user_agent() {
    let fixture = Fixture::new();
    let without = Recorder::replying(Reply::Original);
    let mut request = fixture.request();
    request.scope_headers = None;
    let _outcome = fixture
        .handle_request(&without, settings(false), http(401, false), request)
        .await;
    let with = Recorder::replying(Reply::Original);
    let _stamped = fixture
        .handle(&with, settings(false), http(401, false))
        .await;
    let metadata = |ports: &Recorder| {
        ports
            .hook_calls()
            .first()
            .and_then(|(data, ..)| data.get("metadata").cloned())
    };
    let ip_only = object(&[("requester_ip_address", Json::Text(CLIENT_IP.to_owned()))]);
    assert_eq!(metadata(&without), Some(Json::Object(ip_only)));
    let both = object(&[
        ("requester_ip_address", Json::Text(CLIENT_IP.to_owned())),
        ("user_agent", Json::Text("curl/8.5".to_owned())),
    ]);
    assert_eq!(metadata(&with), Some(Json::Object(both)));
}

#[tokio::test]
async fn log_before_raising_callback() {
    let fixture = Fixture::new();
    let ports = Recorder::replying(Reply::Raise);
    let outcome = fixture
        .handle(&ports, settings(false), http(401, false))
        .await;
    assert!(matches!(
        outcome,
        Err(HandlerError::Hook(HookError::Raised))
    ));
    let events = ports.events();
    assert_eq!(events.len(), 3);
    assert!(matches!(events.first(), Some(Event::Log(_))));
    assert!(matches!(events.get(1), Some(Event::Seed { .. })));
    assert!(matches!(events.get(2), Some(Event::Hook { .. })));
}

#[rstest]
#[case::quiet_malformed_key(http(401, true), false, Logger::ProxyStdout, Level::Warning, false)]
#[case::tracebacks_enabled(http(401, true), true, Logger::Proxy, Level::Error, true)]
#[case::unmarked_401(http(401, false), false, Logger::Proxy, Level::Error, false)]
#[case::marked_non_401(http(403, true), false, Logger::Proxy, Level::Error, false)]
#[case::unexpected_error(
    AuthFailure::Other { message: "boom".to_owned(), param: None },
    false,
    Logger::Proxy,
    Level::Error,
    true
)]
#[tokio::test]
async fn invalid_key_warning_stdout_and_traceback_matrix(
    #[case] error: AuthFailure,
    #[case] tracebacks: bool,
    #[case] logger: Logger,
    #[case] level: Level,
    #[case] traceback: bool,
) {
    let fixture = Fixture::new();
    let ports = Recorder::replying(Reply::Original);
    let _outcome = fixture.handle(&ports, settings(tracebacks), error).await;
    let logs = ports.logs();
    let first = logs.first().unwrap();
    assert_eq!(
        (first.logger, first.level, first.traceback),
        (logger, level, traceback)
    );
    assert!(
        first.message.contains("\nRequester IP Address:203.0.113.7"),
        "{}",
        first.message
    );
    assert_eq!(first.requester_ip.as_deref(), Some(CLIENT_IP));
    assert_eq!(logs.len(), 1);
}

#[tokio::test]
async fn failure_log_prints_the_exception_and_redacts_secrets() {
    let fixture = Fixture::new();
    let ports = Recorder::replying(Reply::Original);
    let mut leaky = http(401, false);
    if let AuthFailure::Http { detail, .. } = &mut leaky {
        *detail = "bad key sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned();
    }
    let _outcome = fixture.handle(&ports, settings(false), leaky).await;
    let logs = ports.logs();
    let expected = "litellm.proxy.proxy_server.user_api_key_auth(): Exception occured - \
                    401: bad key REDACTED\nRequester IP Address:203.0.113.7";
    assert_eq!(
        logs.first().map(|entry| entry.message.as_str()),
        Some(expected)
    );
}

#[tokio::test]
async fn identity_copy_api_key_route_and_model_seed() {
    let fixture = Fixture::new();
    let fresh = Recorder::replying(Reply::Original);
    let _anonymous = fixture
        .handle(&fresh, settings(false), http(401, false))
        .await;
    let (route, api_key, model, has_span) = seeded(&fresh);
    assert_eq!(route.as_deref(), Some("/v1/responses/{response_id}"));
    assert_eq!(api_key, Some(safe_hash_api_key("sk-presented")));
    assert_ne!(api_key.as_deref(), Some("sk-presented"));
    assert_eq!(model, Some(Json::Text("gpt-4o".to_owned())));
    assert!(has_span);
    let mut resolved = identity_with(&[("user_id", "known-user")]);
    resolved.extra.api_key = ApiKeySecret(Some(SecretString::from("existing-hash".to_owned())));
    let kept = Recorder::replying(Reply::Original);
    let mut request = fixture.request();
    request.resolved_identity = Some(&resolved);
    request.parent_span = None;
    let _known = fixture
        .handle_request(&kept, settings(false), http(401, false), request)
        .await;
    let (_, kept_key, _, kept_span) = seeded(&kept);
    assert_eq!(
        (kept_key.as_deref(), kept_span),
        (Some("existing-hash"), false)
    );
    assert!(resolved.extra.request_route.is_none());
    let users: Vec<Option<String>> = kept
        .events()
        .into_iter()
        .filter_map(|event| match event {
            Event::Hook { user_id, .. } => Some(user_id),
            Event::Service { .. } | Event::Log(_) | Event::Seed { .. } => None,
        })
        .collect();
    assert_eq!(users, vec![Some("known-user".to_owned())]);
}

#[rstest]
#[case::string_model(Json::Text("gpt-4o".to_owned()), "", vec!["gpt-4o"], "openai")]
#[case::non_string_model(Json::Number(Number::from(7_i32)), "", vec![], "litellm_proxy")]
#[case::provider_already_known(Json::Text("gpt-4o".to_owned()), "anthropic", vec![], "anthropic")]
#[tokio::test]
async fn budget_missing_provider_resolved_before_hook(
    #[case] model: Json,
    #[case] preset: &str,
    #[case] resolved: Vec<&str>,
    #[case] provider: &str,
) {
    let mut fixture = Fixture::new();
    fixture.request_data = object(&[("model", model)]);
    let ports = Recorder::replying(Reply::Original);
    let outcome = fixture
        .handle(&ports, settings(false), budget(preset))
        .await;
    assert_eq!(rejected(outcome).failure.exception().code, "422");
    assert_eq!(*ports.resolver.calls.lock().unwrap(), resolved);
    let calls = ports.hook_calls();
    let Some((_, original, error_type, route)) = calls.first() else {
        panic!("the failure hook runs once");
    };
    assert_eq!(original, &budget(provider));
    assert_eq!(*error_type, ProxyErrorTypes::AuthError);
    assert_eq!(route, RESPONSE_ROUTE);
}

#[rstest]
#[case::non_401_escalates(http(403, false), "403", false, 2)]
#[case::still_401_stays_quiet(http(401, false), "401", true, 1)]
#[tokio::test]
async fn transformed_exception_marker_and_non401_error_escalation(
    #[case] replacement: AuthFailure,
    #[case] code: &str,
    #[case] marked: bool,
    #[case] log_count: usize,
) {
    let fixture = Fixture::new();
    let ports = Recorder::replying(Reply::Replace(replacement));
    let outcome = fixture
        .handle(&ports, settings(false), http(401, true))
        .await;
    let raised = rejected(outcome);
    assert_eq!(raised.failure.exception().code, code);
    assert_eq!(raised.invalid_virtual_key, marked);
    let logs = ports.logs();
    assert_eq!(logs.len(), log_count);
    assert_eq!(
        logs.first().map(|entry| entry.logger),
        Some(Logger::ProxyStdout)
    );
    if log_count == 2 {
        assert!(matches!(
            ports.events().last(),
            Some(Event::Log(AuthLog {
                logger: Logger::Proxy,
                level: Level::Error,
                ..
            }))
        ));
    }
}

#[tokio::test]
async fn normal_hook_error_propagates() {
    let fixture = Fixture::new();
    let ports = Recorder::replying(Reply::Raise);
    let outcome = fixture
        .handle(
            &ports,
            settings(false),
            database(ErrorType::Connect, "refused"),
        )
        .await;
    let Err(HandlerError::Hook(error)) = outcome else {
        panic!("a raising failure hook reaches the caller");
    };
    assert_eq!(error.to_string(), "a post-call failure callback raised");
    assert_eq!(ports.logs().len(), 1);
}
