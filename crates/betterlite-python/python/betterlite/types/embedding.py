from typing import ClassVar

from pydantic import ConfigDict, Field, JsonValue

from betterlite.types.utils.base import ResponseObject


class EmbeddingRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")

    model: str
    input: list[str] = Field(default_factory=list)
    timeout: int = 600
    api_base: str | None = None
    api_version: str | None = None
    api_key: str | None = None
    api_type: str | None = None
    caching: bool = False
    user: str | None = None
    custom_llm_provider: str | dict[str, JsonValue] | None = None
    litellm_call_id: str | None = None
    litellm_logging_obj: dict[str, JsonValue] | None = None
    logger_fn: str | None = None
