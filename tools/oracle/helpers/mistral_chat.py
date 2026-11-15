import json
from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, Protocol, TypedDict, Unpack, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.mistral.chat.transformation import (
    MistralChatResponseIterator,
    MistralConfig,
)
from litellm.types.utils import ModelResponse, ModelResponseStream
from pydantic import JsonValue, TypeAdapter


class Message(TypedDict, total=False):
    role: str
    content: JsonValue
    name: str
    tool_calls: list[dict[str, JsonValue]]
    tool_call_id: str
    reasoning_content: str
    thinking_blocks: list[dict[str, JsonValue]]
    cache_control: dict[str, JsonValue]
    function_call: dict[str, JsonValue]


class RequestStep(TypedDict):
    kind: Literal["request"]
    model: str
    messages: list[Message]
    optional_params: NotRequired[dict[str, JsonValue]]


class ReplyStep(TypedDict):
    kind: Literal["reply"]
    reply: dict[str, JsonValue]


class ChunkStep(TypedDict):
    kind: Literal["chunk"]
    chunk: dict[str, JsonValue]


class Input(TypedDict):
    steps: list[RequestStep | ReplyStep | ChunkStep]


class ReplyMessage(TypedDict):
    content: NotRequired[str | None]
    reasoning_content: NotRequired[str | None]
    tool_calls: NotRequired[list[JsonValue] | None]


class ReplyChoice(TypedDict):
    finish_reason: NotRequired[str | None]
    message: ReplyMessage


class Reply(TypedDict):
    model: str
    choices: list[ReplyChoice]


class ChunkDelta(TypedDict):
    content: NotRequired[str | None]
    reasoning_content: NotRequired[str | None]
    thinking_blocks: NotRequired[list[dict[str, JsonValue]] | None]


class ChunkChoice(TypedDict):
    index: int
    finish_reason: NotRequired[str | None]
    delta: ChunkDelta


class Chunk(TypedDict):
    object: str
    choices: list[ChunkChoice]


class ResponseArgs(TypedDict):
    model: str
    raw_response: httpx.Response
    model_response: ModelResponse
    logging_obj: Logging
    request_data: dict[str, JsonValue]
    messages: list[Message]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    encoding: None


class ReplyApi(Protocol):
    def transform_request(
        self,
        model: str,
        messages: list[Message],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, JsonValue],
    ) -> JsonValue: ...

    def transform_response(self, **kwargs: Unpack[ResponseArgs]) -> ModelResponse: ...


class StreamApi(Protocol):
    def chunk_parser(self, chunk: dict[str, JsonValue]) -> ModelResponseStream: ...


LITELLM_PARAMS: Final[dict[str, JsonValue]] = {"custom_llm_provider": "mistral"}
JSON: Final = TypeAdapter(JsonValue)


def apis() -> tuple[ReplyApi, StreamApi]:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/chat/transformation.py#L55
        "tuple[ReplyApi, StreamApi]",
        (
            MistralConfig(),
            MistralChatResponseIterator(
                streaming_response=iter(()), sync_stream=True, json_mode=False
            ),
        ),
    )


def request_value(api: ReplyApi, step: RequestStep) -> JsonValue:
    optional: dict[str, JsonValue] = {}
    given = step.get("optional_params")
    if given is not None:
        optional.update(given)
    return api.transform_request(
        model=step["model"],
        messages=step["messages"],
        optional_params=optional,
        litellm_params=LITELLM_PARAMS,
        headers={},
    )


def logging_object() -> Logging:
    return Logging(
        model="mistral-small-latest",
        messages=None,
        stream=False,
        call_type="completion",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="b1409",
        function_id="b1409",
    )


def reply_value(api: ReplyApi, step: ReplyStep) -> JsonValue:
    args: ResponseArgs = {
        "model": "mistral-small-latest",
        "raw_response": httpx.Response(200, text=json.dumps(step["reply"])),
        "model_response": ModelResponse(),
        "logging_obj": logging_object(),
        "request_data": {},
        "messages": [],
        "optional_params": {},
        "litellm_params": {},
        "encoding": None,
    }
    result = api.transform_response(**args)
    view = TypeAdapter(Reply).validate_json(result.model_dump_json())
    choice = view["choices"][0]
    message = choice["message"]
    content = message.get("content")
    calls = message.get("tool_calls")
    return {
        "message": {
            "content": content,
            "reasoning_content": message.get("reasoning_content"),
        },
        "response": {
            "model": view["model"],
            "finish_reason": choice.get("finish_reason"),
            "content": content,
            "tool_calls": None if calls is None else len(calls),
        },
    }


def chunk_value(api: StreamApi, step: ChunkStep) -> JsonValue:
    result = api.chunk_parser(step["chunk"])
    view = TypeAdapter(Chunk).validate_json(result.model_dump_json())
    choices: list[JsonValue] = []
    for choice in view["choices"]:
        delta = choice["delta"]
        entry: ChunkChoice = {
            "index": choice["index"],
            "finish_reason": choice.get("finish_reason"),
            "delta": {
                "content": delta.get("content"),
                "reasoning_content": delta.get("reasoning_content"),
                "thinking_blocks": delta.get("thinking_blocks"),
            },
        }
        choices.append(JSON.validate_python(entry))
    return {"object": view["object"], "choices": choices}


def run(input_value: Input) -> JsonValue:
    reply_api, stream_api = apis()
    results: list[JsonValue] = []
    for step in input_value["steps"]:
        if step["kind"] == "request":
            results.append(request_value(reply_api, step))
        elif step["kind"] == "reply":
            results.append(reply_value(reply_api, step))
        else:
            results.append(chunk_value(stream_api, step))
    return results
