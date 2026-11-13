use betterlite_guardrails::pillar::bool_config::{BoolSetting, resolve_bool_config};
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

mod protect;

use protect::BoolCase;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ResolveBool(BoolCase),
    Protect(Box<protect::ProtectInput>),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ResolveBool(case) => resolve_bool(case),
            Input::Protect(case) => protect::run(case),
        }
    }
}

pub(super) fn fail(class: &str, message: String, status: Option<u16>) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn resolve_bool(case: &BoolCase) -> HelperResult {
    let (env_var, default) = case.setting.config();
    let provided = case.provided.map(BoolSetting::Boolean);
    let environment = case.environment.clone();
    let value = resolve_bool_config(
        provided.as_ref(),
        Some(env_var),
        default,
        &environment,
        env_var,
    );
    Ok(json!({"value": value}))
}
