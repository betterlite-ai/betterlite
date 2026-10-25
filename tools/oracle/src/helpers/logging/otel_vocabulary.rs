use betterlite_logging::otel::vocabulary::constants::MCPMethod;
use betterlite_logging::otel::vocabulary::registry::{SPAN_REGISTRY, SpanRole, validate_registry};
use betterlite_logging::otel::vocabulary::resolvers::{
    db_system, resolve_operation, resolve_output_type, resolve_provider, span_role_for_service,
};
use betterlite_logging::otel::vocabulary::semconv::parse_semconv_opt_in;
use betterlite_logging::otel::vocabulary::span_names::NamedSpan;
use serde_with::{As, DefaultOnNull, DisplayFromStr};

#[path = "otel_vocabulary/tests.rs"]
mod tests;

// LiteLLM treats a None call type or provider like an empty string, so null decodes to ""
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "function", deny_unknown_fields)]
pub(crate) enum Input {
    Provider {
        #[serde(with = "As::<Vec<DefaultOnNull>>")]
        values: Vec<String>,
    },
    Operation {
        #[serde(with = "As::<Vec<DefaultOnNull>>")]
        values: Vec<String>,
    },
    OutputType {
        #[serde(with = "As::<Vec<DefaultOnNull>>")]
        values: Vec<String>,
    },
    DbSystem {
        values: Vec<String>,
    },
    SpanRole {
        values: Vec<String>,
    },
    Registry,
    SpanName {
        spans: Vec<SpanInput>,
    },
    Semconv {
        #[serde(with = "As::<Vec<DefaultOnNull>>")]
        values: Vec<String>,
    },
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "role", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SpanInput {
    LlmCall {
        call_type: String,
        request_model: Option<String>,
    },
    McpToolCall {
        #[serde(with = "As::<DisplayFromStr>")]
        method: MCPMethod,
        tool_name: String,
    },
    McpListTools {
        #[serde(with = "As::<DisplayFromStr>")]
        method: MCPMethod,
    },
    ProxyRequest {
        http_method: String,
        route: String,
    },
    Guardrail {
        guardrail_name: String,
    },
    Service {
        service_name: String,
        call_type: Option<String>,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::Provider { values } => {
                resolve_each(values, |value| resolve_provider(value).to_owned())
            }
            Input::Operation { values } => {
                resolve_each(values, |value| resolve_operation(value).to_string())
            }
            Input::OutputType { values } => resolve_each(values, |value| {
                resolve_output_type(value).map(|output| output.to_string())
            }),
            Input::DbSystem { values } => resolve_each(values, db_system),
            Input::SpanRole { values } => resolve_each(values, |value| {
                span_role_for_service(value).map(|role| role.to_string())
            }),
            Input::Registry => registry(),
            Input::SpanName { spans } => {
                serde_json::json!(spans.iter().map(span_name).collect::<Vec<_>>())
            }
            Input::Semconv { values } => resolve_each(values, |value| {
                let categories = parse_semconv_opt_in(value);
                categories
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            }),
        };
        Ok(output)
    }
}

fn resolve_each<T>(values: &[String], resolve: fn(&str) -> T) -> serde_json::Value
where
    T: serde::Serialize,
{
    let resolved: Vec<T> = values.iter().map(|value| resolve(value)).collect();
    serde_json::json!(resolved)
}

fn registry() -> serde_json::Value {
    let entries: Vec<serde_json::Value> = SPAN_REGISTRY
        .iter()
        .map(|spec| {
            let parent = spec.parent.map(|role| role.to_string());
            serde_json::json!({"role": spec.role.to_string(), "kind": spec.kind.to_string(), "parent": parent})
        })
        .collect();
    let roots: Vec<String> = SPAN_REGISTRY
        .iter()
        .filter(|spec| spec.parent.is_none())
        .map(|spec| spec.role.to_string())
        .collect();
    let children: Vec<String> = SPAN_REGISTRY
        .iter()
        .filter(|spec| spec.parent == Some(SpanRole::ProxyRequest))
        .map(|spec| spec.role.to_string())
        .collect();
    let valid = validate_registry(&SPAN_REGISTRY).is_ok();
    serde_json::json!({"valid": valid, "entries": entries, "roots": roots, "children": children})
}

fn span_name(span: &SpanInput) -> String {
    let named = match span {
        SpanInput::LlmCall {
            call_type,
            request_model,
        } => NamedSpan::LlmCall {
            operation: resolve_operation(call_type),
            request_model: request_model.as_deref().unwrap_or_default(),
        },
        SpanInput::McpToolCall { method, tool_name } => NamedSpan::McpToolCall {
            method: *method,
            tool_name,
        },
        SpanInput::McpListTools { method } => NamedSpan::McpListTools { method: *method },
        SpanInput::ProxyRequest { http_method, route } => {
            NamedSpan::ProxyRequest { http_method, route }
        }
        SpanInput::Guardrail { guardrail_name } => NamedSpan::Guardrail { guardrail_name },
        SpanInput::Service {
            service_name,
            call_type,
        } => NamedSpan::Service {
            service_name,
            call_type: call_type.as_deref().unwrap_or_default(),
        },
    };
    named.name()
}
