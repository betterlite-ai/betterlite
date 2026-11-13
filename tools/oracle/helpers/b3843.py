import json
from collections.abc import Callable
from contextlib import suppress
from dataclasses import asdict
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.integrations.otel.model import payloads
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.services import ServiceLoggerPayload, ServiceTypes

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

if TYPE_CHECKING:
    from litellm.types.utils import StandardLoggingGuardrailInformation, StandardLoggingPayload


def run(input_value: dict[str, Json]) -> Json:
    return {"result": dispatch(input_value)}


def dispatch(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    value = input_value.get("value")
    if function == "guardrail_error_status":
        entry = cast(dict[str, object], input_value.get("entry", {}))
        parsed = payloads.GuardrailSpanData.from_logging_entry(
            cast("StandardLoggingGuardrailInformation", entry)
        )
        error_type = None if parsed.error is None else parsed.error.error_type
        return {"status": parsed.status, "error_type": error_type}
    factories = {
        "request_params": request_params,
        "usage": usage,
        "cost": cost,
        "server_info": server_info,
        "service": service,
        "span_error": span_error,
        "proxy_request": proxy_request,
    }
    factory = factories.get(cast(str, function))
    if factory is not None:
        return factory(value)
    if function == "guardrail":
        guardrail_entry = cast(dict[str, Json], input_value.get("entry", {}))
        guardrail_entry["guardrail_mode"] = mode_input(guardrail_entry.get("guardrail_mode"))
        entry = cast("StandardLoggingGuardrailInformation", guardrail_entry)
        result = asdict(payloads.GuardrailSpanData.from_logging_entry(entry))
        for key in ("response_json", "usage_json"):
            encoded = result.get(key)
            if isinstance(encoded, str):
                with suppress(json.JSONDecodeError):
                    result[key] = json.dumps(
                        json.loads(encoded), separators=(",", ":"), sort_keys=True
                    )
        return result
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
    elif function == "guardrail_mode_str":
        result = operation(mode_input(value))
    else:
        result = operation(value)
    return result


def mode_input(value: Json) -> Json:
    if isinstance(value, str):
        return GuardrailEventHooks(value)
    if isinstance(value, list):
        return [mode_input(item) for item in value]
    return value


def request_params(value: Json) -> Json:
    raw = cast(dict[str, object], value)
    return payloads.LLMRequestParams.from_model_parameters(raw).__dict__


def usage(value: Json) -> Json:
    raw = cast("StandardLoggingPayload", value)
    parsed = payloads.LLMUsage.from_standard_logging_payload(raw)
    return {
        **parsed.__dict__,
        "uncached_input_tokens": parsed.uncached_input_tokens,
        "non_reasoning_output_tokens": parsed.non_reasoning_output_tokens,
    }


def cost(value: Json) -> Json:
    raw = cast(dict[str, object], value)
    fields = cast(dict[str, Json], payloads.LLMCost.from_breakdown(raw).__dict__)
    return {
        key: int(amount) if isinstance(amount, float) and amount.is_integer() else amount
        for key, amount in fields.items()
    }


def server_info(value: Json) -> Json:
    parsed = payloads.ServerInfo.from_api_base(cast(str | None, value))
    return None if parsed is None else parsed.__dict__


def service(value: Json) -> Json:
    raw = cast(dict[str, object], value)
    payload = ServiceLoggerPayload(
        is_error=False,
        service=ServiceTypes(cast(str, raw.get("service"))),
        duration=0,
        call_type=cast(str, raw.get("call_type", "")),
        caller=cast(str | None, raw.get("caller")),
        error=cast(str | None, raw.get("error")),
        event_metadata={},
    )
    metadata = cast(dict[str, object], raw.get("event_metadata", {}))
    parsed = payloads.ServiceSpanData.from_payload(payload, metadata)
    error = None if parsed.error is None else parsed.error.message
    return {
        "service_name": parsed.service_name,
        "call_type": parsed.call_type,
        "caller": parsed.caller,
        "error": error,
        "event_metadata": dict(parsed.event_metadata),
    }


def span_error(value: Json) -> Json:
    name = "_parse_error"
    parse = cast(Callable[[object], payloads.SpanError | None], getattr(payloads, name))
    parsed = parse(value)
    return None if parsed is None else asdict(parsed)


def proxy_request(value: Json) -> Json:
    raw = cast(dict[str, Json], value)
    parsed = payloads.ProxyRequestSpanData(
        http_method=cast(str, raw["http_method"]),
        route=cast(str, raw["route"]),
        url_path=cast(str | None, raw.get("url_path")),
        status_code=cast(int | None, raw.get("status_code")),
    )
    return asdict(parsed)
