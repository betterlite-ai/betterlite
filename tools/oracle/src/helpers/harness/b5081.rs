use std::collections::BTreeMap;

use betterlite_harness::context::{SessionContext, SessionSettings, TurnOutcome};
use betterlite_harness::errors::HarnessError;
use betterlite_harness::options::{CodexOptions, HarnessOptions, OpenCodeOptions};
use betterlite_harness::providers::codex::transformation::CodexHarnessConfig;
use betterlite_harness::providers::opencode::transformation::OpenCodeHarnessConfig;
use betterlite_harness::transformation::BaseHarnessConfig as _;
use betterlite_harness::types::{Harness, JsonObject, PermissionMode};
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    harness: Harness,
    config: JsonObject,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.harness {
            Harness::Codex => validate_codex(&input.config),
            Harness::OpenCode => validate_opencode(&input.config),
            Harness::ClaudeCode | Harness::DeepAgents => Err(Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "b5081 harness must be codex or opencode".to_owned(),
            })),
        }
        .map(|()| json!({"valid": true}))
    }
}

fn session_settings(options: HarnessOptions) -> SessionSettings {
    SessionSettings {
        model: None,
        gateway: None,
        api_key: None,
        api_base: None,
        instructions: None,
        skills: Vec::new(),
        disable_tools: Vec::new(),
        permissions: PermissionMode::Full,
        on_approval: None,
        output_schema: None,
        max_turns: None,
        timeout: None,
        metadata: JsonObject::new(),
        options: Some(options),
    }
}

fn validate_codex(config: &JsonObject) -> Result<(), Box<HelperError>> {
    let context = SessionContext {
        harness: Harness::Codex,
        sandbox: (),
        session_id: "b5081-session".to_owned(),
        settings: session_settings(HarnessOptions::Codex(CodexOptions {
            reasoning_effort: None,
            web_search: false,
            config: config.clone(),
            env: BTreeMap::new(),
        })),
        outcome: TurnOutcome::default(),
    };
    CodexHarnessConfig
        .validate_environment(&context)
        .map_err(report)
}

fn validate_opencode(config: &JsonObject) -> Result<(), Box<HelperError>> {
    let context = SessionContext {
        harness: Harness::OpenCode,
        sandbox: (),
        session_id: "b5081-session".to_owned(),
        settings: session_settings(HarnessOptions::OpenCode(OpenCodeOptions {
            agent: "build".to_owned(),
            config: config.clone(),
            env: BTreeMap::new(),
        })),
        outcome: TurnOutcome::default(),
    };
    OpenCodeHarnessConfig
        .validate_environment(&context)
        .map_err(report)
}

fn report(error: HarnessError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "OptionsMismatch".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
