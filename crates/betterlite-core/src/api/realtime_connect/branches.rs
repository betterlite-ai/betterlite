pub(super) mod vertex;

use betterlite_azure::realtime::protocol::client_protocol;
use betterlite_config::api::realtime_connect::RealtimeConnectionSettings;
use betterlite_config::sections::xai_identity::Settings as XaiIdentitySettings;
use betterlite_openai::realtime::connect::DEFAULT_API_BASE;
use betterlite_openai::realtime::url::QueryParams;
use betterlite_openai::xai_identity::{self, KeyOrder};
use betterlite_openai::xai_realtime::connect::XAI_API_BASE;
use secrecy::{ExposeSecret as _, SecretString};

use super::dispatch::{Forwarding, Prepared, connection_args, connector_query};
use super::{
    AZURE_DEFAULT_API_VERSION, AzureAdTokenRequest, Branch, RealtimeConnectCall,
    RealtimeConnectDispatch, RealtimeConnectError, RealtimeConnection,
};

// OpenAI and xAI differ only in where their base and key come from
#[derive(Clone, Copy)]
pub(super) enum DirectProvider {
    OpenAi,
    Xai,
}

pub(super) fn connector<Config>(
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
    api_base: Option<String>,
    api_key: Option<SecretString>,
    branch: Branch<Config>,
) -> RealtimeConnection<Config> {
    RealtimeConnection {
        args: connection_args(call, prepared, api_base, api_key, Forwarding::Connector),
        branch,
    }
}

pub(super) fn azure_connection<Dispatch>(
    dispatch: &Dispatch,
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
) -> Result<RealtimeConnection<Dispatch::ProviderConfig>, RealtimeConnectError<Dispatch::Error>>
where
    Dispatch: RealtimeConnectDispatch,
{
    let (api_base, api_key) = azure_base_and_key(settings, call, prepared);
    let api_version = azure_api_version(call);
    let query = connector_query(prepared.query_params.as_ref());
    let protocol = azure_protocol(settings, call, query.as_ref());
    // a truthy key suppresses the Azure AD lookup entirely
    let azure_ad_token = if api_key.is_none() {
        dispatch.azure_ad_token(AzureAdTokenRequest {
            azure_ad_token: call.params.azure_ad_token.as_ref(),
            params: call.params,
        })?
    } else {
        None
    };
    Ok(connector(
        call,
        prepared,
        api_base,
        api_key,
        Branch::Azure {
            api_version,
            protocol,
            azure_ad_token,
        },
    ))
}

fn azure_base_and_key(
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
) -> (Option<String>, Option<SecretString>) {
    let api_base = first_text([
        prepared.dynamic_api_base.as_deref(),
        call.params.generic_api_base.as_deref(),
        settings.api_base.as_deref(),
        settings.azure_api_base.as_deref(),
    ]);
    let api_key = first_secret([
        prepared.dynamic_api_key.as_ref(),
        settings.api_key.as_ref(),
        settings.openai_key.as_ref(),
        settings.azure_api_key.as_ref(),
    ]);
    (api_base, api_key)
}

fn azure_api_version(call: &RealtimeConnectCall<'_>) -> String {
    first_text([
        call.params.api_version.as_deref(),
        call.params.generic_api_version.as_deref(),
    ])
    .unwrap_or_else(|| String::from(AZURE_DEFAULT_API_VERSION))
}

fn azure_protocol(
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
    query: Option<&QueryParams>,
) -> String {
    let configured = first_text([
        call.params.realtime_protocol.as_deref(),
        call.params.generic_realtime_protocol.as_deref(),
        settings.azure_realtime_protocol.as_deref(),
    ]);
    client_protocol(configured.as_deref(), query, call.client_sent_beta)
}

pub(super) fn direct_connection<Config>(
    provider: DirectProvider,
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
) -> RealtimeConnection<Config> {
    let dynamic = prepared.dynamic_api_base.as_deref();
    let generic = call.params.generic_api_base.as_deref();
    match provider {
        DirectProvider::OpenAi => {
            let api_base = first_text([dynamic, generic, settings.api_base.as_deref()])
                .unwrap_or_else(|| String::from(DEFAULT_API_BASE));
            let api_key = first_secret([
                prepared.dynamic_api_key.as_ref(),
                settings.api_key.as_ref(),
                settings.openai_key.as_ref(),
                settings.openai_api_key.as_ref(),
            ]);
            connector(call, prepared, Some(api_base), api_key, Branch::OpenAi)
        }
        DirectProvider::Xai => {
            let api_base = first_text([dynamic, generic, settings.xai_api_base.as_deref()])
                .unwrap_or_else(|| String::from(XAI_API_BASE));
            let api_key = xai_api_key(settings, prepared);
            connector(call, prepared, Some(api_base), api_key, Branch::Xai)
        }
    }
}

fn xai_api_key(settings: &RealtimeConnectionSettings, prepared: &Prepared) -> Option<SecretString> {
    xai_identity::get_api_key(
        prepared.dynamic_api_key.as_ref(),
        KeyOrder::GenericBeforeEnvironment,
        &XaiIdentitySettings {
            xai_key: settings.xai_key.clone(),
            generic_api_key: settings.api_key.clone(),
            environment_key: settings.xai_api_key.clone(),
            api_base: settings.xai_api_base.clone(),
        },
    )
}

// bedrock reads the named api_base and api_key, with no generic or global fallback
pub(super) fn bedrock_connection<Config>(
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
) -> RealtimeConnection<Config> {
    let params = call.params;
    let api_base = first_text([
        prepared.dynamic_api_base.as_deref(),
        params.api_base.as_deref(),
    ]);
    let api_key = first_secret([prepared.dynamic_api_key.as_ref(), params.api_key.as_ref()]);
    let args = connection_args(call, prepared, api_base, api_key, Forwarding::Bedrock);
    RealtimeConnection {
        args,
        branch: Branch::Bedrock(Box::new(params.clone())),
    }
}

fn first_text<const N: usize>(candidates: [Option<&str>; N]) -> Option<String> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

fn first_secret<const N: usize>(candidates: [Option<&SecretString>; N]) -> Option<SecretString> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty())
        .cloned()
}
