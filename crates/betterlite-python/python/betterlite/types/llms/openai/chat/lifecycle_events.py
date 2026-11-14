from typing import Literal, TypeAlias

from pydantic import JsonValue, PrivateAttr

from betterlite.types.llms.base import BaseLiteLLMOpenAIResponseObject
from betterlite.types.llms.openai.chat.event_types import ResponsesAPIStreamEvents
from betterlite.types.responses import ResponsesAPIResponse


# ast-grep-ignore: no-long-names
class ResponseCreatedEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.RESPONSE_CREATED]
    response: ResponsesAPIResponse


# ast-grep-ignore: no-long-names
class ResponseInProgressEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.RESPONSE_IN_PROGRESS]
    response: ResponsesAPIResponse


# ast-grep-ignore: no-long-names
class ResponseCompletedEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.RESPONSE_COMPLETED]
    response: ResponsesAPIResponse
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)


# ast-grep-ignore: no-long-names
class ResponseFailedEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.RESPONSE_FAILED]
    response: ResponsesAPIResponse


# ast-grep-ignore: no-long-names
class ResponseIncompleteEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.RESPONSE_INCOMPLETE]
    response: ResponsesAPIResponse


ResponsesTerminalEvent: TypeAlias = (
    ResponseCompletedEvent | ResponseIncompleteEvent | ResponseFailedEvent
)
