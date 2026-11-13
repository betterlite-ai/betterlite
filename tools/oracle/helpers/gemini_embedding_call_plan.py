from typing import TypedDict

from litellm.llms.vertex_ai.gemini_embeddings.batch_embed_content_handler import (
    GoogleBatchEmbeddings,
)
from litellm.types.llms.vertex_ai import GeminiEmbeddingInput

FLATTEN = GoogleBatchEmbeddings._flatten_and_detect_file_refs  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini_embeddings/batch_embed_content_handler.py#L38-L47


class Input(TypedDict):
    inputs: list[GeminiEmbeddingInput]


class Detected(TypedDict):
    elements: list[str]
    has_file_refs: bool


def run(input_value: Input) -> dict[str, list[Detected]]:
    detected: list[Detected] = []
    for each in input_value["inputs"]:
        elements, has_file_refs = FLATTEN(each)
        detected.append({"elements": elements, "has_file_refs": has_file_refs})
    return {"calls": detected}
