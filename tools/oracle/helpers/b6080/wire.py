from typing import Literal, TypedDict, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.openrouter.chat.transformation import OpenrouterConfig
from litellm.llms.openrouter.embedding.transformation import OpenrouterEmbeddingConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import EmbeddingResponse, ModelResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

COST_HEADER = "llm_provider-x-litellm-response-cost"
WIRE_MODEL = "m"
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
CHAT_CONFIG = OpenrouterConfig()
EMBEDDING_CONFIG = OpenrouterEmbeddingConfig()


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


class EmbeddingReplyInput(TypedDict):
    op: Literal["embedding_reply"]
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
    def post_call(
        self,
        original_response: str,
        input: JsonValue = None,
        api_key: str | None = None,
        additional_args: dict[str, JsonValue] | None = None,
    ) -> None:
        self.texts.append(original_response)


def request(case: RequestCase) -> JsonValue:
    body: dict[str, JsonValue] = CHAT_CONFIG.transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openrouter/chat/transformation.py#L137-L163
        model=case["model"],
        messages=cast("list[AllMessageValues]", case["messages"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params=dict(case["optional_params"]),
        litellm_params={},
        headers={},
    )
    return body


def response(case: ReplyCase) -> JsonValue:
    reply = CHAT_CONFIG.transform_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openrouter/chat/transformation.py#L165-L224
        model=WIRE_MODEL,
        raw_response=httpx.Response(200, content=case["body"].encode()),
        model_response=ModelResponse(),
        logging_obj=RecordingLogging(),
        request_data={},
        messages=[],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params={},
        litellm_params={},
        encoding=None,
    )
    hidden = cast("dict[str, dict[str, JsonValue]]", reply._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openrouter/chat/transformation.py#L210-L216
    return {"cost": hidden["additional_headers"].get(COST_HEADER)}


def embedding_reply(case: ReplyCase) -> JsonValue:
    logging = RecordingLogging()
    reply = EMBEDDING_CONFIG.transform_embedding_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openrouter/embedding/transformation.py#L122-L145
        model=WIRE_MODEL,
        raw_response=httpx.Response(200, content=case["body"].encode()),
        model_response=EmbeddingResponse(),
        logging_obj=logging,
        api_key=None,
        request_data={},
        optional_params={},
        litellm_params={},
    )
    decoded = EMBEDDING_OUT.validate_json(reply.model_dump_json())
    result: dict[str, JsonValue] = {
        "response": JSON.validate_python(decoded),
        "logged": list(logging.texts),
    }
    return result
