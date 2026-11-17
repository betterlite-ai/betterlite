use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

const HIDDEN_KEY: &str = "_hidden_params";
const ADDITIONAL_HEADERS: &str = "additional_headers";
const MODEL_ID: &str = "model_id";
const MODEL_INFO: &str = "model_info";

// What a caller hands the router to attach headers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    // LiteLLM passes None through untouched
    Absent,
    // a dict response, whose hidden params sit under its `_hidden_params` key
    Object(JsonObject),
    // an object that already carries a `_hidden_params` attribute
    Attributed(JsonObject),
    // a bare async iterator, which HiddenParamsAsyncIteratorWrapper gives a hidden-params attribute
    Stream,
    // any other object: LiteLLM returns it unchanged, and a write to it goes nowhere
    Opaque,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Carrier {
    Dict,
    Attribute,
    Wrapped,
    Opaque,
}

// The hidden params one response exposes, in the fields the router stamps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    carrier: Carrier,
    // the dict body, kept so a write-back returns every key the caller sent
    body: JsonObject,
    had_hidden: bool,
    headers_present: bool,
    model_id: Option<String>,
    additional_headers: AdditionalHeaders,
    // the hidden keys this module does not model, kept so a write-back does not drop them
    other: JsonObject,
}

impl Prepared {
    fn from_hidden(
        carrier: Carrier,
        body: JsonObject,
        hidden: JsonObject,
        had_hidden: bool,
    ) -> Self {
        let mut prepared = Self {
            carrier,
            body,
            had_hidden,
            headers_present: false,
            model_id: None,
            additional_headers: AdditionalHeaders::new(),
            other: JsonObject::new(),
        };
        for (key, value) in hidden {
            match (key.as_str(), value) {
                (MODEL_ID, Json::Text(id)) if !id.is_empty() => prepared.model_id = Some(id),
                (ADDITIONAL_HEADERS, Json::Object(headers)) => {
                    prepared.additional_headers = headers;
                    prepared.headers_present = true;
                }
                // a falsy model_id or non-object header map stays until the router stamps over it
                (_, kept) => {
                    prepared.other.insert(key, kept);
                }
            }
        }
        prepared
    }

    // a write reaches a dict or an object that has the attribute, and nothing else
    fn attach(&mut self) {
        if self.carrier != Carrier::Opaque {
            self.had_hidden = true;
        }
    }

    #[must_use]
    pub fn hidden_params(&self) -> JsonObject {
        let mut hidden = self.other.clone();
        if let Some(model_id) = &self.model_id {
            hidden.insert(MODEL_ID.to_owned(), Json::Text(model_id.clone()));
        }
        if self.headers_present {
            hidden.insert(
                ADDITIONAL_HEADERS.to_owned(),
                Json::Object(self.additional_headers.clone()),
            );
        }
        hidden
    }
}

// prepare_response_for_header_attachment: a bare async iterator comes back wrapped, everything else as it is
#[must_use]
pub fn prepare_response(response: Response) -> Option<Prepared> {
    match response {
        Response::Absent => None,
        Response::Object(body) => {
            let had_hidden = body.contains_key(HIDDEN_KEY);
            let hidden = match body.get(HIDDEN_KEY) {
                Some(Json::Object(hidden)) => hidden.clone(),
                None
                | Some(
                    Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
                ) => JsonObject::new(),
            };
            Some(Prepared::from_hidden(
                Carrier::Dict,
                body,
                hidden,
                had_hidden,
            ))
        }
        Response::Attributed(hidden) => Some(Prepared::from_hidden(
            Carrier::Attribute,
            JsonObject::new(),
            hidden,
            true,
        )),
        // HiddenParamsAsyncIteratorWrapper.__init__ starts the wrapper with an empty map
        Response::Stream => Some(Prepared::from_hidden(
            Carrier::Wrapped,
            JsonObject::new(),
            JsonObject::new(),
            true,
        )),
        Response::Opaque => Some(Prepared::from_hidden(
            Carrier::Opaque,
            JsonObject::new(),
            JsonObject::new(),
            false,
        )),
    }
}

// response_has_hidden_params: whether the response carries hidden params now.
#[must_use]
pub const fn response_has_hidden_params(prepared: &Prepared) -> bool {
    match prepared.carrier {
        Carrier::Dict => prepared.had_hidden,
        Carrier::Attribute | Carrier::Wrapped => true,
        Carrier::Opaque => false,
    }
}

// ensure_response_additional_headers: attach hidden params where the response holds them, then answer the map
pub fn ensure_additional_headers(prepared: &mut Prepared) -> &mut AdditionalHeaders {
    prepared.attach();
    prepared.headers_present = true;
    &mut prepared.additional_headers
}

// apply_response_model_id: the deployment's model id, when the request carries one and nothing set it yet.
pub fn apply_response_model_id(prepared: &mut Prepared, request_metadata: Option<&JsonObject>) {
    let Some(model_id) = metadata_model_id(request_metadata) else {
        return;
    };
    if prepared.model_id.is_some() {
        return;
    }
    prepared.model_id = Some(model_id.to_owned());
    prepared.attach();
}

fn metadata_model_id(request_metadata: Option<&JsonObject>) -> Option<&str> {
    let metadata = request_metadata?;
    let Some(Json::Object(info)) = metadata.get(MODEL_INFO) else {
        return None;
    };
    match info.get("id") {
        Some(Json::Text(id)) if !id.is_empty() => Some(id.as_str()),
        // an empty id names no deployment, and the guarded arm above never covers it
        None
        | Some(
            Json::Text(_)
            | Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_),
        ) => None,
    }
}

// _ensure_additional_headers_dict: the header map already in the hidden params, or an empty one to fill.
#[must_use]
pub fn ensure_additional_headers_dict(hidden: &JsonObject) -> AdditionalHeaders {
    match hidden.get(ADDITIONAL_HEADERS) {
        Some(Json::Object(headers)) => headers.clone(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            AdditionalHeaders::new()
        }
    }
}

// _write_hidden_params: return the response the caller passed in, now carrying the hidden params.
#[must_use]
pub fn write_hidden_params(prepared: Prepared) -> Response {
    let hidden = prepared.hidden_params();
    match prepared.carrier {
        Carrier::Dict => {
            let mut body = prepared.body;
            body.insert(HIDDEN_KEY.to_owned(), Json::Object(hidden));
            Response::Object(body)
        }
        Carrier::Attribute => Response::Attributed(hidden),
        Carrier::Wrapped => Response::Stream,
        Carrier::Opaque => Response::Opaque,
    }
}

// response_total_token_count: usage total_tokens, else the sum of the input and output counts.
#[must_use]
pub fn response_total_token_count(response: &JsonObject) -> u64 {
    let Some(Json::Object(usage)) = response.get("usage") else {
        return 0;
    };
    let total = token_count(usage.get("total_tokens"));
    if total != 0 {
        return total;
    }
    token_count(usage.get("input_tokens")).saturating_add(token_count(usage.get("output_tokens")))
}

fn token_count(value: Option<&Json>) -> u64 {
    match value {
        Some(Json::Number(number)) => number.as_u64().unwrap_or(0),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => 0,
    }
}
