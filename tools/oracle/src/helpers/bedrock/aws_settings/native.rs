use super::{failure, input::Local};
use crate::helpers::HelperResult;
use aws_smithy_async::{
    rt::sleep::{SharedAsyncSleep, TokioSleep},
    time::{SharedTimeSource, StaticTimeSource},
};
use aws_smithy_runtime_api::client::{
    http::{HttpConnector, HttpConnectorFuture, SharedHttpConnector, http_client_fn},
    orchestrator::HttpRequest,
    result::ConnectorError,
};
use aws_types::os_shim_internal::Fs;
use betterlite_bedrock::aws_settings::sources::{
    NativeSources, auth_selector_presence, resolve_local_credentials,
};
use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::ExposeSecret as _;
use serde_json::json;
use std::io;
use std::time::{Duration, UNIX_EPOCH};

#[derive(Debug)]
struct NoTraffic;

impl HttpConnector for NoTraffic {
    fn call(&self, _: HttpRequest) -> HttpConnectorFuture {
        HttpConnectorFuture::ready(Err(ConnectorError::other(
            Box::new(io::Error::other("unexpected external IO")),
            None,
        )))
    }
}

pub(super) async fn local(input: &Local) -> HelperResult {
    let files: Vec<_> = input
        .files
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let sources = NativeSources {
        environment: input.environment.clone(),
        filesystem: Fs::from_slice(&files),
        client: http_client_fn(|_, _| SharedHttpConnector::new(NoTraffic)),
        clock: SharedTimeSource::new(StaticTimeSource::new(UNIX_EPOCH)),
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
        timeout: Duration::from_secs(5),
    };
    let settings = AwsSettings::from(&input.settings);
    let result = resolve_local_credentials(&settings, &sources)
        .await
        .map_err(failure)?;
    let source = local_source(&settings);
    let ttl = match source {
        "static" => result.cache_ttl.map(|ttl| ttl.as_secs()),
        "env" => Some(super::io::cache_default_ttl()?),
        _ => None,
    };
    let value = result.credentials;
    Ok(json!({
        "credentials": (
            value.access_key_id.expose_secret().to_owned(),
            value.secret_access_key.expose_secret().to_owned(),
            value.session_token.map(|token| token.expose_secret().to_owned()),
        ),
        "ttl": ttl,
        "source": source,
    }))
}

pub(super) const fn local_source(settings: &AwsSettings) -> &'static str {
    let [_web_identity, _role, profile, session_token, static_pair] =
        auth_selector_presence(settings);
    if profile {
        "profile"
    } else if session_token {
        "session_token"
    } else if static_pair {
        "static"
    } else {
        "env"
    }
}
