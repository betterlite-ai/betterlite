use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::prompt_contracts::PromptManagementClient;
use betterlite_models::prompt_spec::{PromptLiteLLMParams, PromptSpec};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::bitbucket::initializer::prompt_initializer;
use betterlite_prompts::bitbucket::manager::BitBucketPromptManager;
use betterlite_prompts::bitbucket::messages::parse_prompt_to_messages;
use betterlite_prompts::bitbucket_prompt_manager::parse::{YamlParser, parse_prompt_file};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    mode: Mode,
    #[serde(default)]
    prompt_id: Option<String>,
    #[serde(default)]
    variables: Object,
    #[serde(default)]
    files: BTreeMap<String, String>,
    #[serde(default)]
    template: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Parse,
    Compile,
    GetTemplate,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("RuntimeError", &error.to_string()))?;
        runtime.block_on(execute(input))
    }
}

async fn execute(input: &Input) -> HelperResult {
    match input.mode {
        Mode::Parse => parse(input),
        Mode::Compile => compile(input).await,
        Mode::GetTemplate => get_template(input).await,
    }
}

fn parse(input: &Input) -> HelperResult {
    let Some(template) = input.template.as_deref() else {
        return Err(failure("ValueError", "template is required for parse"));
    };
    Ok(json!({ "messages": parse_prompt_to_messages(template) }))
}

impl Input {
    fn required_prompt_id(&self) -> Result<&str, Box<HelperError>> {
        self.prompt_id
            .as_deref()
            .ok_or_else(|| failure("ValueError", "prompt_id is required"))
    }
}

async fn prepared(input: &Input) -> Result<(BitBucketPromptManager, &str), Box<HelperError>> {
    let prompt_id = input.required_prompt_id()?;
    let manager = build_manager(input).await?;
    Ok((manager, prompt_id))
}

async fn compile(input: &Input) -> HelperResult {
    let (manager, prompt_id) = prepared(input).await?;
    let client = manager
        .compile_prompt_helper(Some(prompt_id), Some(&input.variables), None, None)
        .await
        .map_err(|error| failure("ValueError", &error.to_string()))?;
    Ok(client_json(&client))
}

fn client_json(client: &PromptManagementClient) -> Value {
    json!({
        "prompt_id": client.prompt_id,
        "messages": client.prompt_template,
        "model": client.prompt_template_model,
        "optional_params": client.prompt_optional_params,
        "completed_messages": client.completed_messages,
    })
}

async fn get_template(input: &Input) -> HelperResult {
    let (manager, prompt_id) = prepared(input).await?;
    let (rendered, metadata) = manager
        .get_prompt_template(prompt_id, Some(&input.variables))
        .await
        .map_err(|error| failure("ValueError", &error.to_string()))?;
    Ok(json!({ "rendered": rendered, "metadata": metadata }))
}

async fn build_manager(input: &Input) -> Result<BitBucketPromptManager, Box<HelperError>> {
    let params = params()?;
    let spec = prompt_spec()?;
    let manager = prompt_initializer(&params, &spec, transport()?)
        .map_err(|error| failure("ValueError", &error.to_string()))?;
    let mut templates = manager
        .prompt_manager()
        .await
        .map_err(|error| failure("ValueError", &error.to_string()))?;
    for (prompt_id, content) in &input.files {
        let template = parse_prompt_file(content, prompt_id, YamlParser::Full)
            .map_err(|error| failure("ValueError", &error.to_string()))?;
        templates.prompts.insert(prompt_id.clone(), template);
    }
    drop(templates);
    Ok(manager)
}

fn params() -> Result<PromptLiteLLMParams, Box<HelperError>> {
    serde_json::from_value(json!({
        "prompt_integration": "bitbucket",
        "bitbucket_config": {
            "workspace": "ws",
            "repository": "repo",
            "access_token": "token",
            "": "http://127.0.0.1:1/2.0",
        },
    }))
    .map_err(|error| failure("ValueError", &error.to_string()))
}

fn prompt_spec() -> Result<PromptSpec, Box<HelperError>> {
    serde_json::from_value(json!({
        "prompt_id": "case",
        "litellm_params": { "prompt_integration": "bitbucket" },
    }))
    .map_err(|error| failure("ValueError", &error.to_string()))
}

fn transport() -> Result<ProxiedClient, Box<HelperError>> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .map_err(|error| failure("OSError", &error.to_string()))?;
    ProxiedClient::outbound(&Arc::new(policy))
        .map_err(|error| failure("OSError", &error.to_string()))
}

fn failure(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
