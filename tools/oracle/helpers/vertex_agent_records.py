from typing import Final, Literal, NotRequired, TypedDict

import httpx
import litellm
from litellm._uuid import uuid as litellm_uuid
from litellm.llms.vertex_ai.agent_engine.transformation import (
    VertexAgentEngineConfig,
    VertexAgentEngineError,
)
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

EMPTY: Final[dict[str, JsonValue]] = {}
CONFIG: Final = VertexAgentEngineConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/agent_engine/transformation.py#L57
TOKEN_MODEL: Final = "gpt-3.5-turbo"


class PromptMessage(TypedDict):
    role: str
    content: str


class UrlCall(TypedDict):
    model: str
    project: str | None
    location: str | None


class RequestCall(TypedDict):
    messages: list[PromptMessage]
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    uuid: str


class ResponseCall(TypedDict):
    status: int
    body: str
    model: str
    messages: list[PromptMessage]
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


Input = UrlInput | RequestInput | ResponseInput


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


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
    usage: NotRequired[UsageView | None]


ANSWER: Final = TypeAdapter(AnswerView)


class FixedCounter:
    def __init__(self, prompt: int, completion: int) -> None:
        self.prompt = prompt
        self.completion = completion

    def __call__(
        self,
        *,
        model: str,
        messages: list[PromptMessage] | None = None,
        text: str | None = None,
        count_response_tokens: bool = False,
    ) -> int:
        del text, count_response_tokens
        if model != TOKEN_MODEL:
            raise ValueError(model)
        if messages is not None:
            return self.prompt
        return self.completion


class FailingCounter:
    def __call__(
        self,
        *,
        model: str,
        messages: list[PromptMessage] | None = None,
        text: str | None = None,
        count_response_tokens: bool = False,
    ) -> int:
        del model, messages, text, count_response_tokens
        raise RuntimeError("token counting is unavailable")


class FixedUuid:
    def __init__(self, value: str) -> None:
        self.value = value

    def __call__(self) -> object:
        return litellm_uuid.UUID(self.value)


def url_outcome(call: UrlCall) -> JsonValue:
    params: dict[str, JsonValue] = {
        "vertex_project": call["project"],
        "vertex_location": call["location"],
    }
    try:
        url = CONFIG.get_complete_url(None, None, call["model"], EMPTY, params)
    except ValueError:
        return {"ok": False}
    return {"ok": True, "url": url}


def request_outcome(call: RequestCall) -> JsonValue:
    litellm_uuid.uuid4 = FixedUuid(call["uuid"])
    optional: dict[str, JsonValue] = dict(call["optional_params"])
    payload = CONFIG.transform_request(call["model"], call["messages"], CONFIG.map_openai_params(call["non_default_params"], optional, call["model"], False), EMPTY, EMPTY)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/agent_engine/transformation.py#L184
    return {"ok": True, "payload": payload}


def usage_value(answer: AnswerView) -> JsonValue:
    usage = answer.get("usage")
    if usage is None:
        return None
    return [usage["prompt_tokens"], usage["completion_tokens"], usage["total_tokens"]]


def answer_state(answer: AnswerView) -> JsonValue:
    choice = answer["choices"][0]
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
        "usage": usage_value(answer),
    }


def response_outcome(call: ResponseCall) -> JsonValue:
    if call["counter"] == "fixed":
        litellm.utils.token_counter = FixedCounter(call["prompt_tokens"], call["completion_tokens"])  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L2550
    else:
        litellm.utils.token_counter = FailingCounter()  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L2550
    raw = httpx.Response(
        call["status"],
        text=call["body"],
        request=httpx.Request("POST", "https://agent-engine.invalid"),
    )
    try:
        response = CONFIG.transform_response(model=call["model"], raw_response=raw, model_response=ModelResponse(), logging_obj=None, request_data=EMPTY, messages=call["messages"], optional_params=EMPTY, litellm_params=EMPTY, encoding=None)  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/agent_engine/transformation.py#L300
    except VertexAgentEngineError:
        return {"ok": False}
    return {"ok": True, "answer": answer_state(ANSWER.validate_json(response.model_dump_json()))}


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "url":
        return {"calls": [url_outcome(call) for call in input_value["calls"]]}
    if input_value["operation"] == "request":
        return {"calls": [request_outcome(call) for call in input_value["calls"]]}
    return {"calls": [response_outcome(call) for call in input_value["calls"]]}
