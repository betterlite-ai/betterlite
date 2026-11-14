from typing import Annotated, TypeAlias

from pydantic import Discriminator

from betterlite.types.llms.base import BaseLiteLLMOpenAIResponseObject
from betterlite.types.llms.openai.chat.lifecycle_events import (
    ResponseCompletedEvent,
    ResponseCreatedEvent,
    ResponseFailedEvent,
    ResponseIncompleteEvent,
    ResponseInProgressEvent,
)
from betterlite.types.llms.openai.chat.mcp_events import (
    MCPCallArgumentsDeltaEvent,
    MCPCallArgumentsDoneEvent,
    MCPCallCompletedEvent,
    MCPCallFailedEvent,
    MCPCallInProgressEvent,
    MCPListToolsCompletedEvent,
    MCPListToolsFailedEvent,
    MCPListToolsInProgressEvent,
)
from betterlite.types.llms.openai.chat.search_events import (
    FileSearchCallCompletedEvent,
    FileSearchCallInProgressEvent,
    FileSearchCallSearchingEvent,
    WebSearchCallCompletedEvent,
    WebSearchCallInProgressEvent,
    WebSearchCallSearchingEvent,
)
from betterlite.types.llms.openai.chat.streaming_events import (
    ContentPartAddedEvent,
    ContentPartDoneEvent,
    ErrorEvent,
    FunctionCallArgumentsDeltaEvent,
    FunctionCallArgumentsDoneEvent,
    ImageGenerationPartialImageEvent,
    OutputItemAddedEvent,
    OutputItemDoneEvent,
    OutputTextAnnotationAddedEvent,
    OutputTextDeltaEvent,
    OutputTextDoneEvent,
    ReasoningSummaryPartDoneEvent,
    ReasoningSummaryTextDeltaEvent,
    ReasoningSummaryTextDoneEvent,
    RefusalDeltaEvent,
    RefusalDoneEvent,
    ResponsePartAddedEvent,
)


# ast-grep-ignore: no-long-names
class GenericEvent(BaseLiteLLMOpenAIResponseObject):
    type: str


ResponsesAPIStreamingResponse: TypeAlias = Annotated[
    ResponseCreatedEvent
    | ResponseInProgressEvent
    | ResponseCompletedEvent
    | ResponseFailedEvent
    | ResponseIncompleteEvent
    | ResponsePartAddedEvent
    | ReasoningSummaryTextDeltaEvent
    | ReasoningSummaryTextDoneEvent
    | ReasoningSummaryPartDoneEvent
    | OutputItemAddedEvent
    | OutputItemDoneEvent
    | ContentPartAddedEvent
    | ContentPartDoneEvent
    | OutputTextDeltaEvent
    | OutputTextAnnotationAddedEvent
    | OutputTextDoneEvent
    | RefusalDeltaEvent
    | RefusalDoneEvent
    # ast-grep-ignore: no-long-names
    | FunctionCallArgumentsDeltaEvent
    | FunctionCallArgumentsDoneEvent
    | FileSearchCallInProgressEvent
    | FileSearchCallSearchingEvent
    | FileSearchCallCompletedEvent
    | WebSearchCallInProgressEvent
    | WebSearchCallSearchingEvent
    | WebSearchCallCompletedEvent
    | MCPListToolsInProgressEvent
    | MCPListToolsCompletedEvent
    | MCPListToolsFailedEvent
    | MCPCallInProgressEvent
    | MCPCallArgumentsDeltaEvent
    | MCPCallArgumentsDoneEvent
    | MCPCallCompletedEvent
    | MCPCallFailedEvent
    # ast-grep-ignore: no-long-names
    | ImageGenerationPartialImageEvent
    | ErrorEvent
    | GenericEvent
    | BaseLiteLLMOpenAIResponseObject,  # ast-grep-ignore: no-long-names
    Discriminator("type"),
]
