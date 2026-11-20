use std::collections::BTreeMap;

use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_bedrock::vector_stores::request::{SearchOptionalParams, SearchQuery};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::bedrock_interactive_dispatch::vector_retrieve;
use indexmap::IndexMap;

use super::{VectorRun, shared};

struct VectorFixture {
    query: SearchQuery,
    optional: SearchOptionalParams,
    litellm_params: IndexMap<String, Json>,
    extra_body: Option<IndexMap<String, Json>>,
}

impl VectorFixture {
    fn new(run: &VectorRun) -> Result<Self, Box<HelperError>> {
        let optional = run.optional.clone().unwrap_or_else(|| shared::json!({}));
        let litellm_params = shared::json!({"aws_region_name": "us-east-1"});
        Ok(Self {
            query: SearchQuery::Text(run.query.clone()),
            optional: serde_json::from_value(optional)?,
            litellm_params: serde_json::from_value(litellm_params)?,
            extra_body: run
                .extra_body
                .clone()
                .map(serde_json::from_value::<IndexMap<String, Json>>)
                .transpose()?,
        })
    }

    fn operation(&self) -> vector_retrieve::Operation<'_> {
        vector_retrieve::Operation {
            vector_store_id: shared::VECTOR_ID,
            query: &self.query,
            optional: &self.optional,
            litellm_params: &self.litellm_params,
            extra_body: self.extra_body.as_ref(),
            api_base: None,
            configured_endpoint: None,
            environment_endpoint: None,
            aws_region_name: Some("us-east-1"),
            environment_region_name: None,
            environment_region: None,
            headers: BTreeMap::new(),
            signing_params: BTreeMap::new(),
            api_key: None,
            configured_bearer: None,
            credentials: shared::credentials(),
            region_inputs: RegionInputs::default(),
            now: shared::fixture_time(),
            deadline: shared::DEADLINE,
        }
    }
}

pub(crate) fn vector_row(run: &VectorRun) -> shared::HelperResult {
    let fixture = VectorFixture::new(run)?;
    let transport = shared::Scripted(shared::reply(run.status, &run.reply));
    let outcome = shared::block_on(fixture.operation().run(&transport))?;
    Ok(shared::outcome_row(outcome, failure, success))
}

fn success(outcome: &vector_retrieve::Outcome) -> shared::Value {
    let mut request = outcome.request.clone();
    request.headers = Vec::new();
    let mut row = shared::success_row(&request);
    row.insert("response".to_owned(), shared::json!(outcome.response));
    shared::Value::Object(row)
}

fn failure(error: &vector_retrieve::Error) -> shared::Value {
    match error {
        vector_retrieve::Error::Leaf(_) => shared::error_row("leaf", None),
        vector_retrieve::Error::Transport(_) => shared::error_row("transport", None),
    }
}
