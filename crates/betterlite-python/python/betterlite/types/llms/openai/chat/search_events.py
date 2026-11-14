from typing import Literal

from betterlite.types.llms.base import BaseLiteLLMOpenAIResponseObject
from betterlite.types.llms.openai.chat.event_types import ResponsesAPIStreamEvents


# ast-grep-ignore: no-long-names
class FileSearchCallInProgressEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.FILE_SEARCH_CALL_IN_PROGRESS]
    output_index: int
    item_id: str


# ast-grep-ignore: no-long-names
class FileSearchCallSearchingEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.FILE_SEARCH_CALL_SEARCHING]
    output_index: int
    item_id: str


# ast-grep-ignore: no-long-names
class FileSearchCallCompletedEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.FILE_SEARCH_CALL_COMPLETED]
    output_index: int
    item_id: str


# ast-grep-ignore: no-long-names
class WebSearchCallInProgressEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.WEB_SEARCH_CALL_IN_PROGRESS]
    output_index: int
    item_id: str


# ast-grep-ignore: no-long-names
class WebSearchCallSearchingEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.WEB_SEARCH_CALL_SEARCHING]
    output_index: int
    item_id: str


# ast-grep-ignore: no-long-names
class WebSearchCallCompletedEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.WEB_SEARCH_CALL_COMPLETED]
    output_index: int
    item_id: str
