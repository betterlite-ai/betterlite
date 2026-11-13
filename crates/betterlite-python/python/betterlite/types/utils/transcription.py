from typing import ClassVar, Literal

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.values import StoredValue


class TranscriptionUsageDurationObject(ResponseObject):  # ast-grep-ignore: no-long-names
    type: Literal["duration"]
    seconds: float


# ast-grep-ignore: no-long-names
class TranscriptionUsageInputTokenDetailsObject(ResponseObject):
    audio_tokens: int
    text_tokens: int


class TranscriptionUsageTokensObject(ResponseObject):
    type: Literal["tokens"]
    input_tokens: int
    output_tokens: int
    total_tokens: int
    # ast-grep-ignore: no-long-names
    input_token_details: TranscriptionUsageInputTokenDetailsObject


class TranscriptionResponse(ResponseObject):
    text: str | None
    # ast-grep-ignore: no-long-names
    usage: TranscriptionUsageDurationObject | TranscriptionUsageTokensObject | None
    _hidden_params: dict[str, StoredValue]
    _response_headers: dict[str, StoredValue] | None
    defaults: ClassVar[dict[str, StoredValue]] = {
        "_hidden_params": {}, "_response_headers": None,
    }

    @override
    def __init__(self, text: str | None = None) -> None:
        super().__init__(text=text)
