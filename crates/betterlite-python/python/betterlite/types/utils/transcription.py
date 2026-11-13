from typing import ClassVar, Literal

from pydantic import ConfigDict, JsonValue, PrivateAttr
from typing_extensions import override

from betterlite.types.utils.base import DictJsonObject, ResponseObject


class TranscriptionUsageDurationObject(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    type: Literal["duration"]
    seconds: float


# ast-grep-ignore: no-long-names
class TranscriptionUsageInputTokenDetailsObject(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    audio_tokens: int
    text_tokens: int


class TranscriptionUsageTokensObject(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    type: Literal["tokens"]
    input_tokens: int
    output_tokens: int
    total_tokens: int
    # ast-grep-ignore: no-long-names
    input_token_details: TranscriptionUsageInputTokenDetailsObject


class TranscriptionResponse(DictJsonObject):
    text: str | None = None
    # ast-grep-ignore: no-long-names
    usage: TranscriptionUsageDurationObject | TranscriptionUsageTokensObject | None = None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)
    _response_headers: dict[str, JsonValue] | None = PrivateAttr(default=None)

    @override
    def __init__(self, text: JsonValue = None) -> None:
        super().__init__(text=text)
