import json
from typing import Final, Literal, NotRequired, Protocol, TypedDict, Unpack, cast

import httpx
from litellm.llms.mistral.chat.transformation import (
    MistralChatResponseIterator,
    MistralConfig,
)
from litellm.types.utils import ModelResponse
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


class Capture:
    def __init__(self) -> None:
        self.model_call_details: dict[str, JsonValue] = {}

    def post_call(self, original_response: str) -> None:
        self.model_call_details["original_response"] = original_response


class ReplyMessage(Protocol):
    content: str | None
    reasoning_content: str | None
    tool_calls: list[JsonValue] | None


class ReplyChoice(Protocol):
    finish_reason: str | None
    message: ReplyMessage


class Reply(Protocol):
    model: str
    choices: list[ReplyChoice]


class ChunkDelta(Protocol):
    content: str | None
    reasoning_content: str | None
    thinking_blocks: list[dict[str, JsonValue]] | None


class ChunkChoice(Protocol):
    index: int
    finish_reason: str | None
    delta: ChunkDelta


class Chunk(Protocol):
    object: str
    choices: list[ChunkChoice]


class ResponseArgs(TypedDict):
    model: str
    raw_response: httpx.Response
    model_response: ModelResponse
    logging_obj: Capture
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

    def transform_response(self, **kwargs: Unpack[ResponseArgs]) -> Reply: ...


class StreamApi(Protocol):
    def chunk_parser(self, chunk: dict[str, JsonValue]) -> Chunk: ...


LITELLM_PARAMS: Final[dict[str, JsonValue]] = {"custom_llm_provider": "mistral"}


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


def reply_value(api: ReplyApi, step: ReplyStep) -> JsonValue:
    args: ResponseArgs = {
        "model": "mistral-small-latest",
        "raw_response": httpx.Response(200, text=json.dumps(step["reply"])),
        "model_response": ModelResponse(),
        "logging_obj": Capture(),
        "request_data": {},
        "messages": [],
        "optional_params": {},
        "litellm_params": {},
        "encoding": None,
    }
    result = api.transform_response(**args)
    choice = result.choices[0]
    calls = choice.message.tool_calls
    return {
        "message": {
            "content": choice.message.content,
            "reasoning_content": choice.message.reasoning_content,
        },
        "response": {
            "model": result.model,
            "finish_reason": choice.finish_reason,
            "content": choice.message.content,
            "tool_calls": None if calls is None else len(calls),
        },
    }


def chunk_value(api: StreamApi, step: ChunkStep) -> JsonValue:
    result = api.chunk_parser(step["chunk"])
    choices: list[JsonValue] = []
    for choice in result.choices:
        projected = TypeAdapter(JsonValue).validate_python(
            {
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "delta": {
                    "content": choice.delta.content,
                    "reasoning_content": choice.delta.reasoning_content,
                    "thinking_blocks": choice.delta.thinking_blocks,
                },
            }
        )
        choices.append(projected)
    return {"object": result.object, "choices": choices}


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
