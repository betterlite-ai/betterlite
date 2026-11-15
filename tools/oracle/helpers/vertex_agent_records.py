import functools
from collections.abc import Callable, Mapping, Sequence
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict

import httpx
import litellm
from fastuuid import UUID
from litellm._uuid import uuid as litellm_uuid
from litellm.llms.vertex_ai.agent_engine.transformation import (
    VertexAgentEngineConfig,
    VertexAgentEngineError,
)
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

EMPTY: Final[dict[str, JsonValue]] = {}
# transform_request takes a model for its signature, but the query payload never carries one
REQUEST_MODEL: Final = "agent_engine"


class UrlCall(TypedDict):
    model: str
    project: str | None
    location: str | None


class RequestCall(TypedDict):
    messages: list[dict[str, JsonValue]]
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    uuid: str


class ResponseCall(TypedDict):
    status: int
    body: str
    model: str
    messages: list[dict[str, JsonValue]]
    prompt_tokens: int
    completion_tokens: int
    counter: Literal["fixed", "failure"]


class UrlInput(TypedDict):
    operation: Literal["url"]
    calls: list[UrlCall]


class RequestInput(TypedDict):
    operation: Literal["request"]
    calls: list[RequestCall]


class ResponseInput(TypedDict):
    operation: Literal["response"]
    calls: list[ResponseCall]


Input: TypeAlias = UrlInput | RequestInput | ResponseInput


class MessageView(TypedDict):
    role: str
    content: str | None


class ChoiceView(TypedDict):
    index: int
    finish_reason: str | None
    message: MessageView


class AnswerView(TypedDict):
    model: str | None
    choices: list[ChoiceView]
    usage: NotRequired[dict[str, JsonValue] | None]


ANSWER: Final = TypeAdapter(AnswerView)


class Counter:
    def __init__(self, kind: Literal["fixed", "failure"], prompt: int, completion: int) -> None:
        self.kind = kind
        self.prompt = prompt
        self.completion = completion

    def __call__(
        self,
        *,
        model: str = "",
        messages: Sequence[Mapping[str, JsonValue]] | None = None,
        text: str | None = None,
        count_response_tokens: bool = False,
    ) -> int:
        del model, text, count_response_tokens
        if self.kind == "failure":
            raise RuntimeError("token counting is unavailable")
        if messages is not None:
            return self.prompt
        return self.completion


class FixedUuid:
    def __init__(self, value: str) -> None:
        self.value = value

    def __call__(self) -> UUID:
        return litellm_uuid.UUID(self.value)


# the cast carries LiteLLM's message and logging types; this view names the shapes the case sends
TransformResponse: TypeAlias = Callable[
    [
        str,
        httpx.Response,
        ModelResponse,
        None,
        dict[str, JsonValue],
        Sequence[Mapping[str, JsonValue]],
        dict[str, JsonValue],
        dict[str, JsonValue],
        None,
    ],
    ModelResponse,
]


class Config(Protocol):
    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> str: ...

    def transform_request(
        self,
        model: str,
        messages: Sequence[Mapping[str, JsonValue]],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, JsonValue],
    ) -> dict[str, JsonValue]: ...

    transform_response: TransformResponse


class TokenCounterHost(Protocol):
    token_counter: Callable[[str], int]


CFG: Final[Config] = VertexAgentEngineConfig()  # type: ignore[no-untyped-call, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/agent_engine/transformation.py#L57-L65


def url_outcome(call: UrlCall) -> JsonValue:
    params: dict[str, JsonValue] = {
        "vertex_project": call["project"],
        "vertex_location": call["location"],
    }
    try:
        url = CFG.get_complete_url(None, None, call["model"], EMPTY, params)
    except ValueError:
        return {"ok": False}
    return {"ok": True, "url": url}


def request_outcome(call: RequestCall) -> JsonValue:
    litellm_uuid.uuid4 = FixedUuid(call["uuid"])
    optional: dict[str, JsonValue] = dict(call["optional_params"])
    mapped = CFG.map_openai_params(
        call["non_default_params"], optional, REQUEST_MODEL, drop_params=False
    )
    payload = CFG.transform_request(REQUEST_MODEL, call["messages"], mapped, EMPTY, EMPTY)
    return {"ok": True, "payload": payload}


def answer_state(answer: AnswerView) -> JsonValue:
    choice = answer["choices"][0]
    usage = answer.get("usage")
    return {
        "model": answer["model"],
        "choice": {
            "index": choice["index"],
            "finish_reason": choice["finish_reason"],
            "message": {
                "role": choice["message"]["role"],
                "content": choice["message"]["content"],
            },
        },
        "usage": None
        if usage is None
        else [usage["prompt_tokens"], usage["completion_tokens"], usage["total_tokens"]],
    }


def response_outcome(call: ResponseCall) -> JsonValue:
    counter = Counter(call["counter"], call["prompt_tokens"], call["completion_tokens"])
    # LiteLLM re-imports its counter from this module on each call, so the fixture swaps it there
    host: TokenCounterHost = litellm.utils
    host.token_counter = functools.partial(counter)
    raw = httpx.Response(
        call["status"],
        text=call["body"],
        request=httpx.Request("POST", "https://agent-engine.invalid"),
    )
    try:
        response = CFG.transform_response(
            call["model"],
            raw,
            ModelResponse(),
            None,
            EMPTY,
            call["messages"],
            EMPTY,
            EMPTY,
            None,
        )
    except VertexAgentEngineError:
        return {"ok": False}
    return {"ok": True, "answer": answer_state(ANSWER.validate_json(response.model_dump_json()))}


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "url":
        url_calls = input_value["calls"]
        return {"calls": [url_outcome(call) for call in url_calls]}
    if input_value["operation"] == "request":
        request_calls = input_value["calls"]
        return {"calls": [request_outcome(call) for call in request_calls]}
    response_calls = input_value["calls"]
    return {"calls": [response_outcome(call) for call in response_calls]}
