from types import SimpleNamespace
from typing import TypeAlias, cast

from litellm.integrations.opentelemetry_utils.gen_ai_semconv import parse_semconv_opt_in
from litellm.integrations.otel.model.payloads import (
    GuardrailSpanData,
    LLMCallSpanData,
    MCPListToolsSpanData,
    MCPToolCallSpanData,
    ProxyRequestSpanData,
    ServiceSpanData,
)
from litellm.integrations.otel.model.semconv import (
    resolve_operation,
    resolve_output_type,
    resolve_provider,
)
from litellm.integrations.otel.model.spans import (
    SPAN_REGISTRY,
    SpanRole,
    child_roles,
    db_system,
    guardrail_span_name,
    llm_call_span_name,
    mcp_list_tools_span_name,
    mcp_tool_call_span_name,
    proxy_request_span_name,
    root_roles,
    service_span_name,
    span_role_for_service,
    validate_registry,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def text(value: Json) -> str:
    if not isinstance(value, str):
        raise ValueError("expected a string")
    return value


def optional_text(value: Json) -> str | None:
    return None if value is None else text(value)


def listed(input_value: dict[str, Json], name: str) -> list[Json]:
    values = input_value[name]
    if not isinstance(values, list):
        raise ValueError(f"{name} must be a list")
    return values


def span_name(span: Json) -> str:
    if not isinstance(span, dict):
        raise ValueError("a span must be an object")
    role = span["role"]
    if role == "llm_call":
        operation = resolve_operation(text(span["call_type"]))
        llm = SimpleNamespace(
            operation=operation, request_model=optional_text(span["request_model"])
        )
        return llm_call_span_name(cast("LLMCallSpanData", llm))
    if role == "mcp_tool_call":
        tool = SimpleNamespace(method=text(span["method"]), tool_name=text(span["tool_name"]))
        return mcp_tool_call_span_name(cast("MCPToolCallSpanData", tool))
    if role == "mcp_list_tools":
        listing = SimpleNamespace(method=text(span["method"]))
        return mcp_list_tools_span_name(cast("MCPListToolsSpanData", listing))
    if role == "proxy_request":
        request = ProxyRequestSpanData(
            http_method=text(span["http_method"]), route=text(span["route"])
        )
        return proxy_request_span_name(request)
    if role == "guardrail":
        guardrail = GuardrailSpanData(guardrail_name=text(span["guardrail_name"]))
        return guardrail_span_name(guardrail)
    if role == "service":
        service = ServiceSpanData(
            service_name=text(span["service_name"]), call_type=optional_text(span["call_type"])
        )
        return service_span_name(service)
    raise ValueError(f"unknown span role {role}")


def registry() -> Json:
    validate_registry()
    entries: list[Json] = [
        {
            "role": spec.role.value,
            "kind": spec.kind.value,
            "parent": None if spec.parent is None else spec.parent.value,
        }
        for spec in SPAN_REGISTRY.values()
    ]
    return {
        "valid": True,
        "entries": entries,
        "roots": [role.value for role in root_roles()],
        "children": [role.value for role in child_roles(SpanRole.PROXY_REQUEST)],
    }


def output_type(call_type: str | None) -> str | None:
    resolved = resolve_output_type(call_type)
    return None if resolved is None else resolved.value


def span_role(service_name: str) -> str | None:
    role = span_role_for_service(service_name)
    return None if role is None else role.value


def semconv(raw: str | None) -> Json:
    values = sorted(category.value for category in parse_semconv_opt_in(raw))
    return list[Json](values)


def run(input_value: dict[str, Json]) -> Json:
    result: Json
    match input_value["function"]:
        case "Registry":
            result = registry()
        case "SpanName":
            result = [span_name(span) for span in listed(input_value, "spans")]
        case "Provider":
            result = [
                resolve_provider(optional_text(value)) for value in listed(input_value, "values")
            ]
        case "Operation":
            result = [
                resolve_operation(optional_text(value)).value
                for value in listed(input_value, "values")
            ]
        case "OutputType":
            result = [output_type(optional_text(value)) for value in listed(input_value, "values")]
        case "DbSystem":
            result = [db_system(text(value)) for value in listed(input_value, "values")]
        case "SpanRole":
            result = [span_role(text(value)) for value in listed(input_value, "values")]
        case "Semconv":
            result = [semconv(optional_text(value)) for value in listed(input_value, "values")]
        case function:
            raise ValueError(f"unknown otel_vocabulary function {function}")
    return result
