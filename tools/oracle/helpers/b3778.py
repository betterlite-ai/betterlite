from typing import TypeAlias, cast

from litellm.proxy.guardrails.guardrail_hooks.lasso.lasso import LassoGuardrail, LassoResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    value = cast(dict[str, Json], input_value.get("value"))
    guardrail = LassoGuardrail(lasso_api_key="oracle-key")
    if operation == "extract_tool_arguments":
        return cast(Json, guardrail._extract_tool_call_fields(value)[2])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    if operation == "process_response":
        guardrail._process_lasso_response(cast(LassoResponse, value))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        return None
    raise ValueError(f"unsupported operation {operation}")
