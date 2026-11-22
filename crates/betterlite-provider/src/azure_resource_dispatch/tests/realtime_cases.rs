use betterlite_azure::realtime::connect::RealtimeConnectSettings;
use betterlite_azure::realtime::error::RealtimeError;
use betterlite_azure::realtime::http_config::RealtimeHttpSettings;
use betterlite_config::tls::Verification;
use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;

use crate::azure_resource_dispatch::realtime::{self, Call, Outcome, SessionRoute};
use crate::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};

fn connect_settings<'text>(
    key: &'text SecretString,
    api_base: Option<&'text str>,
    api_version: Option<&'text str>,
    protocol: Option<&'text str>,
) -> RealtimeConnectSettings<'text> {
    RealtimeConnectSettings {
        api_base,
        api_key: Some(key),
        azure_ad_token: None,
        model: "gpt-4o-realtime-preview",
        api_version,
        realtime_protocol: protocol,
        query_params: None,
        ssl_verification: &Verification::Enabled,
        max_message_size_bytes: 4096,
    }
}

fn connect(settings: &RealtimeConnectSettings<'_>) -> Result<Outcome, RealtimeError> {
    let result = super::block_on(dispatch(AzureResourceOperation::Realtime(Box::new(
        realtime::Dispatch {
            call: Call::Connection(settings),
        },
    ))));
    match result {
        Ok(AzureResourceResult::Realtime(outcome)) => Ok(outcome),
        Err(AzureResourceError::Realtime(error)) => Err(error),
        Ok(_) | Err(_) => panic!("the realtime dispatch returns its own outcome"),
    }
}

fn session(call: Call<'_>) -> Outcome {
    let result = super::block_on(dispatch(AzureResourceOperation::Realtime(Box::new(
        realtime::Dispatch { call },
    ))));
    match result {
        Ok(AzureResourceResult::Realtime(outcome)) => outcome,
        Ok(_) | Err(_) => panic!("the realtime dispatch returns its own outcome"),
    }
}

fn connection_plan_row() {
    let key = SecretString::from("key-1");
    let settings = connect_settings(&key, Some("https://res.test"), None, None);
    let Outcome::Connection(plan) = connect(&settings).expect("the plan builds") else {
        panic!("the connection call returns the plan");
    };
    assert_eq!(
        plan.url,
        "wss://res.test/openai/v1/realtime?model=gpt-4o-realtime-preview"
    );
    assert_eq!(
        plan.headers.get("api-key").map(String::as_str),
        Some("key-1")
    );
    assert_eq!(plan.max_message_size_bytes, 4096_usize);
    assert!(!plan.backend_uses_beta_protocol);
    assert!(plan.force_transcription_model.is_none());
}

fn connection_beta_rows() {
    let key = SecretString::from("key-1");
    let beta = connect_settings(
        &key,
        Some("https://res.test"),
        Some("2024-10-01-preview"),
        Some("beta"),
    );
    let Outcome::Connection(plan) = connect(&beta).expect("the beta plan builds") else {
        panic!("the connection call returns the plan");
    };
    assert!(plan.backend_uses_beta_protocol);
    assert!(plan.url.starts_with("wss://res.test/openai/realtime?"));
    assert!(plan.url.contains("api-version=2024-10-01-preview"));
    assert!(plan.url.contains("deployment=gpt-4o-realtime-preview"));

    let unversioned = connect_settings(&key, Some("https://res.test"), None, Some("beta"));
    assert!(matches!(
        connect(&unversioned),
        Err(RealtimeError::MissingApiVersion)
    ));
    let baseless = connect_settings(&key, None, None, None);
    assert!(matches!(
        connect(&baseless),
        Err(RealtimeError::MissingApiBase)
    ));
}

fn session_settings() -> RealtimeHttpSettings {
    RealtimeHttpSettings {
        litellm_api_base: None,
        litellm_api_key: None,
        azure_api_base: Some(String::from("https://res.test")),
        azure_api_key: Some(SecretString::from("key-1")),
        azure_api_version: None,
    }
}

fn session_client_url_row() {
    let settings = session_settings();
    let Outcome::Url(secrets) = session(Call::SessionUrl {
        route: SessionRoute::ClientSecrets,
        settings: &settings,
        base: None,
        version: None,
    }) else {
        panic!("the session url call returns its endpoint");
    };
    assert_eq!(
        secrets,
        "https://res.test/openai/realtime/client_secrets?api-version=2024-12-17"
    );
}

fn session_other_url_rows() {
    let settings = session_settings();
    let Outcome::Url(calls) = session(Call::SessionUrl {
        route: SessionRoute::Calls,
        settings: &settings,
        base: Some("https://caller.test"),
        version: Some("2025-01-01"),
    }) else {
        panic!("the calls url call returns its endpoint");
    };
    assert_eq!(
        calls,
        "https://caller.test/openai/realtime/calls?api-version=2025-01-01"
    );
    let Outcome::Url(transcription) = session(Call::SessionUrl {
        route: SessionRoute::Transcription,
        settings: &settings,
        base: None,
        version: None,
    }) else {
        panic!("the transcription url call returns its endpoint");
    };
    assert_eq!(
        transcription,
        "https://res.test/openai/realtime/transcription_sessions?api-version=2024-12-17"
    );
}

fn session_header_rows() {
    let key = SecretString::from("key-1");
    let Outcome::Headers(headers) = session(Call::SessionHeaders {
        headers: &Headers::new(),
        api_key: Some(&key),
    }) else {
        panic!("the session headers call returns headers");
    };
    assert_eq!(headers.get("api-key").map(String::as_str), Some("key-1"));
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );

    let Outcome::Headers(calls) = session(Call::CallsHeaders("ek_1")) else {
        panic!("the calls headers call returns headers");
    };
    assert_eq!(calls.get("api-key").map(String::as_str), Some("ek_1"));
}

pub(super) fn check() {
    connection_plan_row();
    connection_beta_rows();
    session_client_url_row();
    session_other_url_rows();
    session_header_rows();
}
