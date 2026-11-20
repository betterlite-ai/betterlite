from typing import Literal, TypeAlias, TypedDict

from pydantic import JsonValue


class SystemStringTargetInput(TypedDict):
    kind: Literal["system_string"]


class SystemBlockTextTargetInput(TypedDict):
    kind: Literal["system_block_text"]
    block_idx: int


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


MessageTextTargetInput: TypeAlias = (
    ContentBlockTextTargetInput
    | MessageContentTargetInput
    | ToolResultBlockTextTargetInput
    | ToolResultStringTargetInput
)
TargetInput: TypeAlias = (
    MessageTextTargetInput | SystemBlockTextTargetInput | SystemStringTargetInput
)


class ToolTarget(TypedDict):
    msg_idx: int
    content_idx: int


class ScannedTextItem(TypedDict):
    text: str
    target: TargetInput


class ScannedCallItem(TypedDict):
    tool_call: dict[str, JsonValue]
    tool_target: ToolTarget


class SystemBlockScenario(TypedDict):
    system: JsonValue
    block_idx: int
    response: str


class MessageTextScenario(TypedDict):
    message: dict[str, JsonValue]
    target: MessageTextTargetInput
    response: str


class ResponsesToInputScenario(TypedDict):
    data: dict[str, JsonValue]
    responses: list[str]
    scanned_texts: list[ScannedTextItem]


class ToolUseScenario(TypedDict):
    message: dict[str, JsonValue]
    tool_target: ToolTarget
    shape_name: str | None
    arguments: str


class ToolCallsScenario(TypedDict):
    messages: list[JsonValue]
    scanned_calls: list[ScannedCallItem]
    pre: list[JsonValue]
    returned: list[JsonValue] | None
    guardrail_name: str | None


class DataScenario(TypedDict):
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


class SystemBlockRequest(TypedDict):
    function: Literal["system_block"]
    scenarios: list[SystemBlockScenario]


class MessageTextRequest(TypedDict):
    function: Literal["message_text"]
    scenarios: list[MessageTextScenario]


class ResponsesToInputRequest(TypedDict):
    function: Literal["responses_to_input"]
    scenarios: list[ResponsesToInputScenario]


class ToolUseRequest(TypedDict):
    function: Literal["tool_use"]
    scenarios: list[ToolUseScenario]


class ToolCallsRequest(TypedDict):
    function: Literal["tool_calls"]
    scenarios: list[ToolCallsScenario]


class HoistedRequest(TypedDict):
    function: Literal["hoisted"]
    scenarios: list[DataScenario]


class SystemConversionRequest(TypedDict):
    function: Literal["system_conversion"]
    scenarios: list[SystemConversionScenario]


class FoldRequest(TypedDict):
    function: Literal["fold"]
    scenarios: list[FoldScenario]


class DeferRequest(TypedDict):
    function: Literal["defer"]
    scenarios: list[DeferScenario]


class StructuredRequest(TypedDict):
    function: Literal["structured"]
    scenarios: list[StructuredScenario]


Input: TypeAlias = (
    DeferRequest
    | FoldRequest
    | HoistedRequest
    | MessageTextRequest
    | ResponsesToInputRequest
    | StructuredRequest
    | SystemBlockRequest
    | SystemConversionRequest
    | ToolCallsRequest
    | ToolUseRequest
)
