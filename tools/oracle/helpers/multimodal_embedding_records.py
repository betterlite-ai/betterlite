from collections.abc import Callable
from decimal import Decimal
from typing import TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.exceptions import InternalServerError
from litellm.llms.base_llm.embedding.transformation import LiteLLMLoggingObj
from litellm.llms.vertex_ai.multimodal_embeddings.transformation import (
    VertexAIMultimodalEmbeddingConfig,
)
from litellm.types.utils import Embedding, EmbeddingResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

litellm.suppress_debug_info = True

CONFIG = VertexAIMultimodalEmbeddingConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/multimodal_embeddings/transformation.py#L28


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
    requests: list[RequestCall]
    replies: list[ReplyCall]


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
    EmbeddingResponse,
]

REQUEST = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/multimodal_embeddings/transformation.py#L166
    RequestCallType,
    CONFIG.transform_embedding_request,
)
RESPONSE = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/multimodal_embeddings/transformation.py#L197
    ResponseCall,
    CONFIG.transform_embedding_response,
)


def request_entry(call: RequestCall) -> Json:
    try:
        return REQUEST("", call["input"], call["optional"], {})
    except ValueError:
        return {"error": True}


def answer_entry(answer: EmbeddingResponse) -> Json:
    records: list[Json] = [
        cast(Json, record.model_dump())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2280
        for record in cast("list[Embedding]", answer.data)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2280
    ]
    usage = answer.usage
    if usage is None:
        return {"error": True}
    details = None if usage.prompt_tokens_details is None else cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1748
        "dict[str, Json]",
        usage.prompt_tokens_details.model_dump(),
    )
    seconds = None if details is None else details.get("video_length_seconds")
    video_length = None if seconds is None else format(Decimal(str(seconds)).normalize(), "f")
    return {
        "model": answer.model,
        "data": records,
        "usage": {
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
            "prompt_tokens_details": {
                "character_count": None if details is None else details.get("character_count"),
                "image_count": None if details is None else details.get("image_count"),
                "video_length_seconds": video_length,
            },
        },
    }


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
    requests: list[Json] = [request_entry(call) for call in input_value["requests"]]
    replies: list[Json] = [reply_entry(call) for call in input_value["replies"]]
    output: dict[str, Json] = {"requests": requests, "replies": replies}
    return output
