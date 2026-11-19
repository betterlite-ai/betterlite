use super::*;

struct VectorFixture {
    query: SearchQuery,
    optional: SearchOptionalParams,
    litellm_params: IndexMap<String, Json>,
    extra_body: Option<IndexMap<String, Json>>,
}

impl VectorFixture {
    fn new(run: &VectorRun) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            query: SearchQuery::Text(run.query.clone()),
            optional: serde_json::from_value(run.optional.clone().unwrap_or_else(|| json!({})))?,
            litellm_params: serde_json::from_value(json!({"aws_region_name": "us-east-1"}))?,
            extra_body: run
                .extra_body
                .clone()
                .map(serde_json::from_value::<IndexMap<String, Json>>)
                .transpose()?,
        })
    }

    fn operation(&self) -> vector_retrieve::Operation<'_> {
        vector_retrieve::Operation {
            vector_store_id: VECTOR_ID,
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
            credentials: credentials(),
            region_inputs: RegionInputs::default(),
            now: fixture_time(),
        }
    }
}

pub(crate) fn vector_row(run: &VectorRun) -> HelperResult {
    let fixture = VectorFixture::new(run)?;
    let transport = Scripted(reply(run.status, &run.reply));
    let outcome = block_on(fixture.operation().run(&transport))?;
    Ok(outcome_row(outcome, vector_failure, vector_success))
}

fn vector_success(outcome: &vector_retrieve::Outcome) -> Value {
    let mut row = empty_row();
    row.insert(
        "request".to_owned(),
        request_row("POST", &outcome.request.url, &[], &outcome.request.body),
    );
    row.insert("response".to_owned(), json!(outcome.response));
    Value::Object(row)
}

fn vector_failure(error: &vector_retrieve::Error) -> Value {
    match error {
        vector_retrieve::Error::Leaf(_) => error_row("leaf", None),
        vector_retrieve::Error::Transport(_) => error_row("transport", None),
    }
}
