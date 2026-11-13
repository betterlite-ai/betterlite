from collections.abc import Collection
from typing import ClassVar, Final, Literal

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.values import (
    JsonValue,
    ModelObject,
    StoredValue,
    ValidationError,
    field_validator,
)

ResponsesAPIStatus = Literal[
    "completed", "failed", "in_progress", "cancelled", "queued", "incomplete",
]
DROPPED_REASONING_KEYS: Final[tuple[str, ...]] = ("status", "content", "encrypted_content")


class CachedTokensDetails(ResponseObject):
    text_tokens: int | None
    audio_tokens: int | None
    image_tokens: int | None

    @override
    def __init__(self, **data: StoredValue) -> None:
        super().__init__(
            **{name: value for name, value in data.items() if name in type(self).__fields__},
        )


class OutputTokensDetails(ResponseObject):
    audio_tokens: int | None
    reasoning_tokens: int | None
    text_tokens: int | None


class InputTokensDetails(ResponseObject):
    audio_tokens: int | None
    cached_tokens: int
    cached_tokens_details: CachedTokensDetails | None
    image_tokens: int | None
    text_tokens: int | None
    video_tokens: int | None
    defaults: ClassVar[dict[str, StoredValue]] = {"cached_tokens": 0}


class ResponseAPIUsage(ResponseObject):
    input_tokens: int
    input_tokens_details: InputTokensDetails | None
    output_tokens: int
    output_tokens_details: OutputTokensDetails | None
    total_tokens: int
    cost: float | None

    @field_validator("cost", mode="before")
    def _cost_total(self, value: StoredValue) -> StoredValue:
        match value:
            case dict():
                return value.get("total_cost")
            case _other:
                return value


class _FrozenObject(ResponseObject):
    @override
    def __init__(self, **data: StoredValue) -> None:
        super().__init__(
            **{name: value for name, value in data.items() if name in type(self).__fields__},
        )

    @override
    def __setattr__(self, name: str, value: StoredValue) -> None:
        if name in type(self).__fields__:
            raise ValidationError(type(self).__name__, (name,), "frozen")
        super().__setattr__(name, value)


class WebSearchToolUsage(_FrozenObject):
    num_requests: int

    @field_validator("num_requests", mode="before")
    def _non_negative(self, value: StoredValue) -> StoredValue:
        match value:
            case int() if value < 0:
                raise ValueError("num_requests must not be negative")
            case _other:
                return value


class ResponsesToolUsage(_FrozenObject):
    web_search: WebSearchToolUsage | None


def _field(item: StoredValue, name: str) -> StoredValue:
    match item:
        case ResponseObject():
            return item.get(name)
        case dict():
            return item.get(name)
        case _other:
            return None


def _strip_reasoning_nulls(item: JsonValue) -> JsonValue:
    match item:
        case dict():
            if item.get("type") != "reasoning":
                return item
            return {
                key: value for key, value in item.items()
                if value is not None or key not in DROPPED_REASONING_KEYS
            }
        case _other:
            return item


def _without_reasoning_nulls(dumped: dict[str, JsonValue]) -> dict[str, JsonValue]:
    output = dumped.get("output")
    match output:
        case list():
            dumped["output"] = [_strip_reasoning_nulls(item) for item in output]
        case _other:
            pass
    return dumped


class ResponsesAPIResponse(ResponseObject):
    id: str
    created_at: int
    error: dict[str, JsonValue] | None
    incomplete_details: dict[str, JsonValue] | None
    instructions: str | None
    metadata: dict[str, JsonValue] | None
    model: str | None
    object: str | None
    output: list[dict[str, JsonValue] | ResponseObject]
    parallel_tool_calls: bool | None
    temperature: float | None
    tool_choice: JsonValue
    tools: JsonValue
    top_p: float | None
    max_output_tokens: int | None
    previous_response_id: str | None
    reasoning: dict[str, JsonValue] | None
    status: str | None
    text: JsonValue
    truncation: Literal["auto", "disabled"] | None
    usage: ResponseAPIUsage | None
    user: str | None
    store: bool | None
    _hidden_params: dict[str, StoredValue]
    defaults: ClassVar[dict[str, StoredValue]] = {"_hidden_params": {}}

    @field_validator("reasoning", mode="before")
    def _reasoning_mapping(self, value: StoredValue) -> StoredValue:
        match value:
            case ModelObject():
                return value.model_dump()
            case _other:
                return value

    @field_validator("usage", mode="before")
    def _usage_object(self, value: StoredValue) -> StoredValue:
        match value:
            case dict():
                return ResponseAPIUsage(**value)
            case _other:
                return value

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
                return _without_reasoning_nulls(dumped)
            case _other:
                return dumped

    @property
    def output_text(self) -> str:
        texts: list[str] = []
        for output_item in self.output:
            if _field(output_item, "type") != "message":
                continue
            content = _field(output_item, "content")
            match content:
                case list():
                    for content_item in content:
                        if _field(content_item, "type") == "output_text":
                            text = _field(content_item, "text")
                            match text:
                                case str():
                                    texts.append(text)
                                case _other:
                                    texts.append("")
                case _other:
                    pass
        return "".join(texts)
