from collections.abc import Callable
from typing import TYPE_CHECKING, TypeAlias, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.proxy.pass_through_endpoints import upstream_usage_headers
from litellm.proxy.pass_through_endpoints.success_handler import PassThroughEndpointLogging
from litellm.proxy.pass_through_endpoints.upstream_usage_headers import UpstreamReportedUsage
from litellm.types.utils import Usage

if TYPE_CHECKING:
    from litellm.types.passthrough_endpoints.pass_through_endpoints import (
        PassthroughStandardLoggingPayload,
    )

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Operation: TypeAlias = Callable[[dict[str, Json]], Json]


class Recording:
    def __init__(self, details: dict[str, object]) -> None:
        self.model_call_details = details


def run(input_value: dict[str, Json]) -> Json:
    operation = _text(input_value.get("operation"), "operation")
    handler = _HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown b3605 operation: {operation}")
    return handler(input_value)


def _text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def _optional_text(value: Json) -> str | None:
    if isinstance(value, str):
        return value
    return None


def _optional_float(value: Json) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    return float(value)


def _optional_int(value: Json) -> int | None:
    if isinstance(value, bool) or not isinstance(value, int):
        return None
    return value


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def _header_pair(value: Json) -> tuple[str, str]:
    if not isinstance(value, list):
        raise TypeError("each header must be a [name, value] pair")
    return (_text(value[0], "header name"), _text(value[1], "header value"))


def _headers(input_value: dict[str, Json]) -> httpx.Headers:
    value = input_value.get("headers", [])
    if not isinstance(value, list):
        raise TypeError("headers must be a list")
    return httpx.Headers([_header_pair(item) for item in value])


def _seed(details: dict[str, object], state: dict[str, Json]) -> None:
    reported = state.get("upstream_reported_usage")
    if isinstance(reported, dict):
        details[upstream_usage_headers.UPSTREAM_REPORTED_USAGE_KEY] = UpstreamReportedUsage(
            response_cost=_optional_float(reported.get("response_cost")),
            total_tokens=_optional_int(reported.get("total_tokens")),
        )
    response_cost = _optional_float(state.get("response_cost"))
    if response_cost is not None:
        details["response_cost"] = response_cost
    total_tokens = _optional_int(state.get("combined_usage_total_tokens"))
    if total_tokens is not None:
        details["combined_usage_object"] = Usage(total_tokens=total_tokens)


def _logging_object(input_value: dict[str, Json]) -> LiteLLMLoggingObj:
    details: dict[str, object] = {}
    state = input_value.get("state")
    if isinstance(state, dict):
        _seed(details, state)
    return cast(LiteLLMLoggingObj, Recording(details))


def _reported_json(value: object) -> Json:
    if value is None:
        return None
    reported = cast(UpstreamReportedUsage, value)
    return {"response_cost": reported.response_cost, "total_tokens": reported.total_tokens}


def _total_tokens(value: object) -> Json:
    if value is None:
        return None
    usage = cast(Usage, value)
    return cast(Json, usage.total_tokens)


def _parse_usage(input_value: dict[str, Json]) -> Json:
    reported = upstream_usage_headers.parse_upstream_reported_usage(_headers(input_value))
    if reported is None:
        return None
    return {"response_cost": reported.response_cost, "total_tokens": reported.total_tokens}


def _parse_cost(input_value: dict[str, Json]) -> Json:
    return upstream_usage_headers._parse_response_cost(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        _optional_text(input_value.get("raw"))
    )


def _parse_tokens(input_value: dict[str, Json]) -> Json:
    return upstream_usage_headers._parse_total_tokens(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        _optional_text(input_value.get("raw"))
    )


def _apply_usage(input_value: dict[str, Json]) -> Json:
    logging_obj = _logging_object(input_value)
    applied = upstream_usage_headers.apply_upstream_reported_usage(
        logging_obj, _headers(input_value)
    )
    details = cast("dict[str, object]", logging_obj.model_call_details)
    return {
        "upstream_reported_usage": _reported_json(
            details.get(upstream_usage_headers.UPSTREAM_REPORTED_USAGE_KEY)
        ),
        "response_cost": cast(Json, details.get("response_cost")),
        "combined_usage_total_tokens": _total_tokens(details.get("combined_usage_object")),
        "has_upstream_reported_usage": upstream_usage_headers.has_upstream_reported_usage(
            logging_obj
        ),
        "applied": _reported_json(applied),
    }


def _set_cost(input_value: dict[str, Json]) -> Json:
    logger = PassThroughEndpointLogging()
    logging_obj = _logging_object(input_value)
    kwargs: dict[str, object] = {}
    kwargs_cost = _optional_float(input_value.get("kwargs_response_cost"))
    if kwargs_cost is not None:
        kwargs["response_cost"] = kwargs_cost
    payload: PassthroughStandardLoggingPayload = {
        "cost_per_request": _optional_float(input_value.get("cost_per_request"))
    }
    updated = cast(
        "dict[str, object]",
        logger._set_cost_per_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
            logging_obj, payload, kwargs
        ),
    )
    details = cast("dict[str, object]", logging_obj.model_call_details)
    return {
        "kwargs_response_cost": cast(Json, updated.get("response_cost")),
        "state_response_cost": cast(Json, details.get("response_cost")),
    }


def _route_result(logger: PassThroughEndpointLogging, route: dict[str, Json]) -> Json:
    url_route = _text(route.get("url_route"), "url_route")
    provider = _optional_text(route.get("custom_llm_provider"))
    return {
        "anthropic": logger.is_anthropic_route(url_route),
        "cohere": logger.is_cohere_route(url_route),
        "assemblyai": logger.is_assemblyai_route(url_route),
        "comprehend_medical": logger.is_comprehend_medical_route(provider),
        "azure_speech": logger.is_azure_speech_route(provider),
        "transcribe": logger.is_transcribe_route(provider),
        "typesafe": logger.is_typesafe_route(provider),
        "openrouter_decisions": logger.is_openrouter_decisions_route(url_route, provider),
        "langfuse": logger.is_langfuse_route(url_route),
        "vertex_ai_live": logger.is_vertex_ai_live_route(url_route),
        "cursor": logger.is_cursor_route(url_route, provider),
    }


def _route(input_value: dict[str, Json]) -> Json:
    logger = PassThroughEndpointLogging()
    routes = input_value.get("routes", [])
    if not isinstance(routes, list):
        raise TypeError("routes must be a list")
    return [_route_result(logger, _object(route, "route")) for route in routes]


_HANDLERS: dict[str, Operation] = {
    "parse_usage": _parse_usage,
    "parse_cost": _parse_cost,
    "parse_tokens": _parse_tokens,
    "apply_usage": _apply_usage,
    "set_cost_per_request": _set_cost,
    "route": _route,
}
