use super::call::GenericParams;
use super::dispatch::{Headers, Params};
use super::selection::SelectedConfig;
use betterlite_config::api::rerank_provider_plan::RerankSettings;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_error::dispatch::RawException;
use betterlite_models::resolution::ResolvedModel;
use providers::Branches;
use secrecy::SecretString;
use watsonx::{WatsonxRequest, watsonx_credentials};
mod providers;
mod watsonx;
pub(super) struct Credentials {
    pub(super) api_key: Option<SecretString>,
    pub(super) api_base: Option<String>,
    pub(super) headers: Headers,
}
pub(super) struct BranchInput<'request> {
    pub(super) provider: &'request str,
    pub(super) model: &'request str,
    pub(super) config: SelectedConfig,
    pub(super) resolved: &'request ResolvedModel,
    pub(super) generic: &'request GenericParams,
    pub(super) generic_params: &'request Params,
    pub(super) settings: &'request RerankSettings,
    pub(super) environment: &'request dyn EnvironmentLookup,
    pub(super) headers: Headers,
}
// Every branch of the dispatcher: the key and base chain it reads and the headers it hands its handler
pub(super) fn select_credentials(
    mut input: BranchInput<'_>,
    mapped: &mut Params,
) -> Result<Credentials, RawException> {
    let environment = input.settings.environment.resolve(input.environment);
    merge_extra_headers(input.provider, input.generic, &mut input.headers);
    if input.provider == "watsonx" {
        return watsonx_credentials(
            WatsonxRequest {
                model: input.model,
                provider: input.provider,
                resolved: input.resolved,
                generic_params: input.generic_params,
                environment: input.environment,
                headers: input.headers,
            },
            mapped,
        );
    }
    let branches = Branches {
        provider: input.provider,
        model: input.model,
        config: input.config,
        resolved: input.resolved,
        generic: input.generic,
        settings: input.settings,
        environment: &environment,
    };
    let (api_key, api_base) = branches.select()?;
    Ok(Credentials {
        api_key,
        api_base,
        headers: input.headers,
    })
}
// The call's extra_headers merge over the resolved headers for bedrock only
fn merge_extra_headers(provider: &str, generic: &GenericParams, headers: &mut Headers) {
    if provider != "bedrock" {
        return;
    }
    let Some(extra) = generic
        .extra_headers
        .as_ref()
        .filter(|extra| !extra.is_empty())
    else {
        return;
    };
    for (name, value) in extra {
        headers.insert(name.clone(), value.clone());
    }
}
