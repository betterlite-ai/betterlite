use std::collections::BTreeMap;
use std::fmt::Display;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_bedrock::converse_request::{
    ConverseMedia, ConverseRequestBody, ConverseRequestInput, ConverseSettings, converse_request,
};
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_bedrock::request_policy::BedrockRequestPolicy;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{Resolve, UrlValidator};
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::bedrock_images::remote::{RemoteImage, TRANSPORT_LIMITS};
use serde_json::json;
use tokio::runtime::{Builder, Runtime};

use super::fixture::{CaseLookup, Fixture};
use super::input::Input;

const MAX_DOWNLOAD_SIZE_MB: f64 = 50.0;

fn helper_error<E>(error: E, status: Option<u16>) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "BedrockConverseRequest".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn request_policy(fields: &[String]) -> Option<BedrockRequestPolicy> {
    if fields.is_empty() {
        return None;
    }
    Some(BedrockRequestPolicy {
        fields: fields.iter().cloned().collect(),
    })
}

fn current_thread_runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error(error, None))
}

// Oracle cases carry supplied bytes only, so the seam never resolves a host.
#[derive(Debug, Clone, Copy)]
struct Refusing;

#[async_trait::async_trait]
impl Resolve for Refusing {
    async fn resolve(&self, _host: &str, _port: u16) -> io::Result<Vec<SocketAddr>> {
        Err(io::Error::other("the oracle helper resolves no host"))
    }
}

fn remote() -> Result<RemoteImage<Refusing>, Box<HelperError>> {
    let proxy_policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(|error| helper_error(error, None))?,
    );
    let client = ProxiedClient::build(base_builder(), &proxy_policy, TRANSPORT_LIMITS)
        .map_err(|error| helper_error(error, None))?;
    let hosts: Vec<String> = Vec::new();
    let validator = UrlValidator::new(Refusing, &hosts, &Verification::Enabled);
    Ok(RemoteImage {
        client,
        urls: UserUrlFetcher {
            validator,
            validation: true,
        },
        deadline: TRANSPORT_LIMITS.read_timeout,
        max_download_size_mb: MAX_DOWNLOAD_SIZE_MB,
    })
}

struct Assembly<'request> {
    input: &'request Input,
    fixture: &'request Fixture,
    media: &'request RemoteImage<Refusing>,
    lookup: CaseLookup<'request>,
    policy: Option<BedrockRequestPolicy>,
    catalogs: CatalogSources<'request>,
    optional_params: &'request mut ParamMap,
    litellm_params: &'request mut Object,
}

fn assemble(
    input: &Input,
    fixture: &Fixture,
    media: &RemoteImage<Refusing>,
    optional_params: &mut ParamMap,
    litellm_params: &mut Object,
) -> Result<ConverseRequestBody, Box<HelperError>> {
    let assembly = Assembly {
        input,
        fixture,
        media,
        lookup: CaseLookup {
            model_cost: &input.model_cost,
        },
        policy: input
            .request_metadata_fields
            .as_deref()
            .and_then(request_policy),
        catalogs: fixture.catalogs(),
        optional_params,
        litellm_params,
    };
    current_thread_runtime()?.block_on(assemble_body(assembly))
}

async fn assemble_body(
    mut assembly: Assembly<'_>,
) -> Result<ConverseRequestBody, Box<HelperError>> {
    let mut keys = ModelCostKeys::default();
    let dynamic = NoDynamicMetadata;
    let resolution = assembly.fixture.resolution();
    let mut environment = assembly
        .fixture
        .capability_environment(&resolution, &mut keys, &dynamic);
    let new_id = || "call_generated".to_owned();
    let request = ConverseRequestInput {
        model: &assembly.input.model,
        messages: &assembly.input.messages,
        optional_params: &mut *assembly.optional_params,
        litellm_params: Some(&mut *assembly.litellm_params),
        headers: None,
        settings: ConverseSettings {
            global_drop: assembly.input.global_drop,
            modify_params: assembly.input.modify_params,
            neutralize_orphans: assembly.input.neutralize_orphaned_blocks,
            request_metadata: assembly.policy.as_ref(),
        },
        model_cost: &assembly.input.model_cost,
        catalogs: &assembly.catalogs,
        lookup: &assembly.lookup,
        packaged: &assembly.fixture.packaged,
    };
    converse_request(
        request,
        ConverseMedia {
            remote: assembly.media,
            new_id: &new_id,
        },
        &mut environment,
    )
    .await
    .map_err(|error| helper_error(&error, error.status_code()))
}

pub(super) fn run(input: &Input) -> HelperResult {
    let fixture = Fixture::load(&input.model_cost).map_err(|error| helper_error(error, None))?;
    let media = remote()?;
    let mut optional_params = input.optional_params.clone();
    let mut litellm_params = input.litellm_params.clone();
    let body = assemble(
        input,
        &fixture,
        &media,
        &mut optional_params,
        &mut litellm_params,
    )?;
    Ok(json!({
        "body": body,
        "optional_params": optional_params,
        "litellm_params": litellm_params,
    }))
}
