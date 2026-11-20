use betterlite_bedrock::model_routes::ParamMap;
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::reasoning_models::{DropParams, ReasoningCapabilities};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_openai::response_params::capabilities::{ModelInfo, ResponsesCapabilities};
use betterlite_openai::response_params::params::ResponsesParams;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_provider::bedrock_interactive_dispatch::responses;

use super::{ResponsesRun, shared};

struct Fakes;

impl ReasoningCapabilities for Fakes {
    fn supports_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl shared::Future<Output = bool> + Send {
        shared::ready(false)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl shared::Future<Output = Option<String>> + Send {
        shared::ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl shared::Future<Output = bool> + Send {
        shared::ready(false)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl shared::Future<Output = bool> + Send {
        shared::ready(false)
    }

    fn catalogue_declares_default(&mut self) -> bool {
        false
    }
}

impl ResponsesCapabilities for Fakes {
    fn model_info(&mut self, _model: &str) -> impl shared::Future<Output = ModelInfo> + Send {
        shared::ready(ModelInfo::Unavailable)
    }
}

struct MapFetcher;

impl RemoteMediaFetcher for MapFetcher {
    fn convert_url(
        &self,
        url: &str,
    ) -> impl shared::Future<Output = Result<String, ImageFetchError>> {
        shared::ready(Err(ImageFetchError::Rejected {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl shared::Future<Output = Result<Vec<String>, ImageFetchError>> {
        shared::ready(Err(ImageFetchError::Rejected {
            url: urls.first().cloned().unwrap_or_default(),
        }))
    }
}

struct ResponsesFixture {
    input: ResponsesInput,
    params: ResponsesParams,
    model_cost: ParamMap,
    bearer: Option<shared::SecretString>,
    capabilities: Fakes,
}

impl ResponsesFixture {
    fn new(run: &ResponsesRun) -> Result<Self, Box<shared::HelperError>> {
        let input = serde_json::from_value::<ResponsesInput>(run.input.clone())?;
        let params = serde_json::from_value::<ResponsesParams>(
            run.params.clone().unwrap_or_else(|| shared::json!({})),
        )?;
        let model_cost = if run.model_cost.is_empty() {
            serde_json::from_str::<ParamMap>(shared::CATALOG).unwrap_or_else(|_| ParamMap::new())
        } else {
            run.model_cost.clone()
        };
        Ok(Self {
            input,
            params,
            model_cost,
            bearer: run
                .api_key
                .as_deref()
                .map(|text| shared::SecretString::from(text.to_owned())),
            capabilities: Fakes,
        })
    }

    fn operation<'call>(
        &'call mut self,
        model: &'call str,
    ) -> responses::Operation<'call, MapFetcher, Fakes> {
        responses::Operation {
            aws: responses::AwsRequest {
                credentials: shared::credentials(),
                api_base: Some(shared::RESPONSES_HOST),
                aws_region_name: Some("us-west-2"),
                aws_bedrock_runtime_endpoint: None,
                environment_endpoint: None,
                region_inputs: RegionInputs::default(),
                api_key: self.bearer.as_ref(),
                environment_token: None,
                headers: shared::IndexMap::new(),
                signing_params: shared::BTreeMap::new(),
                now: shared::fixture_time(),
            },
            body: responses::BodyRequest {
                input: &self.input,
                params: &self.params,
                model_info: None,
            },
            model: Some(model),
            model_cost: &self.model_cost,
            fetcher: MapFetcher,
            capabilities: &mut self.capabilities,
            drop_params: DropParams {
                global: false,
                request: false,
            },
            bundled: BundledReasoningModels::default(),
            deadline: shared::DEADLINE,
        }
    }
}

pub(crate) fn responses_row(run: &ResponsesRun) -> shared::HelperResult {
    let mut fixture = ResponsesFixture::new(run)?;
    let transport = shared::Scripted(shared::reply(run.status, &run.reply));
    let outcome = shared::block_on(fixture.operation(&run.model).run(&transport))?;
    Ok(shared::outcome_row(outcome, failure, success))
}

fn success(outcome: &responses::Outcome) -> shared::Value {
    let mut row = shared::success_row(&outcome.request);
    row.insert("status".to_owned(), shared::json!(outcome.status));
    row.insert("reply".to_owned(), shared::body_json(&outcome.body));
    shared::Value::Object(row)
}

fn failure(error: &responses::Error) -> shared::Value {
    match error {
        responses::Error::Leaf(_) => shared::error_row("leaf", None),
        responses::Error::UnsupportedModel { .. } => shared::error_row("unsupported_model", None),
        responses::Error::Transport(_) => shared::error_row("transport", None),
        responses::Error::Rejected { status, .. } => shared::error_row("rejected", Some(*status)),
        responses::Error::Headers { .. } => shared::error_row("headers", None),
    }
}
