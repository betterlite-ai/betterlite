import time
from collections.abc import Sequence
from typing import ClassVar, Unpack

from pydantic import JsonValue, PrivateAttr
from typing_extensions import override

from betterlite.types.utils.base import (
    DictJsonObject,
    DumpOptions,
    InitValue,
    ModelValue,
    ResponseObject,
    dump_value,
    validated_value,
)
from betterlite.types.utils.choices import FinishReason, map_finish_reason
from betterlite.types.utils.response import _generate_id
from betterlite.types.utils.usage import Usage


class Logprobs(ResponseObject):
    text_offset: list[int] | None
    token_logprobs: list[float | None] | None
    tokens: list[str] | None
    top_logprobs: list[dict[str, float] | None] | None


class TextChoices(DictJsonObject):
    finish_reason: ClassVar[FinishReason | None]
    index: ClassVar[int | None]
    text: ClassVar[str | None]
    logprobs: ClassVar[Logprobs | None]

    @override
    def __init__(
        self, finish_reason: JsonValue = None, index: JsonValue = 0,
        text: JsonValue = None, logprobs: JsonValue = None,
        **params: InitValue,
    ) -> None:
        mapped: JsonValue = None
        if finish_reason is not None and finish_reason != "":
            match finish_reason:
                case str():
                    mapped = map_finish_reason(finish_reason)
                case bool() | int() | float() | list() | dict():
                    mapped = finish_reason
        super().__init__(
            finish_reason=mapped, index=index, text=text,
            logprobs=text_logprobs(logprobs), **params,
        )


class HiddenParams(DictJsonObject):
    original_response: JsonValue = None
    model_id: str | None = None
    api_base: str | None = None
    response_cost: float | None = None
    _response_ms: float | None = PrivateAttr(default=None)

    @override
    def model_dump(self, **kwargs: Unpack[DumpOptions]) -> dict[str, JsonValue]:
        dumped: dict[str, JsonValue] = super().model_dump(**kwargs)
        dumped["_response_ms"] = self._response_ms
        return dumped


class TextCompletionResponse(ResponseObject):
    id: str
    object: str
    created: int
    model: str | None
    choices: list[TextChoices]
    usage: Usage | None
    _response_ms: int | None = PrivateAttr(default=None)
    _hidden_params: HiddenParams = PrivateAttr(default_factory=HiddenParams)

    @override
    def __init__(
        self, id: JsonValue = None, choices: Sequence[ModelValue] | None = None,
        created: JsonValue = None, model: JsonValue = None,
        usage: ModelValue = None, stream: JsonValue = False,
        response_ms: int | None = None, object: JsonValue = None,
        **params: InitValue,
    ) -> None:
        if stream:
            kind = "text_completion.chunk"
            selected = [TextChoices()]
        else:
            kind = "text_completion"
            selected = text_choices(choices)
        super().__init__(
            id=_generate_id() if id is None else id, object=kind,
            created=int(time.time()) if created is None else created, model=model,
            choices=selected, usage=Usage() if usage is None else dump_value(usage),
            **params,
        )
        self._response_ms = response_ms


def text_choices(choices: Sequence[ModelValue] | None) -> list[TextChoices]:
    if choices is None:
        return [TextChoices()]
    match choices:
        case str():
            return [TextChoices()]
        case Sequence():
            return [validated_value(choice, TextChoices) for choice in choices]


def text_logprobs(logprobs: JsonValue) -> ModelValue:
    if logprobs is None:
        return None
    return validated_value(logprobs, Logprobs)
