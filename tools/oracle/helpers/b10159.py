from dataclasses import dataclass, field
from typing import Final, Protocol, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.llms.vertex_ai.vertex_ai_partner_models import get_vertex_ai_partner_model_config
from litellm.types.utils import ModelResponse
from pydantic import ConfigDict, TypeAdapter
from pydantic import JsonValue as Json
from typing_extensions import override

STARTING_ID: Final = "chatcmpl-b10159"
LLAMA_CONFIG: Final = "VertexAILlama3Config"
ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)


class Input(TypedDict):
    publisher: str
    model: str
    status: int
    raw_text: str
    headers: dict[str, str]
    messages: list[Json]
    optional_params: dict[str, Json]
    litellm_params: dict[str, Json]
    request_data: dict[str, Json]
    json_mode: bool | None


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class MessageView(TypedDict):
    role: str
    content: Json


class ChoiceView(TypedDict):
    index: int | None
    message: MessageView
    finish_reason: str | None


class ResponseView(TypedDict):
    id: str | None
    model: str | None
    created: int | None
    choices: list[ChoiceView]
    usage: UsageView | None


RESPONSE: Final = TypeAdapter(ResponseView)


@runtime_checkable
class LoggingSink(Protocol):
    def post_call(self, **kwargs: Json) -> None: ...


@runtime_checkable
class Responder(Protocol):
    def transform_response(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L661-L674
        self,
        *,
        model: str,
        raw_response: httpx.Response,
        model_response: ModelResponse,
        logging_obj: LoggingSink,
        request_data: dict[str, Json],
        messages: list[Json],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        encoding: None,
        api_key: str | None = None,
        json_mode: bool | None = None,
    ) -> ModelResponse: ...


RESPONDER: Final = TypeAdapter(Responder, config=ARBITRARY)


@dataclass
class Logging(LoggingSink):
    kwargs: dict[str, Json] = field(default_factory=dict)

    @override
    def post_call(self, **kwargs: Json) -> None:
        self.kwargs = kwargs


def response_answer(responder: Responder, call: Input) -> ModelResponse:
    raw_response = httpx.Response(
        status_code=call["status"],
        content=call["raw_text"].encode(),
        headers=call["headers"],
    )
    return responder.transform_response(
        model=call["model"],
        raw_response=raw_response,
        model_response=ModelResponse(id=STARTING_ID),
        logging_obj=Logging(),
        request_data=dict(call["request_data"]),
        messages=list(call["messages"]),
        optional_params=dict(call["optional_params"]),
        litellm_params=dict(call["litellm_params"]),
        encoding=None,
        json_mode=call["json_mode"],
    )


def usage_value(usage: UsageView | None) -> Json:
    if usage is None:
        return None
    return {
        "prompt_tokens": usage["prompt_tokens"],
        "completion_tokens": usage["completion_tokens"],
        "total_tokens": usage["total_tokens"],
    }


def choice_value(choice: ChoiceView) -> Json:
    message = choice["message"]
    return {
        "index": choice["index"],
        "message": {"role": message["role"], "content": message["content"]},
        "finish_reason": choice["finish_reason"],
    }


def answer_value(name: str, response: ModelResponse) -> Json:
    record = RESPONSE.validate_json(response.model_dump_json())
    answer: dict[str, Json] = {
        "class": name,
        "id": record["id"],
        "model": record["model"],
        "choices": [choice_value(choice) for choice in record["choices"]],
        "usage": usage_value(record["usage"]),
    }
    if name == LLAMA_CONFIG:
        answer["created"] = record["created"]
    return answer


def run(input_value: Input) -> Json:
    litellm.suppress_debug_info = True
    config = get_vertex_ai_partner_model_config(input_value["model"], input_value["publisher"])
    responder = RESPONDER.validate_python(config)
    return answer_value(type(config).__name__, response_answer(responder, input_value))
