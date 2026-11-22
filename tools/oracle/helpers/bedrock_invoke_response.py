import time
from functools import partial
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, Unpack, cast

import httpx
import litellm
from litellm.llms.bedrock.chat.invoke_transformations.base_invoke_transformation import (
    AmazonInvokeConfig,
)
from litellm.types.utils import Choices, Message, ModelResponse
from pydantic import JsonValue, TypeAdapter

# register_model prints the provider list to stdout
litellm.suppress_debug_info = True

Role: TypeAlias = Literal["assistant", "user", "system", "tool", "function"]


class Logging(Protocol):
    def post_call(self, **kwargs: JsonValue) -> None: ...


class Sink(Logging):
    def __init__(self) -> None:
        self.kwargs: dict[str, JsonValue] = {}

    def post_call(self, **kwargs: JsonValue) -> None:
        self.kwargs = kwargs


class TransformResponseArgs(TypedDict):
    model: str
    raw_response: httpx.Response
    model_response: ModelResponse
    logging_obj: Logging
    request_data: dict[str, JsonValue]
    messages: list[JsonValue]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    encoding: None
    api_key: NotRequired[str | None]
    json_mode: NotRequired[bool | None]


class InvokeApi(Protocol):
    def transform_response(self, **kwargs: Unpack[TransformResponseArgs]) -> ModelResponse: ...


class MessageSeed(TypedDict):
    role: Role
    content: str | None
    tool_calls: list[JsonValue] | None


class ChoiceSeed(TypedDict):
    index: int
    finish_reason: str | None
    message: MessageSeed


class ResponseSeed(TypedDict):
    id: str
    choices: list[ChoiceSeed]


class ReplyCall(TypedDict):
    model: str
    body: str
    status: int
    headers: list[list[str]]
    messages: list[JsonValue]
    request_data: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    tool_name_mappings: dict[str, str]
    json_mode: bool | None
    now: float
    response: ResponseSeed


class Input(TypedDict):
    reply: list[ReplyCall]


class MessageView(TypedDict):
    content: str | None
    tool_calls: list[JsonValue] | None


class ChoiceView(TypedDict):
    index: int
    finish_reason: str | None
    message: MessageView


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ResponseView(TypedDict):
    id: str
    created: int
    model: str
    usage: UsageView | None
    choices: list[ChoiceView]


RESPONSE: Final[TypeAdapter[ResponseView]] = TypeAdapter(ResponseView)


def build_model_response(seed: ResponseSeed) -> ModelResponse:
    choices: list[Choices] = []
    for choice in seed["choices"]:
        picked = choice["message"]
        message = Message(content=picked["content"], role=picked["role"])
        tool_calls = picked["tool_calls"]
        if tool_calls is not None:
            message.tool_calls = []
        choices.append(
            Choices(
                index=choice["index"],
                message=message,
                finish_reason=choice["finish_reason"],
            )
        )
    return ModelResponse(id=seed["id"], choices=choices, created=0, model="")


def error_status(error: Exception) -> int | None:
    carried: object = getattr(error, "status_code", None)
    return carried if isinstance(carried, int) and not isinstance(carried, bool) else None


def reply_view(invoke: InvokeApi, call: ReplyCall) -> JsonValue:
    raw_response = httpx.Response(
        call["status"],
        content=call["body"].encode(),
        headers=[(pair[0], pair[1]) for pair in call["headers"]],
    )
    saved_time = time.time
    time.time = partial(float, call["now"])
    try:
        transformed = invoke.transform_response(
            model=call["model"],
            raw_response=raw_response,
            model_response=build_model_response(call["response"]),
            logging_obj=Sink(),
            request_data=dict(call["request_data"]),
            messages=call["messages"],
            optional_params=dict(call["optional_params"]),
            litellm_params=dict(call["litellm_params"]),
            encoding=None,
            api_key=None,
            json_mode=call["json_mode"],
        )
    finally:
        time.time = saved_time
    view = RESPONSE.validate_json(transformed.model_dump_json())
    choices: list[JsonValue] = []
    for choice in view["choices"]:
        tool_calls = choice["message"]["tool_calls"]
        choices.append(
            {
                "index": choice["index"],
                "finish_reason": choice["finish_reason"],
                "content": choice["message"]["content"],
                "tool_calls": None if tool_calls is None else len(tool_calls),
            }
        )
    usage: JsonValue = None
    carried_usage = view["usage"]
    if carried_usage is not None:
        usage = {
            "prompt_tokens": carried_usage["prompt_tokens"],
            "completion_tokens": carried_usage["completion_tokens"],
            "total_tokens": carried_usage["total_tokens"],
        }
    outcome: JsonValue = {
        "id": view["id"],
        "created": view["created"],
        "model": view["model"],
        "usage": usage,
        "choices": choices,
    }
    return outcome


def reply_result(invoke: InvokeApi, call: ReplyCall) -> JsonValue:
    outcome: JsonValue
    try:
        outcome = {"ok": reply_view(invoke, call)}
    except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L281-L419
        outcome = {
            "error": {
                "class": type(error).__name__,
                "status": error_status(error),
            }
        }
    return outcome


def run(argument: Input) -> JsonValue:
    invoke = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L78-L80
        "InvokeApi",
        AmazonInvokeConfig(),  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L78-L80
    )
    results: list[JsonValue] = [reply_result(invoke, call) for call in argument["reply"]]
    answer: JsonValue = {"results": results}
    return answer
