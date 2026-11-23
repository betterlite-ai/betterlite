use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Mutex;

use betterlite_core::api::model_credentials::{
    CredentialSource, get_credentials_for_model, prepare_data_with_credentials,
};
use betterlite_models::controls::call_params::{TRUSTED_CREDENTIALS_KEY, TrustedCredentials};
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};

// serde_with's double_option needs Option<Option<T>>; the alias keeps clippy::option_option quiet as helper-api does
type OptionalObject = Option<JsonObject>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    // a missing router is LiteLLM's llm_router None, a null one is a router that answered None
    Lookup {
        model_id: String,
        operation_context: String,
        #[serde(default, with = "serde_with::rust::double_option")]
        router: Option<OptionalObject>,
    },
    Merge {
        data: JsonObject,
        credentials: JsonObject,
        #[serde(default)]
        file_id: Option<String>,
        include_internal_credentials: bool,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers = input.calls.iter().map(answer).collect::<Vec<_>>();
        Ok(Value::Array(answers))
    }
}

// what the lookup asked the router, so a case reads the port's call back
struct PortAsked {
    model_id: String,
    team_id: Option<String>,
}

struct ScriptedSource {
    answer: OptionalObject,
    asked: Mutex<Option<PortAsked>>,
}

impl CredentialSource for ScriptedSource {
    // the scripted router answers what the case says and cannot fail
    type Error = Infallible;

    fn credentials_with_provider(
        &self,
        model_id: &str,
        team_id: Option<&str>,
    ) -> Result<OptionalObject, Infallible> {
        let answer = self.answer.clone();
        let asked = PortAsked {
            model_id: model_id.to_owned(),
            team_id: team_id.map(str::to_owned),
        };
        let mut slot = match self.asked.lock() {
            Ok(slot) => slot,
            Err(poisoned) => poisoned.into_inner(),
        };
        *slot = Some(asked);
        drop(slot);
        Ok(answer)
    }
}

fn asked_view(asked: &Mutex<Option<PortAsked>>) -> Value {
    let taken = match asked.lock() {
        Ok(mut slot) => slot.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    };
    match taken {
        Some(record) => json!({"model_id": record.model_id, "team_id": record.team_id}),
        None => Value::Null,
    }
}

fn lookup_answer(
    model_id: &str,
    operation_context: &str,
    source: Option<&ScriptedSource>,
) -> Value {
    let result = get_credentials_for_model(source, model_id, operation_context);
    let (credentials, error) = match &result {
        Ok(found) => (json!(found), Value::Null),
        Err(failure) => (
            Value::Null,
            json!({"status": failure.status(), "detail": failure.detail_error()}),
        ),
    };
    let port = source.map_or(Value::Null, |scripted| asked_view(&scripted.asked));
    json!({
        "function": "lookup",
        "credentials": credentials,
        "port": port,
        "error": error,
    })
}

fn snapshot_view(snapshot: &TrustedCredentials) -> Value {
    let texts = snapshot
        .values
        .iter()
        .map(|(name, secret)| (name.as_str(), secret.expose_secret()))
        .collect::<BTreeMap<_, _>>();
    json!(texts)
}

fn merge_answer(
    data: &JsonObject,
    credentials: &JsonObject,
    file_id: Option<&str>,
    include_internal_credentials: bool,
) -> Value {
    let mut merged = data.clone();
    let snapshot = prepare_data_with_credentials(
        &mut merged,
        credentials,
        file_id,
        include_internal_credentials,
    );
    let internal_key = merged.contains_key(TRUSTED_CREDENTIALS_KEY);
    // the answer carries the merged data without the snapshot, which the adapter read back above
    merged.shift_remove(TRUSTED_CREDENTIALS_KEY);
    let snapshot_json = snapshot.as_ref().map_or(Value::Null, snapshot_view);
    json!({
        "function": "merge",
        "data": merged,
        "snapshot": snapshot_json,
        "internal_key": internal_key,
    })
}

fn answer(call: &Call) -> Value {
    match call {
        Call::Lookup {
            model_id,
            operation_context,
            router,
        } => {
            let source = router.as_ref().map(|configured| ScriptedSource {
                answer: configured.clone(),
                asked: Mutex::new(None),
            });
            lookup_answer(model_id, operation_context, source.as_ref())
        }
        Call::Merge {
            data,
            credentials,
            file_id,
            include_internal_credentials,
        } => merge_answer(
            data,
            credentials,
            file_id.as_deref(),
            *include_internal_credentials,
        ),
    }
}
