from collections.abc import Mapping
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.integrations.datadog import datadog_llm_obs

if TYPE_CHECKING:
    from litellm.types.utils import StandardLoggingPayload

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_LOGGER = object.__new__(datadog_llm_obs.DataDogLLMObsLogger)  # pyright: ignore[reportPrivateUsage]


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "redaction_decision":
        return redaction_decision(input_value)
    if function == "message_redaction":
        return message_redaction(input_value)
    if function == "guardrail_redaction":
        return guardrail_redaction(input_value)
    if function == "router_span_fields":
        return router_span_fields(input_value)
    if function == "time_to_first_token":
        return time_to_first_token(input_value)
    return payload_function(function, input_value)


def payload_function(function: Json, input_value: dict[str, Json]) -> Json:
    if function == "cost_dimension_tags":
        return cost_dimension_tags(input_value)
    if function == "latency_metrics":
        return latency_metrics(input_value)
    if function == "stream_detection":
        return stream_detection(input_value)
    if function == "spend_metrics":
        return spend_metrics(input_value)
    if function == "metadata":
        return metadata(input_value)
    raise ValueError(f"unknown {litellm.__name__} helper function {function}")


def redaction_decision(input_value: dict[str, Json]) -> Json:
    litellm.turn_off_message_logging = False
    flags = object_value(input_value.get("flags"), "flags")
    _LOGGER.turn_off_message_logging = bool_value(
        flags.get("turn_off_message_logging"), "turn_off_message_logging"
    )
    _LOGGER.message_logging = bool_value(flags.get("message_logging"), "message_logging")
    request_redacts = bool_value(input_value.get("request_redacts"), "request_redacts")
    kwargs: dict[str, object] = {}
    if request_redacts:
        kwargs["standard_callback_dynamic_params"] = {"turn_off_message_logging": True}
    return _LOGGER._payload_logging_is_off(kwargs)  # pyright: ignore[reportPrivateUsage]


def message_redaction(input_value: dict[str, Json]) -> Json:
    messages = input_value.get("messages")
    projected = datadog_llm_obs._to_dd_messages(messages)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    redacted = datadog_llm_obs._redact_messages(projected)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return plain(redacted)


def guardrail_redaction(input_value: dict[str, Json]) -> Json:
    value = input_value.get("guardrail_information")
    redacted = datadog_llm_obs._guardrail_information_without_prompt_carriers(value)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return plain(redacted)


def router_span_fields(input_value: dict[str, Json]) -> Json:
    routing_decision = input_value.get("routing_decision")
    redact = bool_value(input_value.get("redact_prompt_text"), "redact_prompt_text")
    payload = cast("StandardLoggingPayload", {"metadata": {"routing_decision": routing_decision}})
    fields = datadog_llm_obs._router_span_fields(payload, redact_prompt_text=redact)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return plain(fields)


def time_to_first_token(input_value: dict[str, Json]) -> Json:
    payload = cast(
        "StandardLoggingPayload",
        {
            "startTime": optional_float(input_value.get("start"), "start"),
            "completionStartTime": optional_float(
                input_value.get("completion_start"), "completion_start"
            ),
            "endTime": optional_float(input_value.get("end"), "end"),
        },
    )
    return _LOGGER._get_time_to_first_token_seconds(payload)  # pyright: ignore[reportPrivateUsage]


def cost_dimension_tags(input_value: dict[str, Json]) -> Json:
    payload = payload_of(input_value)
    router_fields = datadog_llm_obs._router_span_fields(payload, redact_prompt_text=False)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    tags = datadog_llm_obs._cost_dimension_tags(payload, router_fields)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    span_tags = [*span_tags_of(input_value), *tags]
    declared = datadog_llm_obs._declared_cost_tags(span_tags)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return {"tags": list(tags), "declared": list(declared)}


def latency_metrics(input_value: dict[str, Json]) -> Json:
    metrics = _LOGGER._get_latency_metrics(payload_of(input_value))  # pyright: ignore[reportPrivateUsage]
    return plain(metrics)


def stream_detection(input_value: dict[str, Json]) -> Json:
    return _LOGGER._get_stream_value_from_payload(payload_of(input_value))  # pyright: ignore[reportPrivateUsage]


def spend_metrics(input_value: dict[str, Json]) -> Json:
    metrics = _LOGGER._get_spend_metrics(payload_of(input_value))  # pyright: ignore[reportPrivateUsage]
    return plain(metrics)


def metadata(input_value: dict[str, Json]) -> Json:
    payload = payload_of(input_value)
    redact = bool_value(input_value.get("redact_prompt_text"), "redact_prompt_text")
    router_fields = datadog_llm_obs._router_span_fields(payload, redact_prompt_text=redact)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    cost_tags = datadog_llm_obs._cost_dimension_tags(payload, router_fields)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    span_tags = [*span_tags_of(input_value), *cost_tags]
    declared = datadog_llm_obs._declared_cost_tags(span_tags)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return plain(
        _LOGGER._get_dd_llm_obs_payload_metadata(  # pyright: ignore[reportPrivateUsage]
            payload, router_fields=router_fields, cost_tags=declared, redact_prompt_text=redact
        )
    )


def payload_of(input_value: dict[str, Json]) -> "StandardLoggingPayload":
    payload = input_value.get("payload")
    return cast("StandardLoggingPayload", object_value(payload, "payload"))


def span_tags_of(input_value: dict[str, Json]) -> list[str]:
    tags = input_value.get("span_tags")
    if tags is None:
        return []
    if not isinstance(tags, list):
        raise ValueError("span_tags must be a list")
    return [string_value(tag, "span_tag") for tag in tags]


def bool_value(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def optional_float(value: Json, name: str) -> float | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number or null")
    return float(value)


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def plain(value: object) -> Json:
    if isinstance(value, Mapping):
        fields = cast("Mapping[object, object]", value)
        return {str(key): plain(item) for key, item in fields.items()}
    if isinstance(value, (list, tuple)):
        return [plain(item) for item in cast("tuple[object, ...]", value)]
    if isinstance(value, (str, int, float, bool)) or value is None:
        return cast(Json, value)
    raise ValueError(f"cannot represent a {type(value).__name__} in a helper answer")
