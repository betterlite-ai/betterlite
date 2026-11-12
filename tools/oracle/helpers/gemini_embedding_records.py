from collections.abc import Callable
from typing import TypeAlias, TypedDict, TypeVar, cast

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
from litellm.types.llms.vertex_ai import (
    GeminiEmbeddingInput,
)
from litellm.types.llms.vertex_ai import (
    VertexAIBatchEmbeddingsResponseObject as BatchResponse,
)
from litellm.types.utils import Embedding, EmbeddingResponse, Usage

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class FileInfo(TypedDict):
    mime_type: str
    uri: str


class Request(TypedDict):
    model: str
    input: GeminiEmbeddingInput
    optional: dict[str, Json]
    resolved_files: dict[str, FileInfo]


class BatchReply(TypedDict):
    model: str
    input: GeminiEmbeddingInput
    response: BatchResponse


class EmbedReply(TypedDict):
    model: str
    input: GeminiEmbeddingInput
    resolved_files: dict[str, FileInfo]
    response: dict[str, Json]


class Input(TypedDict):
    batch_requests: list[Request]
    embed_content_requests: list[Request]
    batch_replies: list[BatchReply]
    embed_content_replies: list[EmbedReply]


Entry = TypeVar("Entry")


TRANSFORM_CONTENT = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini_embeddings/batch_embed_content_transformation.py#L204
    Callable[
        [GeminiEmbeddingInput, str, dict[str, Json], dict[str, dict[str, str]] | None],
        dict[str, Json],
    ],
    transform_content,
)
TRANSFORM_CONTENT_SINGLE = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini_embeddings/batch_embed_content_transformation.py#L255
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


def batch_request(entry: Request) -> Json:
    return TRANSFORM_CONTENT(
        entry["input"], entry["model"], entry["optional"], resolved(entry["resolved_files"])
    )


def embed_request(entry: Request) -> Json:
    return TRANSFORM_CONTENT_SINGLE(
        entry["input"], entry["model"], entry["optional"], resolved(entry["resolved_files"])
    )


def batch_reply(entry: BatchReply) -> Json:
    answer = process_response(
        entry["input"], EmbeddingResponse(), entry["model"], entry["response"]
    )
    return reply_json(answer)


def embed_reply(entry: EmbedReply) -> Json:
    files = resolved(entry["resolved_files"])
    answer = PROCESS_SINGLE(
        entry["input"], EmbeddingResponse(), entry["model"], entry["response"], files
    )
    return reply_json(answer)


def collect(entries: list[Entry], convert: Callable[[Entry], Json]) -> list[Json]:
    results: list[Json] = []
    for entry in entries:
        try:
            results.append(convert(entry))
        except (AttributeError, KeyError, TypeError, ValueError):
            results.append({"error": True})
    return results


def run(input_value: Input) -> Json:
    return {
        "batch_requests": collect(input_value["batch_requests"], batch_request),
        "embed_content_requests": collect(input_value["embed_content_requests"], embed_request),
        "batch_replies": collect(input_value["batch_replies"], batch_reply),
        "embed_content_replies": collect(input_value["embed_content_replies"], embed_reply),
    }
