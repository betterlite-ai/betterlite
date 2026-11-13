mod extract;

use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::session_token_codec::Raw;
use betterlite_server::cli_login_pages::{
    ConsentPage, SUCCESS_PAGE, render_consent_page, render_verification_page,
};
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperResult};
use extract::{
    COMPLETION_FIELDS, VERIFICATION_FIELDS, classified, consent_entries, field_entries, object,
};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    environment: BTreeMap<String, String>,
    renders: Vec<Render>,
}

#[derive(Deserialize)]
#[serde(tag = "page", rename_all = "snake_case")]
pub(crate) enum Render {
    Verification(Verification),
    Consent(Consent),
    Completion,
}

#[derive(Deserialize)]
pub(crate) struct Verification {
    verify_url: String,
    browser_complete_token: String,
    prefill_user_code: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct Consent {
    client_origin: String,
    user_id: String,
    teams: Vec<Team>,
    flow_handle: String,
    complete_url: String,
}

#[derive(Deserialize)]
pub(crate) struct Team {
    id: String,
    label: String,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut facts = Vec::with_capacity(input.renders.len());
        for render in &input.renders {
            facts.push(facts_for(render, &input.environment)?);
        }
        Ok(Value::Array(facts))
    }
}

fn facts_for(render: &Render, environment: &BTreeMap<String, String>) -> HelperResult {
    match render {
        Render::Verification(request) => {
            let html = render_verification_page(
                &request.verify_url,
                &request.browser_complete_token,
                request.prefill_user_code.as_deref(),
            );
            Ok(object(field_entries(
                &html,
                "verification",
                VERIFICATION_FIELDS,
            )?))
        }
        Render::Consent(request) => consent_facts(environment, request),
        Render::Completion => Ok(object(field_entries(
            SUCCESS_PAGE,
            "completion",
            COMPLETION_FIELDS,
        )?)),
    }
}

fn consent_facts(environment: &BTreeMap<String, String>, request: &Consent) -> HelperResult {
    let settings = Raw::default()
        .convert(environment)
        .map_err(|error| classified("ConfigError", &error.to_string()))?;
    let pairs: Vec<(String, String)> = request
        .teams
        .iter()
        .map(|team| (team.id.clone(), team.label.clone()))
        .collect();
    let html = render_consent_page(ConsentPage {
        client_origin: request.client_origin.as_str(),
        user_id: request.user_id.as_str(),
        teams: &pairs,
        flow_handle: request.flow_handle.as_str(),
        complete_url: request.complete_url.as_str(),
        cli_expiration_hours: settings.cli_expiration_hours,
    });
    Ok(object(consent_entries(&html)?))
}
