from types import SimpleNamespace
from typing import cast

from litellm.integrations.otel.model.semconv import (
    resolve_operation,
    resolve_output_type,
    resolve_provider,
)
from litellm.integrations.opentelemetry_utils.gen_ai_semconv import parse_semconv_opt_in
from litellm.integrations.otel.model.spans import (
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


def run(input_data: dict[str, object]) -> object:
    function = cast(str, input_data["function"])
    if function == "Provider":
        return resolve_provider(cast(str | None, input_data.get("custom_llm_provider")))
    if function == "Operation":
        return resolve_operation(cast(str | None, input_data.get("call_type"))).value
    if function == "OutputType":
        output_type = resolve_output_type(cast(str | None, input_data.get("call_type")))
        return output_type.value if output_type is not None else None
    if function == "DbSystem":
        return db_system(cast(str, input_data["service_name"]))
    if function == "SpanRole":
        role = span_role_for_service(cast(str, input_data["service_name"]))
        return role.value if role is not None else None
    if function == "Registry":
        return {
            "valid": _registry_is_valid(),
            "roots": [role.value for role in root_roles()],
            "children": [role.value for role in child_roles(SpanRole.PROXY_REQUEST)],
        }
    if function == "SpanName":
        return _span_name(input_data)
    if function == "Semconv":
        return [category.value for category in parse_semconv_opt_in(cast(str | None, input_data.get("raw")))]
    raise ValueError(f"unknown function {function}")


def _registry_is_valid() -> bool:
    validate_registry()
    return True


def _span_name(input_data: dict[str, object]) -> str:
    kind = cast(str, input_data["kind"])
    if kind == "llm":
        data = SimpleNamespace(operation=resolve_operation(cast(str | None, input_data.get("call_type"))), request_model=cast(str | None, input_data.get("model")))
        return llm_call_span_name(data)
    if kind == "mcp_tool":
        data = SimpleNamespace(method=cast(str, input_data.get("method", "tools/call")), tool_name=cast(str, input_data.get("tool", "")))
        return mcp_tool_call_span_name(data)
    if kind == "mcp_list":
        return mcp_list_tools_span_name(SimpleNamespace(method="tools/list"))
    if kind == "proxy":
        data = SimpleNamespace(http_method=cast(str, input_data.get("http_method", "")), route=cast(str, input_data.get("route", "")))
        return proxy_request_span_name(data)
    if kind == "guardrail":
        return guardrail_span_name(SimpleNamespace(guardrail_name=cast(str, input_data.get("guardrail", ""))))
    data = SimpleNamespace(service_name=cast(str, input_data.get("service", "")), call_type=cast(str | None, input_data.get("call_type")))
    return service_span_name(data)
