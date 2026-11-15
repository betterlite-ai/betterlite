from typing import Literal, TypedDict, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import EmbeddingResponse, ModelResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from .configs import CHAT, EMBEDDING, MODEL

COST_HEADER = "llm_provider-x-litellm-response-cost"
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class RequestCase(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]


class RequestInput(TypedDict):
    op: Literal["request"]
    cases: list[RequestCase]


class ReplyCase(TypedDict):
    body: str


class ResponseInput(TypedDict):
    op: Literal["response"]
    cases: list[ReplyCase]


class RecordOut(TypedDict):
    object: str
    embedding: list[float] | str
    index: int


class UsageOut(TypedDict):
    prompt_tokens: int
    total_tokens: int


class EmbeddingOut(TypedDict):
    object: str
    data: list[RecordOut]
    model: str
    usage: UsageOut


EMBEDDING_OUT: TypeAdapter[EmbeddingOut] = TypeAdapter(EmbeddingOut)


class RecordingLogging(LiteLLMLoggingObj):
    def __init__(self) -> None:
        self.texts: list[str] = []

    @override
    def post_call(self, original_response: str, *args: JsonValue, **kwargs: JsonValue) -> None:
        self.texts.append(original_response)


def requests(cases: list[RequestCase]) -> JsonValue:
    bodies: list[dict[str, JsonValue]] = [
        CHAT.transform_request(
            model=case["model"],
            messages=cast("list[AllMessageValues]", case["messages"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            optional_params=case["optional_params"],
            litellm_params={},
            headers={},
        )
        for case in cases
    ]
    return JSON.validate_python(bodies)


def responses(cases: list[ReplyCase]) -> JsonValue:
    costs: list[JsonValue] = []
    for case in cases:
        reply = CHAT.transform_response(
            MODEL,
            httpx.Response(200, content=case["body"].encode()),
            ModelResponse(),
            RecordingLogging(),
            {},
            [],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            {},
            {},
            None,
        )
        hidden = cast("dict[str, dict[str, JsonValue]]", reply._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openrouter/chat/transformation.py#L210-L216
        costs.append({"cost": hidden["additional_headers"].get(COST_HEADER)})
    return JSON.validate_python(costs)


def embedding_replies(cases: list[ReplyCase]) -> JsonValue:
    loggings: list[RecordingLogging] = [RecordingLogging() for _ in cases]
    replies = [
        EMBEDDING.transform_embedding_response(
            model=MODEL,
            raw_response=httpx.Response(200, content=case["body"].encode()),
            model_response=EmbeddingResponse(),
            logging_obj=logging,
            api_key=None,
            request_data={},
            optional_params={},
            litellm_params={},
        )
        for case, logging in zip(cases, loggings, strict=True)
    ]
    answers: list[JsonValue] = [
        {
            "response": JSON.validate_python(EMBEDDING_OUT.validate_json(reply.model_dump_json())),
            "logged": list(logging.texts),
        }
        for reply, logging in zip(replies, loggings, strict=True)
    ]
    return JSON.validate_python(answers)
