use betterlite_auth::auth_model_lists::access_groups::AccessGroupScope;
use betterlite_auth::auth_model_lists::complete_list::{
    ListingFlags, ListingRequest, get_complete_model_list,
};
use betterlite_auth::auth_model_lists::fallbacks::get_all_fallbacks;
use betterlite_auth::auth_model_lists::key_models::get_key_models;
use betterlite_auth::auth_model_lists::mcp::get_mcp_server_ids;
use betterlite_auth::auth_model_lists::team_models::get_team_models;
use betterlite_auth::auth_model_lists::wildcard::ExpansionContext;
use betterlite_config::sections::fallback_plans::Settings;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

use fakes::{Catalog, Credential, FallbackSettings, Resolver, Row, captured, failure, snapshot};
mod fakes;
mod tests;
pub(crate) struct Op;
#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    KeyModels(Grants),
    TeamModels(Grants),
    CompleteModelList(CompleteModelList),
    McpServerIds(McpServerIds),
    AllFallbacks(AllFallbacks),
}
#[derive(Deserialize)]
pub(crate) struct Grants {
    key_models: Vec<String>,
    team_models: Vec<String>,
    proxy_model_list: Vec<String>,
    model_access_groups: IndexMap<String, Vec<String>>,
    include_model_access_groups: bool,
}
#[derive(Deserialize)]
pub(crate) struct CompleteModelList {
    key_models: Vec<String>,
    team_models: Vec<String>,
    proxy_model_list: Vec<String>,
    user_model: Option<String>,
    infer_model_from_keys: bool,
    return_wildcard_routes: bool,
    model_access_groups: IndexMap<String, Vec<String>>,
    include_model_access_groups: bool,
    only_model_access_groups: bool,
    team_id: Option<String>,
    deployments: Vec<Row>,
    catalog: IndexMap<String, Vec<String>>,
    credentials: Vec<Credential>,
}
#[derive(Deserialize)]
pub(crate) struct McpServerIds {
    object_permission_id: Option<String>,
}
#[derive(Deserialize)]
pub(crate) struct AllFallbacks {
    model: String,
    fallback_type: String,
    settings: Option<FallbackSettings>,
}
impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::KeyModels(case) => key_models(case),
            Input::TeamModels(case) => team_models(case),
            Input::CompleteModelList(case) => complete_model_list(case),
            Input::McpServerIds(case) => mcp_server_ids(case),
            Input::AllFallbacks(case) => all_fallbacks(case),
        }
    }
}
fn key_models(case: &Grants) -> HelperResult {
    to_value(get_key_models(
        &case.key_models,
        &case.team_models,
        &case.proxy_model_list,
        &case.model_access_groups,
        AccessGroupScope {
            include: case.include_model_access_groups,
            only: false,
        },
    ))
}
// LiteLLM builds a Python set here, so the sorted answer is the comparable one.
fn team_models(case: &Grants) -> HelperResult {
    let mut models = get_team_models(
        &case.team_models,
        &case.proxy_model_list,
        &case.model_access_groups,
        case.include_model_access_groups,
    );
    models.sort();
    to_value(models)
}
fn complete_model_list(case: &CompleteModelList) -> HelperResult {
    let credentials = snapshot(&case.credentials);
    let catalog = Catalog(&case.catalog);
    let deployments = fakes::Rows(&case.deployments);
    let context = ExpansionContext {
        providers: &catalog,
        credentials: &credentials,
        deployments: Some(&deployments),
    };
    let request = ListingRequest {
        key_models: &case.key_models,
        team_models: &case.team_models,
        user_model: case.user_model.as_deref(),
        team_id: case.team_id.as_deref(),
        model_access_groups: &case.model_access_groups,
        flags: ListingFlags {
            infer_model_from_keys: case.infer_model_from_keys,
            return_wildcard_routes: case.return_wildcard_routes,
            groups: AccessGroupScope {
                include: case.include_model_access_groups,
                only: case.only_model_access_groups,
            },
        },
    };
    get_complete_model_list(&case.proxy_model_list, &request, &context)
        .map_err(|error| failure(&error))
        .and_then(to_value)
}
// LiteLLM's `prisma_client` is None until the proxy starts, so no permission row is ever read here.
fn mcp_server_ids(case: &McpServerIds) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| failure(&error))?;
    to_value(runtime.block_on(get_mcp_server_ids(
        None,
        case.object_permission_id.as_deref(),
    )))
}
fn all_fallbacks(case: &AllFallbacks) -> HelperResult {
    let settings = case.settings.as_ref().map(|configured| Settings {
        fallbacks: configured.fallbacks.clone(),
        context_window_fallbacks: configured.context_window_fallbacks.clone(),
        content_policy_fallbacks: configured.content_policy_fallbacks.clone(),
        max_fallbacks: 0,
    });
    let (fallbacks, warnings) = captured(|| {
        get_all_fallbacks(
            &case.model,
            &case.fallback_type,
            settings.as_ref(),
            &Resolver,
        )
    });
    Ok(json!({"fallbacks": fallbacks, "warnings": warnings}))
}
// ast-grep-ignore: no-one-line-functions
fn to_value<T: serde::Serialize>(value: T) -> HelperResult {
    serde_json::to_value(value).map_err(|error| failure(&error))
}
