use std::collections::BTreeMap;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::AwsCredentialResolver;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::ExposeSecret as _;
use tokio::runtime::Builder;

use super::failure;
use super::fixtures::{
    Transport, clocked_memory, fixture_cache, native_sources, secrets, sts_kwargs,
};
use super::io::memory_ttl;
use crate::helpers::HelperError;

pub(super) struct Observed {
    pub(super) credentials: Credentials,
    pub(super) actions: Vec<String>,
    pub(super) ttl: Option<u64>,
}

pub(super) fn credentials(
    settings: &AwsSettings,
    environment: &BTreeMap<String, String>,
    files: &BTreeMap<String, String>,
    now: u32,
) -> Result<Observed, Box<HelperError>> {
    let transport = Transport::default();
    let memory = clocked_memory(now)?;
    let cache = fixture_cache(&memory)?;
    let native = native_sources(environment, files, &transport);
    let secrets = secrets();
    let sts = sts_kwargs(settings, environment)?;
    let resolver = AwsCredentialResolver {
        cache: &cache,
        native: &native,
        secrets: &secrets,
        sts: &sts,
        max_policy_size: 75,
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)?;
    let credentials = runtime
        .block_on(resolver.get_credentials(settings, None))
        .map_err(failure)?;
    Ok(Observed {
        credentials,
        actions: transport.actions(),
        ttl: memory_ttl(&memory, now)?,
    })
}

pub(super) fn source(observed: &Observed, settings: &AwsSettings) -> &'static str {
    if observed
        .actions
        .iter()
        .any(|action| action == "AssumeRoleWithWebIdentity")
    {
        return "web_identity";
    }
    if observed.actions.iter().any(|action| action == "AssumeRole") {
        return "role";
    }
    if observed.ttl.is_some() {
        return cached_source(observed, settings);
    }
    if same_session_token(settings, &observed.credentials) {
        return "session_token";
    }
    "profile"
}

fn cached_source(observed: &Observed, settings: &AwsSettings) -> &'static str {
    let same_access = settings.access_key_id.as_ref().is_some_and(|configured| {
        configured.expose_secret() == observed.credentials.access_key_id.expose_secret()
    });
    let same_secret = settings
        .secret_access_key
        .as_ref()
        .is_some_and(|configured| {
            configured.expose_secret() == observed.credentials.secret_access_key.expose_secret()
        });
    if same_access && same_secret {
        "static"
    } else {
        "env"
    }
}

fn same_session_token(settings: &AwsSettings, credentials: &Credentials) -> bool {
    let Some(configured) = settings.session_token.as_ref() else {
        return false;
    };
    let Some(returned) = credentials.session_token.as_ref() else {
        return false;
    };
    configured.expose_secret() == returned.expose_secret()
        && settings
            .access_key_id
            .as_ref()
            .is_some_and(|value| value.expose_secret() == credentials.access_key_id.expose_secret())
        && settings.secret_access_key.as_ref().is_some_and(|value| {
            value.expose_secret() == credentials.secret_access_key.expose_secret()
        })
}
