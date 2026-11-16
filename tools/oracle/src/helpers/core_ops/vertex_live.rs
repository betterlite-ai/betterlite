mod tests;
mod warnings;

use std::collections::BTreeMap;

use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_vertex::gemini_live_client::transform::audio_mime_type;
use betterlite_vertex::vertex_live::client::VertexLiveClient;
use betterlite_vertex::vertex_live::policy::VertexLivePolicy;
use betterlite_vertex::vertex_live::session::VertexLiveInput;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use warnings::Warnings;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    project: String,
    location: String,
    access_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Endpoint {
        session: Session,
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    Mime {
        session: Session,
        formats: Vec<String>,
    },
    Automatic {
        session: Session,
        model: String,
    },
    Transform {
        session: Session,
        model: String,
        message: Value,
        #[serde(default)]
        session_configuration_request: Option<String>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut bundled = BundledCatalog::default();
        let catalog = bundled.get();
        let schema = SchemaSettings::default();
        let calls = input
            .calls
            .iter()
            .map(|call| answer(call, &catalog, &schema))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "calls": calls }))
    }
}

struct Live<'live> {
    policy: &'live VertexLivePolicy,
    catalog: &'live ModelCatalog,
    schema: &'live SchemaSettings,
}

fn answer(
    call: &Call,
    catalog: &ModelCatalog,
    schema: &SchemaSettings,
) -> Result<Value, Box<HelperError>> {
    let session = call.session();
    let policy = VertexLivePolicy {
        project: session.project.clone(),
        region: session.location.clone(),
        access_token: SecretString::from(session.access_token.clone()),
    };
    let warnings = Warnings::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    let live = Live {
        policy: &policy,
        catalog,
        schema,
    };
    let value = with_default(subscriber, || dispatch(call, &live))?;
    Ok(json!({"value": value, "warnings": warnings.labels()}))
}

fn dispatch(call: &Call, live: &Live<'_>) -> Result<Value, Box<HelperError>> {
    match call {
        Call::Mime { formats, .. } => Ok(json!({
            "mimes": formats.iter().map(String::as_str).map(audio_mime_type).collect::<Vec<_>>(),
        })),
        Call::Endpoint {
            api_base,
            api_key,
            headers,
            ..
        } => Ok(endpoint(
            live.policy,
            api_base.as_deref(),
            api_key.as_deref(),
            headers,
        )),
        Call::Automatic { model, .. } => automatic(live.policy, model),
        Call::Transform {
            model,
            message,
            session_configuration_request,
            ..
        } => {
            let input = VertexLiveInput {
                policy: live.policy,
                catalog: live.catalog,
                schema: live.schema,
                model: model.as_str(),
                session_configuration_request: session_configuration_request.as_deref(),
            };
            transform(&input, message)
        }
    }
}

fn endpoint(
    policy: &VertexLivePolicy,
    api_base: Option<&str>,
    api_key: Option<&str>,
    headers: &BTreeMap<String, String>,
) -> Value {
    let environment = policy.environment(headers);
    let api_key_used =
        api_key.is_some_and(|key| environment.values().any(|value| value.contains(key)));
    json!({
        "url": policy.complete_url(api_base),
        "environment": environment,
        "api_key_used": api_key_used,
    })
}

fn automatic(policy: &VertexLivePolicy, model: &str) -> Result<Value, Box<HelperError>> {
    let request = policy
        .session_configuration_request(model)
        .or_value_error()?;
    let setup = serde_json::from_str::<Value>(&request)
        .or_value_error()?
        .get("setup")
        .cloned()
        .unwrap_or_default();
    Ok(json!({"setup": setup}))
}

fn transform(input: &VertexLiveInput<'_>, message: &Value) -> Result<Value, Box<HelperError>> {
    let encoded = serde_json::to_vec(message).or_value_error()?;
    let mut client = VertexLiveClient::default();
    let messages = client
        .transform_realtime_request(&encoded, usize::MAX, input)
        .or_value_error()?;
    let parsed = messages
        .iter()
        .map(|raw| serde_json::from_str::<Value>(raw).or_value_error())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"messages": parsed}))
}

impl Call {
    const fn session(&self) -> &Session {
        match self {
            Self::Endpoint { session, .. }
            | Self::Mime { session, .. }
            | Self::Automatic { session, .. }
            | Self::Transform { session, .. } => session,
        }
    }
}
