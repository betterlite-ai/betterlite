import datetime as dt
import json
from collections.abc import Callable, Iterable, Mapping
from typing import Protocol, TypeAlias, cast
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
from litellm.types.utils import (
    ChatCompletionDeltaToolCall,
    ChatCompletionMessageToolCall,
    Choices,
    Function,
    Message,
    ModelResponse,
    ModelResponseStream,
    Usage,
)
from pydantic import BaseModel, JsonValue

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class ResponseWithUsage(Protocol):
    usage: Usage | None


class FixedDatetime:
    fixed: dt.datetime = dt.datetime.fromtimestamp(0, tz=dt.UTC)

    @classmethod
    def now(cls) -> dt.datetime:
        return cls.fixed


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def integer_value(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def boolean_value(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def array_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def dump_models(models: Iterable[BaseModel]) -> Json:
    return [cast(Json, model.model_dump(exclude_none=True)) for model in models]


def arguments_value(text: str | None) -> Json:
    if not isinstance(text, str):
        return None
    try:
        return cast(Json, json.loads(text))
    except json.JSONDecodeError:
        return text


def function_value(function: Function) -> Json:
    return {"name": function.name, "arguments": arguments_value(function.arguments)}


def tool_call_value(id_value: str | None, type_value: str | None, function: Function) -> Json:
    return {"id": id_value, "type": type_value, "function": function_value(function)}


def base_response() -> ModelResponse:
    message = Message(content=None, role="assistant")
    return ModelResponse(
        id="chatcmpl-oci",
        choices=[Choices(index=0, message=message, finish_reason=None)],
    )


def response_value(response: ModelResponse) -> Json:
    choices: list[Json] = []
    for choice in response.choices:
        message = choice.message
        tool_calls: Json = None
        if message.tool_calls is not None:
            tool_calls = [
                tool_call_value(call.id, call.type, call.function)
                for call in message.tool_calls
                if isinstance(call, ChatCompletionMessageToolCall)
            ]
        choices.append(
            {
                "index": choice.index,
                "content": message.content,
                "tool_calls": tool_calls,
                "finish_reason": choice.finish_reason,
            }
        )
    usage = cast(ResponseWithUsage, response).usage
    if usage is None:
        raise ValueError("the response has no usage")
    details = usage.completion_tokens_details
    return {
        "created": response.created,
        "model": response.model,
        "choices": choices,
        "usage": {
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
            "reasoning_tokens": None if details is None else details.reasoning_tokens,
            "text_tokens": None if details is None else details.text_tokens,
        },
    }


def choice_value(stream: ModelResponseStream) -> Json:
    choice = stream.choices[0]
    delta = choice.delta
    tool_calls: Json = None
    if delta.tool_calls is not None:
        tool_calls = [
            tool_call_value(call.id, call.type, call.function)
            for call in delta.tool_calls
            if isinstance(call, ChatCompletionDeltaToolCall)
        ]
    return {
        "index": choice.index,
        "content": delta.content,
        "tool_calls": tool_calls,
        "finish_reason": choice.finish_reason,
    }


def generic_messages(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], array_value(input_value.get("messages"), "messages"))
    return dump_models(adapt_messages_to_generic_oci_standard(messages))


def cohere_messages(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], array_value(input_value.get("messages"), "messages"))
    return dump_models(adapt_messages_to_cohere_standard(messages))


def generic_tools(input_value: dict[str, Json]) -> Json:
    tools = cast(list[dict[str, JsonValue]], array_value(input_value.get("tools"), "tools"))
    return dump_models(adapt_tool_definition_to_oci_standard(tools, OCIVendors.GENERIC))


def cohere_tools(input_value: dict[str, Json]) -> Json:
    tools = cast(list[dict[str, JsonValue]], array_value(input_value.get("tools"), "tools"))
    return dump_models(adapt_tool_definitions_to_cohere_standard(tools))


def generic_response(input_value: dict[str, Json]) -> Json:
    body = object_value(input_value.get("body"), "body")
    status = integer_value(input_value.get("status"), "status")
    response = base_response()
    result = handle_generic_response(
        cast(dict[str, JsonValue], body), "", response, httpx.Response(status_code=status)
    )
    return response_value(result)


def cohere_response(input_value: dict[str, Json]) -> Json:
    body = object_value(input_value.get("body"), "body")
    model = string_value(input_value.get("model"), "model")
    created = integer_value(input_value.get("created"), "created")
    status = integer_value(input_value.get("status"), "status")
    response = base_response()
    FixedDatetime.fixed = dt.datetime.fromtimestamp(created, tz=dt.UTC)
    with patch.object(cohere_module.datetime, "datetime", FixedDatetime):  # pyright: ignore[reportArgumentType]
        result = handle_cohere_response(
            cast(Mapping[str, JsonValue], body),
            model,
            response,
            httpx.Response(status_code=status),
        )
    return response_value(result)


def finish_reason(input_value: dict[str, Json]) -> Json:
    reasons = array_value(input_value.get("reasons"), "reasons")
    normalized: list[Json] = []
    for reason in reasons:
        if reason is not None and not isinstance(reason, str):
            raise ValueError("reason must be a string or null")
        normalized.append(_normalize_oci_finish_reason(reason))
    return normalized


def generic_stream(input_value: dict[str, Json]) -> Json:
    outputs: list[Json] = []
    for chunk in array_value(input_value.get("chunks"), "chunks"):
        parsed = cast(dict[str, JsonValue], object_value(chunk, "chunk"))
        outputs.append(choice_value(handle_generic_stream_chunk(parsed)))
    return outputs


def cohere_stream(input_value: dict[str, Json]) -> Json:
    outputs: list[Json] = []
    for call in array_value(input_value.get("calls"), "calls"):
        fields = object_value(call, "call")
        chunk = cast(dict[str, JsonValue], object_value(fields.get("chunk"), "chunk"))
        prior_tool_calls = boolean_value(
            fields.get("prior_tool_calls_emitted"), "prior_tool_calls_emitted"
        )
        prior_text = boolean_value(fields.get("prior_text_emitted"), "prior_text_emitted")
        outputs.append(
            choice_value(
                handle_cohere_stream_chunk(
                    chunk,
                    prior_tool_calls_emitted=prior_tool_calls,
                    prior_text_emitted=prior_text,
                )
            )
        )
    return outputs


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "generic_messages": generic_messages,
    "cohere_messages": cohere_messages,
    "generic_tools": generic_tools,
    "cohere_tools": cohere_tools,
    "generic_response": generic_response,
    "cohere_response": cohere_response,
    "finish_reason": finish_reason,
    "generic_stream": generic_stream,
    "cohere_stream": cohere_stream,
}


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    handler = HANDLERS.get(function)
    if handler is None:
        raise ValueError(f"unknown function {function}")
    return handler(input_value)
