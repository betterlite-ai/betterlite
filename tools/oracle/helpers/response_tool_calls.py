from collections.abc import Callable
from functools import partial
from typing import Final, TypeAlias, cast
from unittest.mock import patch

import litellm
import litellm.llms
from litellm.litellm_core_utils import api_route_to_call_types
from litellm.litellm_core_utils.prompt_templates.factory import (
    get_tool_calls_from_response,
    has_tool_with_name,
    resolve_structured_messages,
)
from litellm.types.utils import CallTypes

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Messages: TypeAlias = list[dict[str, object]]

CALL_TYPES: Final[dict[str, CallTypes]] = {
    "acompletion": CallTypes.acompletion,
    "aanthropic_messages": CallTypes.aanthropic_messages,
    "anthropic_messages": CallTypes.anthropic_messages,
    "aresponses": CallTypes.aresponses,
    "completion": CallTypes.completion,
    "responses": CallTypes.responses,
}


def _call_type(name: str) -> CallTypes:
    call_type = CALL_TYPES.get(name)
    if call_type is None:
        raise ValueError(f"unknown call type {name}")
    return call_type


def _messages(value: Json) -> Messages | None:
    return cast("Messages | None", value)


class _Handler:
    def __init__(self, messages: Messages | None) -> None:
        self._messages = messages

    def get_structured_messages(self, _data: dict[str, object]) -> Messages | None:
        return self._messages


class _Mappings:
    def __init__(self, handlers: list[dict[str, Json]]) -> None:
        self._handlers = handlers

    def __call__(self) -> dict[CallTypes, Callable[[], _Handler]]:
        mappings: dict[CallTypes, Callable[[], _Handler]] = {}
        for handler in self._handlers:
            names = cast(list[str], handler.get("call_types", []))
            factory = partial(_Handler, _messages(handler.get("messages")))
            for name in names:
                mappings[_call_type(name)] = factory
        return mappings


class _Routes:
    def __init__(self, routes: list[dict[str, Json]]) -> None:
        self._routes = routes

    def __call__(self, route: str) -> list[CallTypes] | None:
        for entry in self._routes:
            if entry.get("route") != route:
                continue
            names = cast(list[str], entry.get("call_types", []))
            return [_call_type(name) for name in names]
        return None


def _resolve_call(call: dict[str, Json]) -> Json:
    handlers = cast(list[dict[str, Json]], call.get("handlers", []))
    routes = cast(list[dict[str, Json]], call.get("routes", []))
    request_kwargs = cast(dict[str, object], call.get("request_kwargs", {}))
    with (
        patch.object(litellm.llms, "load_guardrail_translation_mappings", _Mappings(handlers)),
        patch.object(api_route_to_call_types, "get_call_types_for_route", _Routes(routes)),
    ):
        return cast(
            Json,
            resolve_structured_messages(_messages(call.get("messages")), request_kwargs),
        )


def _get_tool_calls(input_value: dict[str, Json]) -> Json:
    response = cast(dict[str, Json], input_value["response"])
    include_all_choices = cast(bool, input_value.get("include_all_choices", False))
    return cast(
        Json,
        get_tool_calls_from_response(response, include_all_choices=include_all_choices),
    )


def _has_tool(input_value: dict[str, Json]) -> Json:
    tools = cast(list[dict[str, Json]], input_value["tools"])
    return has_tool_with_name(tools, cast(str, input_value["tool_name"]))


def _resolve_structured_messages(input_value: dict[str, Json]) -> Json:
    calls = cast(list[dict[str, Json]], input_value["calls"])
    return [_resolve_call(call) for call in calls]


HANDLERS: Final[dict[str, Callable[[dict[str, Json]], Json]]] = {
    "get_tool_calls": _get_tool_calls,
    "has_tool": _has_tool,
    "resolve_structured_messages": _resolve_structured_messages,
}


def run(input_value: dict[str, Json]) -> Json:
    function = cast(str, input_value["function"])
    handler = HANDLERS.get(function)
    if handler is None:
        raise ValueError(f"unknown function {function}")
    return handler(input_value)
