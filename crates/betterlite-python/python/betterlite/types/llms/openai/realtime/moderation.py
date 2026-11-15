from enum import StrEnum

from pydantic import JsonValue, PrivateAttr

from betterlite.types.llms.base import BaseLiteLLMOpenAIResponseObject


class ImageGenerationRequestQuality(StrEnum):
    LOW = "low"
    MEDIUM = "medium"
    HIGH = "high"
    AUTO = "auto"
    STANDARD = "standard"
    HD = "hd"


# ast-grep-ignore: no-long-names
class OpenAIModerationResult(BaseLiteLLMOpenAIResponseObject):
    categories: dict[str, JsonValue] | None
    category_applied_input_types: dict[str, JsonValue] | None
    category_scores: dict[str, JsonValue] | None
    flagged: bool | None


# ast-grep-ignore: no-long-names
class OpenAIModerationResponse(BaseLiteLLMOpenAIResponseObject):
    id: str
    model: str
    results: list[OpenAIModerationResult]
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)
