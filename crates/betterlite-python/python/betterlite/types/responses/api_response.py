from collections.abc import Mapping, Sequence
from typing import ClassVar, Final, Literal, Unpack

from pydantic import (
    ConfigDict,
    JsonValue,
    NonNegativeInt,
    PrivateAttr,
    SerializeAsAny,
    field_validator,
)
from typing_extensions import override

from betterlite.types.utils.base import Dumpable, DumpOptions, InitValue, ResponseObject

ResponsesAPIStatus = Literal[
    "completed", "failed", "in_progress", "cancelled", "queued", "incomplete",
]
DROPPED_REASONING_KEYS: Final[tuple[str, ...]] = ("status", "content", "encrypted_content")


class CachedTokensDetails(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    text_tokens: int | None = None
    audio_tokens: int | None = None
    image_tokens: int | None = None

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)


class OutputTokensDetails(ResponseObject):
    audio_tokens: int | None = None
    reasoning_tokens: int | None = None
    text_tokens: int | None = None

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)


class InputTokensDetails(ResponseObject):
    audio_tokens: int | None = None
    cached_tokens: int = 0
    cached_tokens_details: CachedTokensDetails | None = None
    image_tokens: int | None = None
    text_tokens: int | None = None
    video_tokens: int | None = None

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)


class ResponseAPIUsage(ResponseObject):
    input_tokens: int
    input_tokens_details: InputTokensDetails | None = None
    output_tokens: int
    output_tokens_details: OutputTokensDetails | None = None
    total_tokens: int
    cost: float | None = None

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)

    @field_validator("cost", mode="before")
    @classmethod
    def parse_cost(cls, value: JsonValue) -> JsonValue:
        match value:
            case {"total_cost": total_cost}:
                return total_cost
            case dict():
                return None
            case bool() | int() | float() | str() | list() | None:
                return value


class WebSearchToolUsage(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore", frozen=True)

    num_requests: NonNegativeInt

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)


class ResponsesToolUsage(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore", frozen=True)

    web_search: WebSearchToolUsage | None = None

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)


def _field(item: InitValue, name: str) -> InitValue:
    match item:
        case ResponseObject():
            return item.get(name)
        case Mapping() as mapping:
            return mapping.get(name)
        case Dumpable() | bool() | int() | float() | str() | Sequence() | None:
            return None


def _strip_reasoning_nulls(item: JsonValue) -> JsonValue:
    match item:
        case {"type": "reasoning"}:
            kept: dict[str, JsonValue] = {}
            for key, value in item.items():
                if value is None and key in DROPPED_REASONING_KEYS:
                    continue
                kept[key] = value
            return kept
        case dict() | bool() | int() | float() | str() | list() | None:
            return item


def _without_reasoning_nulls(dumped: dict[str, JsonValue]) -> dict[str, JsonValue]:
    output = dumped.get("output")
    match output:
        case list():
            dumped["output"] = [_strip_reasoning_nulls(item) for item in output]
        case bool() | int() | float() | str() | dict() | None:
            pass
    return dumped


class ResponsesAPIResponse(ResponseObject):
    id: str
    created_at: int
    error: dict[str, JsonValue] | None = None
    incomplete_details: dict[str, JsonValue] | None = None
    instructions: str | None = None
    metadata: dict[str, JsonValue] | None = None
    model: str | None = None
    object: str | None = None
    output: list[dict[str, JsonValue] | SerializeAsAny[ResponseObject]]
    parallel_tool_calls: bool | None = None
    temperature: float | None = None
    tool_choice: JsonValue = None
    tools: JsonValue = None
    top_p: float | None = None
    max_output_tokens: int | None = None
    previous_response_id: str | None = None
    reasoning: dict[str, JsonValue] | None = None
    status: str | None = None
    text: JsonValue = None
    truncation: Literal["auto", "disabled"] | None = None
    usage: ResponseAPIUsage | None = None
    user: str | None = None
    store: bool | None = None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)

    @field_validator("reasoning", mode="before")
    @classmethod
    def validate_reasoning_to_dict(cls, value: InitValue) -> InitValue:
        match value:
            case Mapping() | bool() | int() | float() | str() | Sequence() | None:
                return value
            case Dumpable() as model:
                return model.model_dump()

    @override
    def model_dump(self, **kwargs: Unpack[DumpOptions]) -> dict[str, JsonValue]:
        return _without_reasoning_nulls(super().model_dump(**kwargs))

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
                        if _field(content_item, "type") != "output_text":
                            continue
                        text = _field(content_item, "text")
                        match text:
                            case str():
                                texts.append(text)
                            case (
                                Dumpable() | Mapping() | bool() | int() | float()
                                | Sequence() | None
                            ):
                                texts.append("")
                case Dumpable() | Mapping() | bool() | int() | float() | str() | Sequence() | None:
                    pass
        return "".join(texts)
