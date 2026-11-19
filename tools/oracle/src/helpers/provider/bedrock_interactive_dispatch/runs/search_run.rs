use super::*;

struct SearchFixture {
    query: SearchInput,
    optional: IndexMap<String, Json>,
    bearer: Option<SecretString>,
}

static EMPTY_LOOKUP: BTreeMap<String, String> = BTreeMap::new();

impl SearchFixture {
    fn new(run: &SearchRun) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            query: SearchInput::Single(run.query.clone()),
            optional: serde_json::from_value(run.optional.clone().unwrap_or_else(|| json!({})))?,
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
            api_base: Some(GATEWAY),
            configured_gateway_url: None,
            session_region: None,
            api_key: self.bearer.as_ref(),
            credentials: credentials(),
            lookup: &EMPTY_LOOKUP,
            now: fixture_time(),
        }
    }
}

pub(crate) fn search_row(run: &SearchRun) -> HelperResult {
    let fixture = SearchFixture::new(run)?;
    let transport = Scripted(reply(run.status, &run.reply));
    let outcome = block_on(fixture.operation(run).run(&transport))?;
    Ok(outcome_row(outcome, search_failure, search_success))
}

fn search_success(outcome: &search::Outcome) -> Value {
    let mut row = success_row(&outcome.request);
    row.insert("response".to_owned(), json!(outcome.response));
    Value::Object(row)
}

fn search_failure(error: &search::Error) -> Value {
    match error {
        search::Error::Leaf(_) => error_row("leaf", None),
        search::Error::Reply(reply) => error_row("reply", reply.status()),
        search::Error::Transport(_) => error_row("transport", None),
    }
}
