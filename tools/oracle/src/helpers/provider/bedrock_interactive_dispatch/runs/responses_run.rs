use super::*;

struct Fakes;

impl ReasoningCapabilities for Fakes {
    fn supports_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn catalogue_declares_default(&mut self) -> bool {
        false
    }
}

impl ResponsesCapabilities for Fakes {
    fn model_info(&mut self, _model: &str) -> impl Future<Output = ModelInfo> + Send {
        ready(ModelInfo::Unavailable)
    }
}

struct MapFetcher;

impl RemoteMediaFetcher for MapFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Rejected {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(Err(ImageFetchError::Rejected {
            url: urls.first().cloned().unwrap_or_default(),
        }))
    }
}

struct ResponsesFixture {
    input: ResponsesInput,
    params: ResponsesParams,
    model_cost: ParamMap,
    bearer: Option<SecretString>,
    capabilities: Fakes,
}

impl ResponsesFixture {
    fn new(run: &ResponsesRun) -> Result<Self, Box<HelperError>> {
        let input = serde_json::from_value::<ResponsesInput>(run.input.clone())?;
        let params = serde_json::from_value::<ResponsesParams>(
            run.params.clone().unwrap_or_else(|| json!({})),
        )?;
        let model_cost = if run.model_cost.is_empty() {
            serde_json::from_str::<ParamMap>(CATALOG).unwrap_or_else(|_| ParamMap::new())
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
                .map(|text| SecretString::from(text.to_owned())),
            capabilities: Fakes,
        })
    }

    fn operation<'call>(
        &'call mut self,
        model: &'call str,
    ) -> responses::Operation<'call, MapFetcher, Fakes> {
        responses::Operation {
            aws: responses::AwsRequest {
                credentials: credentials(),
                api_base: Some(RESPONSES_HOST),
                aws_region_name: Some("us-west-2"),
                aws_bedrock_runtime_endpoint: None,
                environment_endpoint: None,
                region_inputs: RegionInputs::default(),
                api_key: self.bearer.as_ref(),
                environment_token: None,
                headers: IndexMap::new(),
                signing_params: BTreeMap::new(),
                now: fixture_time(),
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
        }
    }
}

pub(crate) fn responses_row(run: &ResponsesRun) -> HelperResult {
    let mut fixture = ResponsesFixture::new(run)?;
    let transport = Scripted(reply(run.status, &run.reply));
    let outcome = block_on(fixture.operation(&run.model).run(&transport))?;
    Ok(outcome_row(outcome, responses_failure, responses_success))
}

fn responses_success(outcome: &responses::Outcome) -> Value {
    let mut row = success_row(&outcome.request);
    row.insert("status".to_owned(), json!(outcome.status));
    row.insert("reply".to_owned(), body_json(&outcome.body));
    Value::Object(row)
}

fn responses_failure(error: &responses::Error) -> Value {
    match error {
        responses::Error::Leaf(_) => error_row("leaf", None),
        responses::Error::UnsupportedModel { .. } => error_row("unsupported_model", None),
        responses::Error::Transport(_) => error_row("transport", None),
        responses::Error::Rejected { status, .. } => error_row("rejected", Some(*status)),
        responses::Error::Headers { .. } => error_row("headers", None),
    }
}
