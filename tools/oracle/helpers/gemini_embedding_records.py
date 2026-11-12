from collections.abc import Callable
from typing import Literal, TypeAlias, TypedDict, cast

import litellm.llms.vertex_ai.gemini_embeddings.batch_embed_content_transformation as transformation
from litellm.llms.vertex_ai.gemini_embeddings.batch_embed_content_transformation import (
    process_embed_content_response,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini_embeddings/batch_embed_content_transformation.py#L402
    process_response,
)
from litellm.llms.vertex_ai.gemini_embeddings.batch_embed_content_transformation import (
    transform_openai_input_gemini_content as transform_content,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini_embeddings/batch_embed_content_transformation.py#L204
)
from litellm.llms.vertex_ai.gemini_embeddings.batch_embed_content_transformation import (
    transform_openai_input_gemini_embed_content as transform_single,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini_embeddings/batch_embed_content_transformation.py#L255
)
from litellm.types.llms.vertex_ai import GeminiEmbeddingInput
from litellm.types.llms.vertex_ai import (
    VertexAIBatchEmbeddingsResponseObject as BatchResponse,
)
from litellm.types.utils import Embedding, EmbeddingResponse, Usage

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class FileInfo(TypedDict):
    mime_type: str
    uri: str


class Call(TypedDict):
    kind: Literal["batch_request", "embed_request", "batch_reply", "embed_reply"]
    model: str
    input: GeminiEmbeddingInput
    optional: dict[str, Json]
    resolved_files: dict[str, FileInfo]
    response: Json


class Input(TypedDict):
    calls: list[Call]


TRANSFORM_CONTENT = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini_embeddings/batch_embed_content_transformation.py#L204
    Callable[
        [GeminiEmbeddingInput, str, dict[str, Json], dict[str, dict[str, str]] | None],
        dict[str, Json],
    ],
    transform_content,
)
TRANSFORM_SINGLE = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini_embeddings/batch_embed_content_transformation.py#L255
    Callable[
        [GeminiEmbeddingInput, str, dict[str, Json], dict[str, dict[str, str]] | None],
        dict[str, Json],
    ],
    transform_single,
)
PROCESS_SINGLE = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini_embeddings/batch_embed_content_transformation.py#L402
    Callable[
        [
            GeminiEmbeddingInput,
            EmbeddingResponse,
            str,
            dict[str, Json],
            dict[str, dict[str, str]] | None,
        ],
        EmbeddingResponse,
    ],
    process_embed_content_response,
)


def standin_counter(model: str = "", text: str | list[str] | None = None) -> int:
    del model
    match text:
        case str():
            return len(text)
        case list():
            return sum(len(part) for part in text)
        case None:
            raise ValueError("the standin counter needs text")


# the transformation reads the counter from this module global, its only injection seam
transformation.token_counter = standin_counter  # type: ignore[assignment, attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini_embeddings/batch_embed_content_transformation.py#L28


def resolved(value: dict[str, FileInfo]) -> dict[str, dict[str, str]]:
    files: dict[str, dict[str, str]] = {}
    for name, info in value.items():
        files[name] = {"mime_type": info["mime_type"], "uri": info["uri"]}
    return files


def usage_json(usage: Usage) -> Json:
    details = usage.prompt_tokens_details
    rendered: Json = None
    if details is not None:
        rendered = {
            "text_tokens": details.text_tokens,
            "audio_tokens": details.audio_tokens,
            "image_tokens": details.image_tokens,
            "video_tokens": details.video_tokens,
        }
    return {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_tokens_details": rendered,
    }


def reply_json(answer: EmbeddingResponse) -> Json:
    model = answer.model
    usage = answer.usage
    if model is None:
        raise ValueError("the embedding response carries no model")
    if usage is None:
        raise ValueError("the embedding response carries no usage")
    records: list[Json] = []
    for record in cast("list[Embedding]", answer.data):  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2302
        values = cast("list[Json] | str", record.embedding)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2281
        records.append({"object": record.object, "embedding": values, "index": record.index})
    return {"model": model, "data": records, "usage": usage_json(usage)}


def conversion(call: Call) -> Json:
    if call["kind"] == "batch_request":
        return TRANSFORM_CONTENT(
            call["input"], call["model"], call["optional"], resolved(call["resolved_files"])
        )
    if call["kind"] == "embed_request":
        return TRANSFORM_SINGLE(
            call["input"], call["model"], call["optional"], resolved(call["resolved_files"])
        )
    if call["kind"] == "batch_reply":
        answer = process_response(
            call["input"],
            EmbeddingResponse(),
            call["model"],
            cast("BatchResponse", call["response"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/vertex_ai.py#L605-L609
        )
        return reply_json(answer)
    answer = PROCESS_SINGLE(
        call["input"],
        EmbeddingResponse(),
        call["model"],
        cast("dict[str, Json]", call["response"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini_embeddings/batch_embed_content_transformation.py#L402-L406
        resolved(call["resolved_files"]),
    )
    return reply_json(answer)


def run(input_value: Input) -> Json:
    values: list[Json] = []
    for call in input_value["calls"]:
        try:
            values.append(conversion(call))
        except (AttributeError, KeyError, TypeError, ValueError):
            values.append({"error": True})
    return {"calls": values}
