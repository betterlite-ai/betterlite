from typing import Literal

from pydantic import JsonValue
from typing_extensions import TypedDict


class CompressionInterceptionConfig(TypedDict, total=False):
    enabled: bool
    compression_trigger: int
    compression_target: int | None
    embedding_model: str | None
    embedding_model_params: dict[str, JsonValue] | None


class CompressionSavingsMetadata(TypedDict):
    tokens_before: int
    tokens_after: int
    tokens_saved: int
    source: Literal["compression_interception"]
