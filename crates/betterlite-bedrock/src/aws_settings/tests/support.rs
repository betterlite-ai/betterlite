#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

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
use betterlite_config::{loader::ConfigLoader, sections::bedrock_settings::AwsSettings};
use betterlite_context::local_ttl_cache::{
    LocalTtlCache,
    dual::{Diagnostics, DualLocalCache},
};
use betterlite_redact::secret::Redactor;
use secrecy::SecretString;

use crate::aws_settings::{
    credential_cache::CredentialCache,
    sources::{Credentials, NativeSources, ResolvedCredentials},
};

pub(super) fn settings(params: &str) -> AwsSettings {
    let loaded = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&format!(
        "model_list:\n- model_name: test\n  litellm_params: {{model: bedrock/test, {params}}}\n"
    ))
    .unwrap();
    loaded
        .sections
        .bedrock_settings
        .deployments
        .into_iter()
        .next()
        .unwrap()
}

#[derive(Debug)]
struct NoTraffic;

impl HttpConnector for NoTraffic {
    fn call(&self, _: HttpRequest) -> HttpConnectorFuture {
        HttpConnectorFuture::ready(Err(ConnectorError::other(
            Box::new(io::Error::other(
                "fixture transport rejected unexpected HTTP",
            )),
            None,
        )))
    }
}

pub(super) fn native(files: &[(&str, &str)], environment: &[(&str, &str)]) -> NativeSources {
    NativeSources {
        environment: environment
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
        filesystem: Fs::from_slice(files),
        client: http_client_fn(|_, _| SharedHttpConnector::new(NoTraffic)),
        clock: SharedTimeSource::new(StaticTimeSource::new(UNIX_EPOCH)),
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
        timeout: Duration::from_secs(5),
    }
}

pub(super) fn cache(clock: Arc<AtomicU32>) -> CredentialCache {
    let memory = LocalTtlCache::with_clock(
        Some(1000),
        None,
        None,
        1024,
        Arc::new(move || f64::from(clock.load(Ordering::SeqCst))),
    )
    .unwrap();
    CredentialCache::new(DualLocalCache {
        memory: Arc::new(Mutex::new(memory)),
        default_in_memory_ttl: None,
        diagnostics: Diagnostics {
            redactor: Arc::new(Redactor::new(16).unwrap()),
            emit: Arc::new(|_| {}),
        },
    })
    .unwrap()
}

pub(super) fn credentials(access: &str) -> ResolvedCredentials {
    ResolvedCredentials {
        credentials: Some(Credentials {
            access_key_id: SecretString::from(access.to_owned()),
            secret_access_key: SecretString::from("secret-fixture".to_owned()),
            session_token: Some(SecretString::from("token-fixture".to_owned())),
        }),
        cache_ttl: None,
    }
}
