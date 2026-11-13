import datetime as dt
from collections.abc import Callable, Iterable
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.llms.oci.chat import cohere as cohere_module
from litellm.llms.oci.chat.cohere import (
    adapt_messages_to_cohere_standard,
    adapt_tool_definitions_to_cohere_standard,
    handle_cohere_response,
    handle_cohere_stream_chunk,
)
from litellm.llms.oci.chat.generic import (
    _normalize_oci_finish_reason,  # pyright: ignore[reportPrivateUsage]
    adapt_messages_to_generic_oci_standard,
    adapt_tool_definition_to_oci_standard,  # pyright: ignore[reportUnknownVariableType]
    handle_generic_response,  # pyright: ignore[reportUnknownVariableType]
    handle_generic_stream_chunk,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.types.llms.oci import OCIVendors
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import Choices, Message, ModelResponse
from pydantic import BaseModel, JsonValue

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class FixedDatetime:
    fixed: dt.datetime = dt.datetime.fromtimestamp(0, tz=dt.UTC)

    @classmethod
    def now(cls) -> dt.datetime:
        return cls.fixed


def dumped(model: BaseModel) -> Json:
    return cast(Json, model.model_dump(exclude_none=True))


def dump_models(models: Iterable[BaseModel]) -> Json:
    return [dumped(model) for model in models]


def base_response() -> ModelResponse:
    message = Message(content=None, role="assistant")
    return ModelResponse(
        id="chatcmpl-oci",
        choices=[Choices(index=0, message=message, finish_reason=None)],
    )


def generic_messages(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], input_value.get("messages"))
    return dump_models(adapt_messages_to_generic_oci_standard(messages))


def cohere_messages(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], input_value.get("messages"))
    return dump_models(adapt_messages_to_cohere_standard(messages))


def generic_tools(input_value: dict[str, Json]) -> Json:
    tools = cast(list[dict[str, JsonValue]], input_value.get("tools"))
    return dump_models(adapt_tool_definition_to_oci_standard(tools, OCIVendors.GENERIC))


def cohere_tools(input_value: dict[str, Json]) -> Json:
    tools = cast(list[dict[str, JsonValue]], input_value.get("tools"))
    return dump_models(adapt_tool_definitions_to_cohere_standard(tools))


def finish_reason(input_value: dict[str, Json]) -> Json:
    reasons = cast(list[str | None], input_value.get("reasons"))
    return cast(Json, [_normalize_oci_finish_reason(reason) for reason in reasons])


def generic_stream(input_value: dict[str, Json]) -> Json:
    chunks = cast(list[dict[str, JsonValue]], input_value.get("chunks"))
    return cast(
        Json,
        [dumped(handle_generic_stream_chunk(chunk).choices[0]) for chunk in chunks],
    )


def cohere_stream(input_value: dict[str, Json]) -> Json:
    return cast(
        Json,
        [
            dumped(
                handle_cohere_stream_chunk(
                    cast(dict[str, JsonValue], call.get("chunk")),
                    prior_tool_calls_emitted=cast(bool, call.get("prior_tool_calls_emitted")),
                    prior_text_emitted=cast(bool, call.get("prior_text_emitted")),
                ).choices[0]
            )
            for call in cast(list[dict[str, Json]], input_value.get("calls"))
        ],
    )


def generic_response(input_value: dict[str, Json]) -> Json:
    result = handle_generic_response(
        cast(dict[str, JsonValue], input_value.get("body")),
        "",
        base_response(),
        httpx.Response(status_code=cast(int, input_value.get("status"))),
    )
    return dumped(result)


def cohere_response(input_value: dict[str, Json]) -> Json:
    created = cast(int, input_value.get("created"))
    FixedDatetime.fixed = dt.datetime.fromtimestamp(created, tz=dt.UTC)
    response = base_response()
    with patch.object(cohere_module.datetime, "datetime", FixedDatetime):  # pyright: ignore[reportArgumentType]
        result = handle_cohere_response(
            cast(dict[str, JsonValue], input_value.get("body")),
            cast(str, input_value.get("model")),
            response,
            httpx.Response(status_code=cast(int, input_value.get("status"))),
        )
    return dumped(result)


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "generic_messages": generic_messages,
    "cohere_messages": cohere_messages,
    "generic_tools": generic_tools,
    "cohere_tools": cohere_tools,
    "finish_reason": finish_reason,
    "generic_stream": generic_stream,
    "cohere_stream": cohere_stream,
    "generic_response": generic_response,
    "cohere_response": cohere_response,
}


def run(input_value: dict[str, Json]) -> Json:
    function = cast(str, input_value.get("function"))
    handler = HANDLERS.get(function)
    if handler is None:
        raise ValueError(f"unknown function {function}")
    return handler(input_value)
