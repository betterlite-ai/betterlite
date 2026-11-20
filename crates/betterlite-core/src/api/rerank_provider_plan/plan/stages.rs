use betterlite_config::api::rerank_provider_plan::RerankSettings;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_error::dispatch::RawException;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionRequest, ResolvedModel};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use super::super::branches::{BranchInput, Credentials, select_credentials};
use super::super::call::{GenericParams, RerankCall, TimeoutArg};
use super::super::dispatch::{Headers, OptionsRequest, Params, RerankOperations, value_failure};
use super::super::selection::{SelectedConfig, selected_config};
use super::{
    RerankException, RerankIdentity, call_headers, present_version_params, raise,
    rerank_litellm_params,
};

pub(super) struct PreparedCall {
    pub(super) kwargs: IndexMap<String, Json>,
    pub(super) is_async: bool,
    pub(super) generic: GenericParams,
    pub(super) generic_params: Params,
    pub(super) resolved: ResolvedModel,
    pub(super) provider: String,
}

pub(super) struct MappedOptions {
    pub(super) config: SelectedConfig,
    pub(super) params: Params,
    pub(super) litellm_params: Params,
    pub(super) headers: Headers,
    pub(super) timeout_seconds: Option<f64>,
}

pub(super) fn prepare<Identity>(
    identity: &Identity,
    redactor: &Redactor,
    call: &mut RerankCall,
) -> Result<PreparedCall, RerankException>
where
    Identity: RerankIdentity,
{
    let mut kwargs = std::mem::take(&mut call.kwargs);
    // `kwargs.pop("arerank", False) is True`: only the boolean true selects the async transport
    let is_async = matches!(kwargs.shift_remove("arerank"), Some(Json::Bool(true)));
    let provider_hint = call.custom_llm_provider.clone().unwrap_or_default();
    let generic = GenericParams::read(&kwargs).map_err(|error| {
        raise(
            redactor,
            value_failure(&error.to_string(), &call.model, &provider_hint),
        )
    })?;
    let resolution_request = ResolutionRequest {
        model: &call.model,
        custom_llm_provider: call.custom_llm_provider.as_deref(),
        api_base: generic.api_base.as_deref(),
        api_key: generic.api_key.as_ref(),
    };
    let resolved = identity.resolve(&resolution_request).map_err(|error| {
        raise(
            redactor,
            value_failure(&error.to_string(), &call.model, &provider_hint),
        )
    })?;
    let provider = resolved.provider.slug().to_owned();
    let generic_params = params_from_kwargs(&kwargs);
    Ok(PreparedCall {
        kwargs,
        is_async,
        generic,
        generic_params,
        resolved,
        provider,
    })
}

pub(super) fn map_options<Operations>(
    settings: &RerankSettings,
    operations: &Operations,
    redactor: &Redactor,
    prepared: &PreparedCall,
    call: &RerankCall,
) -> Result<MappedOptions, RerankException>
where
    Operations: RerankOperations,
{
    let config = selected_config(
        &prepared.resolved.provider,
        &prepared.resolved.model,
        prepared.generic.api_base.as_deref(),
        &present_version_params(call.max_chunks_per_doc, call.max_tokens_per_doc),
    );
    // `kwargs.get("drop_params") or litellm.drop_params or False`: a call false cannot switch the global true off
    let drop_params = prepared.generic.drop_params.unwrap_or(false) || settings.drop_params;
    let mut non_default_params = prepared.generic_params.clone();
    let params = operations
        .map_options(OptionsRequest {
            config,
            model: &prepared.resolved.model,
            drop_params,
            query: &call.query,
            documents: &call.documents,
            custom_llm_provider: Some(prepared.provider.as_str()),
            top_n: call.top_n,
            rank_fields: call.rank_fields.as_deref(),
            return_documents: call.return_documents,
            max_chunks_per_doc: call.max_chunks_per_doc,
            max_tokens_per_doc: call.max_tokens_per_doc,
            instruction: prepared.generic.instruction.as_deref(),
            non_default_params: &mut non_default_params,
        })
        .map_err(|failure| raise(redactor, failure))?;
    Ok(MappedOptions {
        config,
        params,
        litellm_params: rerank_litellm_params(&prepared.kwargs),
        headers: call_headers(&prepared.generic, settings),
        timeout_seconds: mapped_timeout(redactor, prepared)?,
    })
}

// a string timeout is converted with float(...) only after the provider mapped the options
fn mapped_timeout(
    redactor: &Redactor,
    prepared: &PreparedCall,
) -> Result<Option<f64>, RerankException> {
    prepared
        .generic
        .timeout
        .as_ref()
        .map(TimeoutArg::seconds)
        .transpose()
        .map_err(|error| {
            raise(
                redactor,
                value_failure(
                    &error.to_string(),
                    &prepared.resolved.model,
                    &prepared.provider,
                ),
            )
        })
}

pub(super) fn log(
    redactor: &Redactor,
    logging: &mut LoggingState,
    prepared: &PreparedCall,
    mapped: &MappedOptions,
) -> Result<(), RerankException> {
    let mut additional = IndexMap::new();
    additional.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(prepared.provider.clone()),
    );
    let litellm = as_index_map(&mapped.litellm_params);
    let update = CallUpdate {
        model: Some(prepared.resolved.model.clone()),
        user: prepared.generic.user.clone(),
        optional_params: as_index_map(&mapped.params),
        additional_params: additional,
    };
    update_from_kwargs(logging, &prepared.kwargs, Some(&litellm), update).map_err(|error| {
        raise(
            redactor,
            value_failure(
                &error.to_string(),
                &prepared.resolved.model,
                &prepared.provider,
            ),
        )
    })
}

pub(super) fn credentials(
    settings: &RerankSettings,
    environment: &dyn EnvironmentLookup,
    prepared: &PreparedCall,
    mapped: &mut MappedOptions,
) -> Result<Credentials, RawException> {
    select_credentials(
        BranchInput {
            provider: &prepared.provider,
            model: &prepared.resolved.model,
            config: mapped.config,
            resolved: &prepared.resolved,
            generic: &prepared.generic,
            generic_params: &prepared.generic_params,
            settings,
            environment,
            headers: mapped.headers.clone(),
        },
        &mut mapped.params,
    )
}

fn params_from_kwargs(kwargs: &IndexMap<String, Json>) -> Params {
    kwargs
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn as_index_map(params: &Params) -> IndexMap<String, Json> {
    params
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}
