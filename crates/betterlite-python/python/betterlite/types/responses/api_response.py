from collections.abc import Mapping, Sequence
from typing import ClassVar, Literal, TypeAlias

from pydantic import (
    ConfigDict,
    JsonValue,
    NonNegativeInt,
    PrivateAttr,
    field_validator,
)
from typing_extensions import Unpack, override

from betterlite.types.responses.main import (
    BaseResponseOutputItem,
    CustomToolCallOutputItem,
    GenericResponseOutputItem,
    OutputCodeInterpreterCall,
    OutputFunctionToolCall,
    OutputImageGenerationCall,
    ResponseReasoningItem,
)
from betterlite.types.utils.base import Dumpable, DumpOptions, InitValue, ResponseObject

ResponsesAPIStatus = Literal[
    "completed", "failed", "in_progress", "cancelled", "queued", "incomplete",
]


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
            case dict():
                return value.get("total_cost")
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


def _raw_string(item: ResponseObject, name: str) -> str | None:
    match item.model_dump().get(name):
        case str() as value:
            return value
        case bool() | int() | float() | list() | dict() | None:
            return None


class RawOutputContent(ResponseObject):
    @property
    def type(self) -> str | None:
        return _raw_string(self, "type")

    @property
    def text(self) -> str | None:
        return _raw_string(self, "text")


def _content_items(value: JsonValue | None) -> list[RawOutputContent]:
    items: list[RawOutputContent] = []
    match value:
        case list():
            for entry in value:
                match entry:
                    case dict():
                        items.append(RawOutputContent.model_validate(entry))
                    case bool() | int() | float() | str() | list() | None:
                        pass
        case bool() | int() | float() | str() | dict() | None:
            pass
    return items


class RawResponseOutputItem(BaseResponseOutputItem):
    @property
    def type(self) -> str | None:
        return _raw_string(self, "type")

    @property
    def content(self) -> list[RawOutputContent]:
        return _content_items(self.model_dump().get("content"))


OutputItem: TypeAlias = (
    GenericResponseOutputItem
    | OutputFunctionToolCall
    | OutputImageGenerationCall
    | OutputCodeInterpreterCall
    | CustomToolCallOutputItem
    | ResponseReasoningItem
    | RawResponseOutputItem
    | ResponseObject
)


def _dump_entry(item: OutputItem, entry: JsonValue) -> JsonValue:
    match item:
        case BaseResponseOutputItem():
            return item.dump_entry(entry)
        case ResponseObject():
            return entry


class ResponsesAPIResponse(ResponseObject):
    id: str
    created_at: int
    error: dict[str, JsonValue] | None = None
    incomplete_details: dict[str, JsonValue] | None = None
    instructions: str | None = None
    metadata: dict[str, JsonValue] | None = None
    model: str | None = None
    object: str | None = None
    output: list[OutputItem]
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
            case Dumpable() as model:
                return model.model_dump()
            case Mapping():
                return value
            case bool() | int() | float() | str() | Sequence() | None:
                return value

    @override
    def model_dump(self, **kwargs: Unpack[DumpOptions]) -> dict[str, JsonValue]:
        dumped = super().model_dump(**kwargs)
        output = dumped.get("output")
        match output:
            case list():
                dumped["output"] = [
                    _dump_entry(item, entry)
                    for item, entry in zip(self.output, output, strict=False)
                ]
            case bool() | int() | float() | str() | dict() | None:
                pass
        return dumped

    @property
    def output_text(self) -> str:
        texts: list[str] = []
        for item in self.output:
            match item:
                case GenericResponseOutputItem() | RawResponseOutputItem():
                    if item.type == "message":
                        for content in item.content:
                            text = content.text
                            if content.type == "output_text" and text is not None:
                                texts.append(text)
                case (
                    OutputFunctionToolCall() | OutputImageGenerationCall()
                    | OutputCodeInterpreterCall() | CustomToolCallOutputItem()
                    | ResponseReasoningItem()
                ):
                    pass
                case ResponseObject():
                    pass
        return "".join(texts)
