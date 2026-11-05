#![cfg(test)]

use std::future::{Future, ready};
use std::time::Duration;

use indexmap::IndexMap;

use super::call::Primitives;
use super::error::PrimitiveError;
use super::http::transport::{
    HttpMethod, HttpReply, OutboundHttp, OutboundRequest, RequestBody, TransportFailure,
    ValidatedDestination,
};
use super::json_tools::json_stringify;
use super::json_value::JsonValue;
use super::registry::{RegistryEntry, get_custom_code_primitives};

mod behaviors;
mod custom_code_async;
mod detection;
mod errors;
mod registry;
mod text;
mod valid;

#[derive(Clone)]
enum Outcome {
    Echo,
    Reply(HttpReply),
    Fail(TransportFailure),
}

// Answers every request with a JSON description of what the transport was asked to send.
struct FakeTransport {
    validating: bool,
    outcome: Outcome,
}

impl Default for FakeTransport {
    fn default() -> Self {
        Self {
            validating: false,
            outcome: Outcome::Echo,
        }
    }
}

impl FakeTransport {
    fn respond(&self, via: &str, request: &OutboundRequest) -> Result<HttpReply, TransportFailure> {
        match &self.outcome {
            Outcome::Reply(reply) => return Ok(reply.clone()),
            Outcome::Fail(failure) => return Err(failure.clone()),
            Outcome::Echo => {}
        }
        let method = match request.method {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Patch => "PATCH",
        };
        let body = match &request.body {
            RequestBody::Empty => JsonValue::Null,
            RequestBody::Json(fields) => JsonValue::Object(fields.clone()),
            RequestBody::Text(text) => JsonValue::String(text.clone()),
        };
        let headers = request
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), JsonValue::String(value.clone())))
            .collect();
        let mut echo = IndexMap::new();
        echo.insert("via".to_owned(), JsonValue::String(via.to_owned()));
        echo.insert("method".to_owned(), JsonValue::String(method.to_owned()));
        echo.insert("url".to_owned(), JsonValue::String(request.url.clone()));
        echo.insert("headers".to_owned(), JsonValue::Object(headers));
        echo.insert("body".to_owned(), body);
        let millis = u64::try_from(request.timeout.as_millis()).unwrap_or_default();
        echo.insert(
            "timeout_ms".to_owned(),
            JsonValue::Number(serde_json::Number::from(millis)),
        );
        Ok(HttpReply {
            status: 200,
            headers: IndexMap::new(),
            body: json_stringify(&JsonValue::Object(echo)),
        })
    }
}

impl OutboundHttp for FakeTransport {
    fn user_url_validation(&self) -> bool {
        self.validating
    }

    fn validate_url(
        &self,
        url: &str,
    ) -> impl Future<Output = Result<ValidatedDestination, TransportFailure>> + Send {
        ready(match &self.outcome {
            Outcome::Fail(failure) => Err(failure.clone()),
            Outcome::Echo | Outcome::Reply(_) => Ok(ValidatedDestination {
                url: url.replacen("example.com", "93.184.216.34", 1),
                host_header: "example.com".to_owned(),
            }),
        })
    }

    fn safe_get(
        &self,
        url: &str,
        headers: Vec<(String, String)>,
        timeout: Duration,
    ) -> impl Future<Output = Result<HttpReply, TransportFailure>> + Send {
        let request = OutboundRequest {
            method: HttpMethod::Get,
            url: url.to_owned(),
            headers,
            body: RequestBody::Empty,
            timeout,
        };
        ready(self.respond("safe_get", &request))
    }

    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<HttpReply, TransportFailure>> + Send {
        ready(self.respond("send", &request))
    }
}

// Calls a registered primitive the way custom code does and returns the JSON text of its result.
async fn run(
    transport: FakeTransport,
    name: &str,
    positional: &str,
    named: &str,
) -> Result<String, PrimitiveError> {
    let registry = get_custom_code_primitives();
    let Some(RegistryEntry::Function(primitive)) = registry.get(name).copied() else {
        panic!("{name} is not a registered function");
    };
    let positional_values: Vec<JsonValue> =
        serde_json::from_str(positional).expect("positional arguments are a JSON array");
    let named_values: IndexMap<String, JsonValue> =
        serde_json::from_str(named).expect("named arguments are a JSON object");
    let output = Primitives::new(transport)?
        .call(primitive, positional_values, named_values)
        .await?;
    Ok(serde_json::to_string(&output).expect("a primitive result encodes"))
}
