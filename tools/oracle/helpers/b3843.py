from collections.abc import Callable
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.integrations.otel.model import payloads

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

if TYPE_CHECKING:
    from litellm.types.utils import StandardLoggingGuardrailInformation


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    value = input_value.get("value")
    if function == "guardrail_error_status":
        entry = cast(dict[str, object], input_value.get("entry", {}))
        parsed = payloads.GuardrailSpanData.from_logging_entry(
            cast("StandardLoggingGuardrailInformation", entry)
        )
        error_type = None if parsed.error is None else parsed.error.error_type
        return {"status": parsed.status, "error_type": error_type}
    names = {
        "cache_token_value": "_cache_token_value",
        "dicts": "_dicts",
        "guardrail_mode_str": "_guardrail_mode_str",
        "json_or_none": "_json_or_none",
        "sanitize_event_metadata": "sanitize_event_metadata",
        "total_masked_entities": "_total_masked_entities",
    }
    operation_name = names.get(cast(str, function))
    if operation_name is None:
        raise ValueError(f"unknown LiteLLM helper function {function} ({litellm.__name__})")
    operation = cast(Callable[..., Json], getattr(payloads, operation_name))
    if function == "cache_token_value":
        result = operation(*cast(list[object], input_value.get("values", [])))
    elif function == "dicts":
        result = list(cast(list[Json], operation(value)))
    else:
        result = operation(value)
    return result
