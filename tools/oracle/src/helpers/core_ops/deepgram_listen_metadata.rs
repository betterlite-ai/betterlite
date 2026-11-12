use betterlite_core::api::deepgram::listen_metadata::frames::{
    ListenFrame, audio_seconds, channel_count, transcript,
};
use betterlite_core::api::deepgram::listen_metadata::pricing::{
    addon_pricing_models, is_priced, registry_key,
};
use betterlite_core::api::deepgram::listen_metadata::query::{ListenQuery, decode_listen_query};
use betterlite_core::api::deepgram::listen_metadata::target::{
    ListenTarget, parse_listen_target, websocket_target,
};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde_json::{Value, json};

type Failure = Box<crate::helpers::HelperError>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    query: Option<String>,
    api_base: Option<String>,
    upstream_url: Option<String>,
    messages: Option<Vec<ListenFrame>>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    QueryIdentity,
    BlankModel,
    Callbacks,
    ExactStreamingPrice,
    AddonFlags,
    ChannelPrecedence,
    DurationPrecedence,
    FinalTranscript,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut bundled = BundledCatalog::default();
        let catalog = bundled.get();
        let outcomes = input
            .calls
            .iter()
            .map(|call| run_call(call, &catalog))
            .collect::<Result<Vec<_>, Failure>>()?;
        Ok(Value::Array(outcomes))
    }
}

fn run_call(call: &Call, catalog: &ModelCatalog) -> Result<Value, Failure> {
    match call.function {
        Function::QueryIdentity | Function::BlankModel => {
            let query = decode(call)?;
            let built = websocket_target(call.api_base.as_deref(), &query).map_err(failure)?;
            Ok(Value::String(built))
        }
        Function::Callbacks => Ok(json!(decode(call)?.callback_params())),
        Function::ExactStreamingPrice => {
            let target = upstream(call)?;
            Ok(json!({
                "registry_key": registry_key(&target),
                "priced": is_priced(catalog, &target),
            }))
        }
        Function::AddonFlags => Ok(json!(addon_pricing_models(&upstream(call)?))),
        Function::ChannelPrecedence => {
            let target = upstream(call)?;
            let declared = target.query.declared_channels();
            Ok(json!(channel_count(messages(call)?, declared)))
        }
        Function::DurationPrecedence => {
            let seconds = audio_seconds(messages(call)?);
            Ok(json!(seconds.to_f64()))
        }
        Function::FinalTranscript => Ok(Value::String(transcript(messages(call)?))),
    }
}

fn decode(call: &Call) -> Result<ListenQuery, Failure> {
    let query = required(call.query.as_ref(), "query")?;
    let decoded = decode_listen_query(query.as_bytes());
    Ok(decoded)
}

fn upstream(call: &Call) -> Result<ListenTarget, Failure> {
    let url = required(call.upstream_url.as_ref(), "upstream_url")?;
    let parsed = parse_listen_target(url).map_err(failure)?;
    Ok(parsed)
}

fn messages(call: &Call) -> Result<&[ListenFrame], Failure> {
    let Some(frames) = call.messages.as_deref() else {
        return Err(failure("messages is required"));
    };
    Ok(frames)
}

fn required<'call>(value: Option<&'call String>, name: &str) -> Result<&'call str, Failure> {
    let Some(text) = value else {
        return Err(failure(format!("{name} is required")));
    };
    Ok(text.as_str())
}

fn failure<Error>(error: Error) -> Failure
where
    Error: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
