mod tests;

use betterlite_logging::spend_bodies::session::SpendTimestamp;
use betterlite_logging::spend_event::build::build_spend_event;
use betterlite_logging::spend_event::decode::decode_spend_event;
use betterlite_logging::spend_event::event::Mapping;
use betterlite_logging::spend_event::input::{
    BuildInput, CachePresence, ResponseBody, ResponseObject, TypedAuth, TypedAuths,
};
use betterlite_logging::spend_event::restore::callback_args;
use betterlite_models::decisions::Json;
use betterlite_prompts::response_tool_calls::projections::ToolCallResponse;
use betterlite_prompts::tool_spend::response_tool_call_names;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Build(Box<BuildSpec>),
    Offloadable { kind: Kind },
    Decode { line: String },
    Restore { line: String },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Absent,
    Model,
    Mapping,
    Batch,
    Interactions,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ResponseSpec {
    Model { fields: Mapping },
    Mapping { fields: Mapping },
    Absent,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthSpec {
    budget_reservation: Option<Mapping>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TypedAuthSpec {
    metadata: Option<AuthSpec>,
    litellm_metadata: Option<AuthSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BuildSpec {
    kwargs: Mapping,
    litellm_params: Mapping,
    metadata: Option<Mapping>,
    litellm_metadata: Option<Mapping>,
    typed_auth: TypedAuthSpec,
    response: ResponseSpec,
    start_time: SpendTimestamp,
    end_time: SpendTimestamp,
    store_bodies: bool,
    cache: CachePresence,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Build(spec) => build(spec),
            Input::Offloadable { kind } => Ok(json!({ "offloadable": offloadable(*kind) })),
            Input::Decode { line } => decode(line),
            Input::Restore { line } => restore(line),
        }
    }
}

fn invalid(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn offloadable(kind: Kind) -> bool {
    let body = ResponseBody::default();
    let response = match kind {
        Kind::Absent => ResponseObject::Absent,
        Kind::Model => ResponseObject::Model(body),
        Kind::Mapping => ResponseObject::Mapping(body),
        Kind::Batch => ResponseObject::Batch(body),
        Kind::Interactions => ResponseObject::Interactions(body),
    };
    response.is_offloadable()
}

// The collector names tools with the B1706 extractor, which this crate runs on the response fields
fn response_body(fields: &Mapping) -> Result<ResponseBody, Box<crate::helpers::HelperError>> {
    let text = serde_json::to_string(fields).map_err(|error| invalid(&error.to_string()))?;
    let projection = serde_json::from_str::<ToolCallResponse>(&text)
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(ResponseBody {
        fields: fields.clone(),
        tool_call_names: response_tool_call_names(&projection),
    })
}

fn response_object(
    spec: &ResponseSpec,
) -> Result<ResponseObject, Box<crate::helpers::HelperError>> {
    match spec {
        ResponseSpec::Model { fields } => Ok(ResponseObject::Model(response_body(fields)?)),
        ResponseSpec::Mapping { fields } => Ok(ResponseObject::Mapping(response_body(fields)?)),
        ResponseSpec::Absent => Ok(ResponseObject::Absent),
    }
}

fn typed_auth(spec: Option<&AuthSpec>) -> Option<TypedAuth> {
    spec.map(|auth| TypedAuth {
        budget_reservation: auth.budget_reservation.clone(),
    })
}

fn parse(line: &[u8]) -> Result<Value, Box<crate::helpers::HelperError>> {
    serde_json::from_slice(line).map_err(|error| invalid(&error.to_string()))
}

fn build(spec: &BuildSpec) -> crate::helpers::HelperResult {
    let mut params = spec.litellm_params.clone();
    params.insert(
        "metadata".to_owned(),
        spec.metadata.clone().map_or(Json::Null, Json::Object),
    );
    params.insert(
        "litellm_metadata".to_owned(),
        spec.litellm_metadata
            .clone()
            .map_or(Json::Null, Json::Object),
    );
    let mut kwargs = spec.kwargs.clone();
    kwargs.insert("litellm_params".to_owned(), Json::Object(params));
    let response = response_object(&spec.response)?;
    let typed = TypedAuths {
        metadata: typed_auth(spec.typed_auth.metadata.as_ref()),
        litellm_metadata: typed_auth(spec.typed_auth.litellm_metadata.as_ref()),
    };
    let built = build_spend_event(&BuildInput {
        kwargs: &kwargs,
        typed_auth: &typed,
        response: &response,
        start_time: spec.start_time,
        end_time: spec.end_time,
        store_bodies: spec.store_bodies,
        cache: spec.cache,
    });
    match built {
        Err(_) => Ok(json!({ "build_error": true })),
        Ok(line) => Ok(json!({
            "offloadable": response.is_offloadable(),
            "newline_terminated": line.ends_with(b"\n"),
            "event": parse(&line)?,
        })),
    }
}

fn decode(line: &str) -> crate::helpers::HelperResult {
    match decode_spend_event(line.as_bytes()) {
        Err(_) => Ok(json!({ "decode_error": true })),
        Ok(event) => {
            let text = serde_json::to_vec(&event).map_err(|error| invalid(&error.to_string()))?;
            Ok(json!({ "event": parse(&text)? }))
        }
    }
}

fn restore(line: &str) -> crate::helpers::HelperResult {
    let Ok(event) = decode_spend_event(line.as_bytes()) else {
        return Ok(json!({ "decode_error": true }));
    };
    match callback_args(event) {
        Err(_) => Ok(json!({ "restore_error": true })),
        Ok(args) => {
            let text = serde_json::to_vec(&args).map_err(|error| invalid(&error.to_string()))?;
            Ok(json!({ "args": parse(&text)? }))
        }
    }
}
