from collections.abc import Callable
from decimal import Decimal
from typing import Protocol, TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.exceptions import InternalServerError
from litellm.llms.base_llm.embedding.transformation import LiteLLMLoggingObj
from litellm.llms.vertex_ai.multimodal_embeddings.transformation import (
    VertexAIMultimodalEmbeddingConfig,
)
from litellm.types.utils import EmbeddingResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

litellm.suppress_debug_info = True

CONFIG = VertexAIMultimodalEmbeddingConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/multimodal_embeddings/transformation.py#L28


class HeaderCall(TypedDict):
    caller: dict[str, str]
    api_key: str


class MappingCall(TypedDict):
    options: dict[str, Json]
    optional: dict[str, Json]


class RequestCall(TypedDict):
    input: Json
    optional: dict[str, Json]


class ReplyCall(TypedDict):
    model: str
    status: int
    response: Json
    input: Json
    optional: dict[str, Json]


class Input(TypedDict):
    headers: list[HeaderCall]
    mappings: list[MappingCall]
    requests: list[RequestCall]
    replies: list[ReplyCall]


class EmbeddingRecord(Protocol):
    embedding: Json
    index: int
    object: str


class PromptDetails(Protocol):
    character_count: int | None
    image_count: int | None
    video_length_seconds: float | None


class AnswerUsage(Protocol):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: PromptDetails | None


class Answer(Protocol):
    model: str | None
    data: list[EmbeddingRecord]
    usage: AnswerUsage | None


MapCall = Callable[[dict[str, Json], dict[str, Json], str, bool], dict[str, Json]]
EnvCall = Callable[
    [dict[str, str], str, list[Json], dict[str, Json], dict[str, Json], str | None, str | None],
    dict[str, str],
]
RequestCallType = Callable[[str, Json, dict[str, Json], dict[str, str]], dict[str, Json]]
ResponseCall = Callable[
    [
        str,
        httpx.Response,
        EmbeddingResponse,
        LiteLLMLoggingObj,
        str | None,
        dict[str, Json],
        dict[str, Json],
        dict[str, Json],
    ],
    Answer,
]

MAP = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/multimodal_embeddings/transformation.py#L32
    MapCall,
    CONFIG.map_openai_params,
)
ENV = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/multimodal_embeddings/transformation.py#L44
    EnvCall,
    CONFIG.validate_environment,
)
REQUEST = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/multimodal_embeddings/transformation.py#L166
    RequestCallType,
    CONFIG.transform_embedding_request,
)
RESPONSE = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/multimodal_embeddings/transformation.py#L197
    ResponseCall,
    CONFIG.transform_embedding_response,
)


def header_entry(call: HeaderCall) -> Json:
    headers = ENV(call["caller"], "", [], {}, {}, call["api_key"], None)
    mapped: dict[str, Json] = dict(headers.items())
    entry: dict[str, Json] = {}
    entry["headers"] = mapped
    return entry


def mapping_entry(call: MappingCall) -> Json:
    optional = MAP(call["options"], dict(call["optional"]), "", False)  # noqa: FBT003 - LiteLLM's positional drop_params  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/multimodal_embeddings/transformation.py#L32
    entry: dict[str, Json] = {}
    entry["optional"] = optional
    return entry


def request_entry(call: RequestCall) -> Json:
    try:
        return REQUEST("", call["input"], call["optional"], {})
    except ValueError:
        return {"error": True}


def duration_text(value: float | None) -> str | None:
    if value is None:
        return None
    return format(Decimal(str(value)).normalize(), "f")


def answer_entry(answer: Answer) -> Json:
    records: list[Json] = []
    for record in answer.data:
        record_entry: dict[str, Json] = {}
        record_entry["object"] = record.object
        record_entry["embedding"] = record.embedding
        record_entry["index"] = record.index
        records.append(record_entry)
    usage = answer.usage
    if usage is None:
        return {"error": True}
    details = usage.prompt_tokens_details
    character_count: Json = None
    image_count: Json = None
    video_length: Json = None
    if details is not None:
        character_count = details.character_count
        image_count = details.image_count
        video_length = duration_text(details.video_length_seconds)
    media: dict[str, Json] = {}
    media["character_count"] = character_count
    media["image_count"] = image_count
    media["video_length_seconds"] = video_length
    counts: dict[str, Json] = {}
    counts["prompt_tokens"] = usage.prompt_tokens
    counts["completion_tokens"] = usage.completion_tokens
    counts["total_tokens"] = usage.total_tokens
    counts["prompt_tokens_details"] = media
    entry: dict[str, Json] = {}
    entry["model"] = answer.model
    entry["data"] = records
    entry["usage"] = counts
    return entry


def reply_entry(call: ReplyCall) -> Json:
    try:
        request = REQUEST(call["model"], call["input"], call["optional"], {})
        raw = httpx.Response(call["status"], json=call["response"])
        answer = RESPONSE(
            call["model"],
            raw,
            EmbeddingResponse(),
            cast("LiteLLMLoggingObj", None),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/multimodal_embeddings/transformation.py#L197
            None,
            request,
            {},
            {},
        )
    except InternalServerError:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L760
        return {
            "error": {
                "status": 500,
                "type": "internal_server_error",
                "param": None,
                "code": None,
            },
        }
    except Exception:  # noqa: BLE001 - LiteLLM raises this bare for a refused reply  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/multimodal_embeddings/transformation.py#L209
        return {"error": True}
    return answer_entry(answer)


def run(input_value: Input) -> Json:
    headers: list[Json] = [header_entry(call) for call in input_value["headers"]]
    mappings: list[Json] = [mapping_entry(call) for call in input_value["mappings"]]
    requests: list[Json] = [request_entry(call) for call in input_value["requests"]]
    replies: list[Json] = [reply_entry(call) for call in input_value["replies"]]
    output: dict[str, Json] = {
        "headers": headers,
        "mappings": mappings,
        "requests": requests,
        "replies": replies,
    }
    return output
