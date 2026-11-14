from typing import Literal

from betterlite.types.llms.base import BaseLiteLLMOpenAIResponseObject
from betterlite.types.llms.openai.chat.event_types import ResponsesAPIStreamEvents


# ast-grep-ignore: no-long-names
class MCPListToolsInProgressEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.MCP_LIST_TOOLS_IN_PROGRESS]
    sequence_number: int
    output_index: int
    item_id: str


# ast-grep-ignore: no-long-names
class MCPListToolsCompletedEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.MCP_LIST_TOOLS_COMPLETED]
    sequence_number: int
    output_index: int
    item_id: str


# ast-grep-ignore: no-long-names
class MCPListToolsFailedEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.MCP_LIST_TOOLS_FAILED]
    sequence_number: int
    output_index: int
    item_id: str


# ast-grep-ignore: no-long-names
class MCPCallInProgressEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.MCP_CALL_IN_PROGRESS]
    sequence_number: int
    output_index: int
    item_id: str


# ast-grep-ignore: no-long-names
class MCPCallArgumentsDeltaEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.MCP_CALL_ARGUMENTS_DELTA]
    output_index: int
    item_id: str
    delta: str
    sequence_number: int


# ast-grep-ignore: no-long-names
class MCPCallArgumentsDoneEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.MCP_CALL_ARGUMENTS_DONE]
    output_index: int
    item_id: str
    arguments: str
    sequence_number: int


# ast-grep-ignore: no-long-names
class MCPCallCompletedEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.MCP_CALL_COMPLETED]
    sequence_number: int
    item_id: str
    output_index: int


# ast-grep-ignore: no-long-names
class MCPCallFailedEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.MCP_CALL_FAILED]
    sequence_number: int
    item_id: str
    output_index: int
