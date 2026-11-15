from typing import NotRequired

from pydantic import JsonValue
from typing_extensions import TypedDict


class CompressedResult(TypedDict):
    messages: list[dict[str, JsonValue]]  # stubs replace low-relevance messages
    original_tokens: int  # token count before compression
    compressed_tokens: int  # token count after compression
    compression_ratio: float  # fraction reduced, e.g. 0.6 means 60% reduction
    cache: dict[str, str]  # key -> original content (for retrieval tool responses)
    tools: list[dict[str, JsonValue]]  # [litellm_content_retrieve tool definition]
    compression_skipped_reason: NotRequired[str]
