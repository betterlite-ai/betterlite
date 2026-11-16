#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::{
    Guardrail, GuardrailDefinitionLocation, GuardrailEventHooks,
};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_redis::config_sync::publish::{PublishChange, redis_config_change_publisher};
use chrono::{DateTime, NaiveDateTime, Utc};
use secrecy::ExposeSecret;

use super::build::GuardrailFactories;
use super::callback::{CallbackScoping, CallbackSpec, GuardrailCallback};
use super::changed::params_changed;
use super::error::RegistryError;
use super::handler::{HandlerSettings, InMemoryGuardrailHandler};
use super::integrations::{CompressionKind, compression_kind};
use super::postgres_rows::{self, PostgresGuardrailRows};
use super::registry::GuardrailRegistry;
use super::rows::{
    GuardrailRow, GuardrailRows, GuardrailUpdate, NewGuardrailRow, RowsError, guardrail_to_new_row,
    guardrail_to_update, row_to_guardrail,
};
use super::scoping::configure_scoping;
use super::stable_id::stable_guardrail_id;
use crate::guardrail_initializers::error::InitializerError;
use crate::guardrail_initializers::presidio::PresidioRole;
use crate::tool_policy::{RequestToolNames, ToolPolicyLookup};

const BEDROCK_PARAMS: &str = r#"{"guardrail":"bedrock","mode":"pre_call"}"#;
const PRESIDIO_PARAMS: &str =
    r#"{"guardrail":"presidio","mode":"pre_call","output_parse_pii":true}"#;
const PII_GUARD_ID: &str = "fcb4375e-8649-5343-8c8f-9e59925ae70b";
const PII_GUARD_ID_1: &str = "7ab7011f-602f-5459-8311-adc11c2b2c7a";
const PII_GUARD_ID_2: &str = "ae350a95-946e-5879-902e-fed29443419e";

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

fn guardrail(id: Option<&str>, name: &str, params_json: &str) -> Guardrail {
    Guardrail {
        guardrail_id: id.map(str::to_owned),
        guardrail_name: name.to_owned(),
        litellm_params: params(params_json),
        guardrail_info: None,
        policy_template: None,
        created_at: None,
        updated_at: None,
        team_id: None,
    }
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

fn naive(text: &str) -> Timestamp {
    Timestamp::Naive(NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S").unwrap())
}

fn row(id: &str, name: &str) -> GuardrailRow {
    GuardrailRow {
        guardrail_id: id.to_owned(),
        guardrail_name: name.to_owned(),
        litellm_params: Some(BEDROCK_PARAMS.to_owned()),
        guardrail_info: Some("{}".to_owned()),
        team_id: None,
        status: "active".to_owned(),
        created_at: naive("2026-01-02T03:04:05"),
        updated_at: naive("2026-01-02T03:04:05"),
    }
}

struct NoPolicies;

impl ToolPolicyLookup for NoPolicies {
    fn is_initialized(&self) -> bool {
        false
    }

    fn effective_policies(
        &self,
        _tool_names: &[String],
        _key_permission_id: Option<&str>,
        _team_permission_id: Option<&str>,
    ) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn output_policy(&self, _tool_name: &str) -> String {
        String::new()
    }
}

struct NoNames;

impl RequestToolNames for NoNames {
    fn names(&self, _route: &str, _request_data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
        Vec::new()
    }
}

fn factories() -> GuardrailFactories {
    GuardrailFactories::new(Arc::new(NoPolicies), Arc::new(NoNames))
}

fn build_one(kind: &str, params_json: &str, name: &str) -> GuardrailCallback {
    let mut callbacks = factories()
        .build(kind, &params(params_json), name, true)
        .unwrap();
    callbacks.remove(0)
}

fn handler() -> InMemoryGuardrailHandler {
    handler_with(
        BTreeMap::new(),
        HandlerSettings {
            strict_modes: true,
            global_skip_tool_message: false,
        },
    )
}

fn handler_with(
    env: BTreeMap<String, String>,
    settings: HandlerSettings,
) -> InMemoryGuardrailHandler {
    InMemoryGuardrailHandler::new(factories(), Box::new(env), settings)
}

include!("tests/stable_id.rs");
include!("tests/build.rs");
include!("tests/scoping.rs");
include!("tests/rows.rs");
include!("tests/postgres.rs");
include!("tests/registry.rs");
include!("tests/handler.rs");
include!("tests/changed.rs");
