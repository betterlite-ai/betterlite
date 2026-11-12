import time
from collections.abc import Collection, Mapping
from typing import ClassVar, Literal

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


class HiddenParams(ResponseObject):
    original_response: JsonValue
    model_id: str | None
    api_base: str | None
    response_cost: float | None
    _response_ms: float | None
    defaults: ClassVar[dict[str, StoredValue]] = {"_response_ms": None}

    @override
    def model_dump(
        self, *, mode: Literal["python", "json"] = "python",
        include: Collection[str] | None = None, exclude: Collection[str] | None = None,
        exclude_unset: bool = False, exclude_none: bool = False,
    ) -> JsonValue:
        dumped = super().model_dump(
            mode=mode, include=include, exclude=exclude,
            exclude_unset=exclude_unset, exclude_none=exclude_none,
        )
        match dumped:
            case dict():
                dumped["_response_ms"] = self._response_ms
                return dumped
            case _other:
                return dumped


class TextCompletionResponse(ResponseObject):
    id: str
    object: str
    created: int
    model: str | None
    choices: list[TextChoices]
    usage: Usage | None
    _response_ms: int | None
    _hidden_params: HiddenParams
    defaults: ClassVar[dict[str, StoredValue]] = {
        "_response_ms": None, "_hidden_params": HiddenParams(),
    }

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
