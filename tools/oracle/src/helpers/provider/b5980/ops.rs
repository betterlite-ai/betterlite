use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_prompts::arguments::json::Json;
use betterlite_provider::databricks_auth::Headers;
use betterlite_provider::databricks_auth::api_base::get_api_base;
use betterlite_provider::databricks_auth::environment::DatabricksEnv;
use betterlite_provider::databricks_auth::oauth::{
    OauthReply, OauthTokenRequest, OauthTokenSource,
};
use betterlite_provider::databricks_auth::redaction::{
    redact_headers_for_logging, redact_sensitive_data,
};
use betterlite_provider::databricks_auth::user_agent::build_user_agent;
use betterlite_provider::databricks_auth::validate::{ValidationRequest, validate_environment};
use betterlite_provider::databricks_embedding::{
    EmbeddingPreparation, PreparedEmbedding, prepare_embedding,
};
use betterlite_provider::databricks_responses::DatabricksResponsesAPIConfig;
use secrecy::SecretString;
use serde_json::json;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::{
    ApiBaseCase, DEFAULT_VERSION, EmbeddingCase, EnvInput, UrlCase, UserAgentCase, ValidateCase,
    ValidateInput,
};
use betterlite_provider::databricks_auth::error::DatabricksError;
use tokio::runtime::{Builder, Runtime};

const WATCHED_HEADERS: [&str; 3] = ["authorization", "content-type", "user-agent"];

struct ScriptedTokens {
    reply: OauthReply,
    sent: Mutex<Vec<OauthTokenRequest>>,
}

impl ScriptedTokens {
    const fn new(reply: OauthReply) -> Self {
        Self {
            reply,
            sent: Mutex::new(Vec::new()),
        }
    }

    fn attach(&self, mut value: serde_json::Value) -> serde_json::Value {
        let recorded = self.sent.lock().ok().and_then(|sent| {
            let request = sent.first()?;
            Some(json!({
                "url": &request.url,
                "body": &request.body,
                "authorization": &request.authorization,
                "content_type": &request.content_type,
                "timeout_seconds": request.timeout.as_secs(),
            }))
        });
        if let Some(fields) = value.as_object_mut() {
            fields.insert("request".to_owned(), recorded.unwrap_or_default());
        }
        value
    }
}

#[async_trait]
impl OauthTokenSource for ScriptedTokens {
    async fn send(&self, request: &OauthTokenRequest) -> OauthReply {
        if let Ok(mut sent) = self.sent.lock() {
            sent.push(request.clone());
        }
        self.reply.clone()
    }
}

fn configure<T, F>(outcome: Result<T, DatabricksError>, render: F) -> serde_json::Value
where
    F: FnOnce(T) -> serde_json::Value,
{
    match outcome {
        Ok(value) => render(value),
        Err(error) => json!({"status": error.status_code}),
    }
}

pub(super) fn validate(input: &ValidateInput) -> HelperResult {
    let env = input.env.build();
    let runtime = Builder::new_current_thread().build().or_config_error()?;
    let mut results = Vec::with_capacity(input.cases.len());
    for case in &input.cases {
        results.push(validate_case(case, &env, &runtime));
    }
    Ok(json!(results))
}

impl ValidateCase {
    fn request<'case>(&'case self, key: Option<&'case SecretString>) -> ValidationRequest<'case> {
        ValidationRequest {
            api_key: key,
            api_base: self.api_base.as_deref(),
            endpoint: self.endpoint.endpoint(),
            custom_endpoint: self.custom_endpoint,
            headers: self.headers.clone(),
            custom_user_agent: self.custom_user_agent.as_deref(),
        }
    }
}

fn validate_case(case: &ValidateCase, env: &DatabricksEnv, runtime: &Runtime) -> serde_json::Value {
    let key = case.api_key.clone().map(SecretString::from);
    let tokens = ScriptedTokens::new(case.oauth.reply());
    let outcome = runtime.block_on(validate_environment(
        case.request(key.as_ref()),
        env,
        &tokens,
    ));
    let rendered = configure(
        outcome,
        |(url, headers)| json!({"url": url, "headers": headers}),
    );
    tokens.attach(rendered)
}

pub(super) fn api_base_case(case: &ApiBaseCase) -> serde_json::Value {
    match get_api_base(case.api_base.as_deref(), case.use_ai_gateway) {
        Ok(base) => {
            let trimmed = base.trim_end_matches('/');
            let url = if case.use_ai_gateway {
                format!("{trimmed}/chat/completions")
            } else {
                format!("{trimmed}/responses")
            };
            json!({"url": url})
        }
        Err(error) => json!({"status": error.status_code}),
    }
}

pub(super) fn user_agent(cases: &[UserAgentCase]) -> serde_json::Value {
    let results: Vec<String> = cases
        .iter()
        .map(|case| build_user_agent(case.custom.as_deref(), DEFAULT_VERSION))
        .collect();
    json!(results)
}

pub(super) fn redaction(headers: &BTreeMap<String, String>, data: &Json) -> serde_json::Value {
    json!({
        "headers": redact_headers_for_logging(headers),
        "data": redact_sensitive_data(data),
    })
}

fn embedding_case(case: &EmbeddingCase, runtime: &Runtime) -> serde_json::Value {
    let key = case.api_key.clone().map(SecretString::from);
    let preparation = EmbeddingPreparation {
        optional_params: case.optional_params.clone(),
        api_key: key.as_ref(),
        api_base: case.api_base.as_deref(),
        custom_endpoint: case.custom_endpoint,
        headers: case.headers.clone(),
    };
    let tokens = ScriptedTokens::new(OauthReply::Reply {
        status: 200,
        body: SecretString::default(),
    });
    configure(
        runtime.block_on(prepare_embedding(preparation, &case.env.build(), &tokens)),
        |prepared| render_embedding(&prepared),
    )
}

fn render_embedding(prepared: &PreparedEmbedding) -> serde_json::Value {
    json!({
        "url": prepared.url,
        "headers": watched(&prepared.headers),
        "optional_params": prepared.optional_params,
    })
}

fn watched(headers: &Headers) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.to_lowercase(), value.clone()))
        .filter(|(name, _value)| WATCHED_HEADERS.contains(&name.as_str()))
        .collect()
}

pub(super) fn responses_url_values(env: &EnvInput, cases: &[UrlCase]) -> Vec<serde_json::Value> {
    let resolved = env.build();
    let config = DatabricksResponsesAPIConfig;
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let outcome = match config.complete_url(case.api_base.as_deref(), &resolved) {
            Ok(url) => json!({"url": url}),
            Err(error) => json!({"status": error.status_code}),
        };
        results.push(outcome);
    }
    results
}

pub(super) fn endpoints(
    env: &EnvInput,
    embedding_cases: &[EmbeddingCase],
    response_cases: &[UrlCase],
) -> HelperResult {
    let runtime = Builder::new_current_thread().build().or_config_error()?;
    let mut embedding_results = Vec::with_capacity(embedding_cases.len());
    for case in embedding_cases {
        embedding_results.push(embedding_case(case, &runtime));
    }
    Ok(json!({
        "embedding": embedding_results,
        "responses": responses_url_values(env, response_cases),
    }))
}
