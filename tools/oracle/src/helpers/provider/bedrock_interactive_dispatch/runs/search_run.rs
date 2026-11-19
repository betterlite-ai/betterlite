use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::search::provider::SearchQuery as SearchInput;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::bedrock_interactive_dispatch::search;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::{SearchRun, shared};

struct SearchFixture {
    query: SearchInput,
    optional: IndexMap<String, Json>,
    bearer: Option<SecretString>,
}

static EMPTY_LOOKUP: BTreeMap<String, String> = BTreeMap::new();

impl SearchFixture {
    fn new(run: &SearchRun) -> Result<Self, Box<HelperError>> {
        let optional = run.optional.clone().unwrap_or_else(|| shared::json!({}));
        Ok(Self {
            query: SearchInput::Single(run.query.clone()),
            optional: serde_json::from_value(optional)?,
            bearer: run.api_key.clone().map(SecretString::from),
        })
    }

    fn operation<'call>(&'call self, run: &'call SearchRun) -> search::Operation<'call> {
        search::Operation {
            query: &self.query,
            optional_params: &self.optional,
            configured_tool_name: run.tool_name.as_deref(),
            headers: Headers::new(),
            configured_protocol_version: None,
            api_base: Some(shared::GATEWAY),
            configured_gateway_url: None,
            session_region: None,
            api_key: self.bearer.as_ref(),
            credentials: shared::credentials(),
            lookup: &EMPTY_LOOKUP,
            now: shared::fixture_time(),
        }
    }
}

pub(crate) fn search_row(run: &SearchRun) -> shared::HelperResult {
    let fixture = SearchFixture::new(run)?;
    let transport = shared::Scripted(shared::reply(run.status, &run.reply));
    let outcome = shared::block_on(fixture.operation(run).run(&transport))?;
    Ok(shared::outcome_row(outcome, failure, success))
}

fn success(outcome: &search::Outcome) -> shared::Value {
    let mut row = shared::success_row(&outcome.request);
    row.insert("response".to_owned(), shared::json!(outcome.response));
    shared::Value::Object(row)
}

fn failure(error: &search::Error) -> shared::Value {
    match error {
        search::Error::Leaf(_) => shared::error_row("leaf", None),
        search::Error::Reply(reply) => shared::error_row("reply", reply.status()),
        search::Error::Transport(_) => shared::error_row("transport", None),
    }
}
