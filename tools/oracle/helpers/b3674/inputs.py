from collections.abc import Coroutine, Mapping, Sequence
from typing import Literal, Protocol, TypedDict

from litellm.llms.anthropic.chat.guardrail_translation.handler import ScannedText
from litellm.types.llms.openai import ChatCompletionToolCallChunk
from pydantic import JsonValue


class ResponsesWriter(Protocol):
    def __call__(
        self,
        data: dict[str, JsonValue],
        responses: Sequence[str],
        scanned: tuple[ScannedText, ...],
    ) -> Coroutine[JsonValue, JsonValue, None]: ...


class HoistedReader(Protocol):
    def __call__(self, data: Mapping[str, JsonValue]) -> dict[str, JsonValue] | None: ...


class SystemRowConverter(Protocol):
    def __call__(self, message: Mapping[str, JsonValue]) -> dict[str, JsonValue] | None: ...


class Deferrer(Protocol):
    def __call__(self, messages: list[JsonValue]) -> list[JsonValue]: ...


class MessageContentTargetInput(TypedDict):
    kind: Literal["message_content"]
    msg_idx: int


class ContentBlockTextTargetInput(TypedDict):
    kind: Literal["content_block_text"]
    msg_idx: int
    content_idx: int


class ToolResultStringTargetInput(TypedDict):
    kind: Literal["tool_result_string"]
    msg_idx: int
    content_idx: int


class ToolResultBlockTextTargetInput(TypedDict):
    kind: Literal["tool_result_block_text"]
    msg_idx: int
    content_idx: int
    block_idx: int


class SystemStringTargetInput(TypedDict):
    kind: Literal["system_string"]


class SystemBlockTextTargetInput(TypedDict):
    kind: Literal["system_block_text"]
    block_idx: int


MessageTargetInput = (
    MessageContentTargetInput
    | ContentBlockTextTargetInput
    | ToolResultStringTargetInput
    | ToolResultBlockTextTargetInput
)

InputTargetInput = (
    SystemStringTargetInput
    | SystemBlockTextTargetInput
    | MessageContentTargetInput
    | ContentBlockTextTargetInput
    | ToolResultStringTargetInput
    | ToolResultBlockTextTargetInput
)


class ToolUseTargetInput(TypedDict):
    msg_idx: int
    content_idx: int


class SystemBlockScenario(TypedDict):
    system: JsonValue
    block_idx: int
    response: str


class MessageTextScenario(TypedDict):
    message: dict[str, JsonValue]
    target: MessageTargetInput
    response: str


class ScannedTextInput(TypedDict):
    text: str
    target: InputTargetInput


class ResponsesScenario(TypedDict):
    data: dict[str, JsonValue]
    responses: list[str]
    scanned: list[ScannedTextInput]


class ToolUseScenario(TypedDict):
    message: dict[str, JsonValue]
    target: ToolUseTargetInput
    shape_name: str | None
    arguments: str


class ScannedCallInput(TypedDict):
    tool_call: ChatCompletionToolCallChunk
    target: ToolUseTargetInput


class ToolCallsScenario(TypedDict):
    messages: list[JsonValue]
    scanned: list[ScannedCallInput]
    pre: list[JsonValue]
    returned: list[JsonValue] | None
    guardrail_name: str | None


class HoistedScenario(TypedDict):
    data: dict[str, JsonValue]


class SystemConversionScenario(TypedDict):
    message: dict[str, JsonValue]


class FoldScenario(TypedDict):
    data: dict[str, JsonValue]
    leading: list[JsonValue]
    include_existing: bool


class DeferScenario(TypedDict):
    messages: list[JsonValue]


class StructuredScenario(TypedDict):
    data: dict[str, JsonValue]
    messages: list[JsonValue]
    preserve_system_messages: bool
    compute_hoisted: bool


class SystemBlockCall(TypedDict):
    function: Literal["system_block"]
    scenarios: list[SystemBlockScenario]


class MessageTextCall(TypedDict):
    function: Literal["message_text"]
    scenarios: list[MessageTextScenario]


class ResponsesToInputCall(TypedDict):
    function: Literal["responses_to_input"]
    scenarios: list[ResponsesScenario]


class ToolUseCall(TypedDict):
    function: Literal["tool_use"]
    scenarios: list[ToolUseScenario]


class ToolCallsCall(TypedDict):
    function: Literal["tool_calls"]
    scenarios: list[ToolCallsScenario]


class HoistedCall(TypedDict):
    function: Literal["hoisted"]
    scenarios: list[HoistedScenario]


class SystemConversionCall(TypedDict):
    function: Literal["system_conversion"]
    scenarios: list[SystemConversionScenario]


class FoldCall(TypedDict):
    function: Literal["fold"]
    scenarios: list[FoldScenario]


class DeferCall(TypedDict):
    function: Literal["defer"]
    scenarios: list[DeferScenario]


class StructuredCall(TypedDict):
    function: Literal["structured"]
    scenarios: list[StructuredScenario]


Input = (
    SystemBlockCall
    | MessageTextCall
    | ResponsesToInputCall
    | ToolUseCall
    | ToolCallsCall
    | HoistedCall
    | SystemConversionCall
    | FoldCall
    | DeferCall
    | StructuredCall
)
