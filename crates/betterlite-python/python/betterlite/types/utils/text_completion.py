import time
from collections.abc import Mapping
from typing import ClassVar, Literal, TypeAlias

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.choices import FinishReason, map_finish_reason
from betterlite.types.utils.response import _generate_id
from betterlite.types.utils.usage import Usage
from betterlite.types.utils.values import JsonValue, StoredValue, ValidationError

LOGPROBS_FIELDS = ("text_offset", "token_logprobs", "tokens", "top_logprobs")


class Logprobs(ResponseObject):
    text_offset: list[int] | None
    token_logprobs: list[float | None] | None
    tokens: list[str] | None
    top_logprobs: list[dict[str, float] | None] | None

    @override
    def __init__(self, **data: StoredValue) -> None:
        missing = [name for name in LOGPROBS_FIELDS if name not in data]
        if missing:
            raise ValidationError("Logprobs", missing)
        super().__init__(**data)


class TextChoices(ResponseObject):
    finish_reason: FinishReason | None
    index: int | None
    text: str | None
    logprobs: Logprobs | None

    @override
    def __init__(
        self, finish_reason: StoredValue = None, index: StoredValue = 0,
        text: StoredValue = None, logprobs: StoredValue = None,
        **params: StoredValue,
    ) -> None:
        data: dict[str, StoredValue] = dict(params)
        match finish_reason:
            case None:
                data["finish_reason"] = None
            case str():
                data["finish_reason"] = map_finish_reason(finish_reason)
            case _other:
                raise ValidationError("TextChoices", ("finish_reason",), "wrong type")
        data["index"] = index
        data["text"] = text
        data["logprobs"] = text_logprobs(logprobs)
        super().__init__(**data)


class TextCompletionResponse(ResponseObject):
    id: str
    object: str
    created: int
    model: str | None
    choices: list[TextChoices]
    usage: Usage | None
    _response_ms: int | None
    _hidden_params: dict[str, StoredValue]
    defaults: ClassVar[dict[str, StoredValue]] = {"_response_ms": None, "_hidden_params": {}}

    @override
    def __init__(
        self, id: StoredValue = None, choices: StoredValue = None,
        created: StoredValue = None, model: StoredValue = None,
        usage: StoredValue = None, stream: StoredValue = False,
        response_ms: int | None = None, object: StoredValue = None,
        **params: StoredValue,
    ) -> None:
        data: dict[str, StoredValue] = dict(params)
        if stream:
            selected: list[StoredValue] = [TextChoices()]
            kind = "text_completion.chunk"
        else:
            kind = "text_completion"
            match choices:
                case list():
                    selected = [text_choice(choice) for choice in choices]
                case _other:
                    selected = [TextChoices()]
        data.update(
            id=_generate_id() if id is None else id,
            object=kind,
            created=int(time.time()) if created is None else created,
            model=model,
            choices=selected,
            usage=Usage() if usage is None else usage,
        )
        super().__init__(**data)
        self._response_ms = response_ms


def text_choice(choice: StoredValue) -> StoredValue:
    match choice:
        case TextChoices():
            return choice
        case Mapping():
            return TextChoices(**choice)
        case _other:
            return choice


def text_logprobs(logprobs: StoredValue) -> StoredValue:
    match logprobs:
        case Mapping():
            return Logprobs(**logprobs)
        case _other:
            return logprobs


class InputTextContent(ResponseObject):
    text: str
    type: Literal["input_text"]
    prompt_cache_breakpoint: JsonValue


class OutputTextContent(ResponseObject):
    text: str
    type: Literal["output_text"]


class InputImageContent(ResponseObject):
    image_url: str
    type: Literal["input_image"]
    detail: str | None


class InputAudioContent(ResponseObject):
    input_audio: JsonValue
    type: Literal["input_audio"]


ContentItem: TypeAlias = (
    str | InputTextContent | OutputTextContent | InputImageContent | InputAudioContent
)
GraderContent: TypeAlias = ContentItem | list[ContentItem]


def content_item(value: StoredValue) -> StoredValue:
    match value:
        case list():
            return [content_item(item) for item in value]
        case Mapping() if "type" not in value:
            raise ValidationError("content", ("type",), "value is required")
        case Mapping():
            selected: ResponseObject
            match value["type"]:
                case "input_text":
                    selected = InputTextContent(**value)
                case "output_text":
                    selected = OutputTextContent(**value)
                case "input_image":
                    selected = InputImageContent(**value)
                case "input_audio":
                    selected = InputAudioContent(**value)
                case _other:
                    raise ValidationError("content", ("type",), "unknown content type")
            return selected
        case _other:
            return value


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
