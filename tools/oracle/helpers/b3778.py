from typing import Any, TypeAlias, cast

from litellm.proxy.guardrails.guardrail_hooks.lasso.lasso import LassoGuardrail

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    value = cast(Any, input_value.get("value"))
    guardrail = LassoGuardrail(lasso_api_key="oracle-key")
    if operation == "extract_tool_arguments":
        return cast(Json, guardrail._extract_tool_call_fields(value)[2])  # noqa: SLF001
    if operation == "process_response":
        guardrail._process_lasso_response(value)  # noqa: SLF001
        return None
    raise ValueError(f"unsupported operation {operation}")
