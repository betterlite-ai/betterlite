use betterlite_logging::otel::vocabulary::constants::MCPMethod;
use betterlite_logging::otel::vocabulary::registry::{SPAN_REGISTRY, SpanRole, validate_registry};
use betterlite_logging::otel::vocabulary::resolvers::{
    db_system, resolve_operation, resolve_output_type, resolve_provider, span_role_for_service,
};
use betterlite_logging::otel::vocabulary::semconv::parse_semconv_opt_in;
use betterlite_logging::otel::vocabulary::span_names::NamedSpan;
use serde_with::{As, DisplayFromStr};

#[path = "otel_vocabulary/tests.rs"]
mod tests;

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "function", deny_unknown_fields)]
pub(crate) enum Input {
    Provider { values: Vec<Option<String>> },
    Operation { values: Vec<Option<String>> },
    OutputType { values: Vec<Option<String>> },
    DbSystem { values: Vec<String> },
    SpanRole { values: Vec<String> },
    Registry,
    SpanName { spans: Vec<SpanInput> },
    Semconv { values: Vec<Option<String>> },
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
            Input::Provider { values } => serde_json::json!(
                values
                    .iter()
                    .map(|value| resolve_provider(value.as_deref().unwrap_or_default()))
                    .collect::<Vec<_>>()
            ),
            Input::Operation { values } => serde_json::json!(
                values
                    .iter()
                    .map(
                        |value| resolve_operation(value.as_deref().unwrap_or_default()).to_string()
                    )
                    .collect::<Vec<_>>()
            ),
            Input::OutputType { values } => serde_json::json!(
                values
                    .iter()
                    .map(
                        |value| resolve_output_type(value.as_deref().unwrap_or_default())
                            .map(|output| output.to_string())
                    )
                    .collect::<Vec<_>>()
            ),
            Input::DbSystem { values } => serde_json::json!(
                values
                    .iter()
                    .map(|value| db_system(value))
                    .collect::<Vec<_>>()
            ),
            Input::SpanRole { values } => serde_json::json!(
                values
                    .iter()
                    .map(|value| span_role_for_service(value).map(|role| role.to_string()))
                    .collect::<Vec<_>>()
            ),
            Input::Registry => registry(),
            Input::SpanName { spans } => {
                serde_json::json!(spans.iter().map(span_name).collect::<Vec<_>>())
            }
            Input::Semconv { values } => serde_json::json!(
                values
                    .iter()
                    .map(
                        |value| parse_semconv_opt_in(value.as_deref().unwrap_or_default())
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                    )
                    .collect::<Vec<_>>()
            ),
        };
        Ok(output)
    }
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
