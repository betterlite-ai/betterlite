from typing import TypeAlias, cast

from litellm.types.llms.openai import (
    ContentPartAddedEvent,
    ContentPartDoneEvent,
    ErrorEvent,
    FileSearchCallCompletedEvent,
    FileSearchCallInProgressEvent,
    FileSearchCallSearchingEvent,
    FunctionCallArgumentsDeltaEvent,
    FunctionCallArgumentsDoneEvent,
    GenericEvent,
    ImageGenerationPartialImageEvent,
    MCPCallArgumentsDeltaEvent,
    MCPCallArgumentsDoneEvent,
    MCPCallCompletedEvent,
    MCPCallFailedEvent,
    MCPCallInProgressEvent,
    MCPListToolsCompletedEvent,
    MCPListToolsFailedEvent,
    MCPListToolsInProgressEvent,
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
    ResponseCompletedEvent,
    ResponseCreatedEvent,
    ResponseFailedEvent,
    ResponseIncompleteEvent,
    ResponseInProgressEvent,
    ResponsePartAddedEvent,
    ResponsesAPIStreamEvents,
    WebSearchCallCompletedEvent,
    WebSearchCallInProgressEvent,
    WebSearchCallSearchingEvent,
)
from pydantic import BaseModel

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

EVENT_MODELS: dict[ResponsesAPIStreamEvents, type[BaseModel]] = {
    ResponsesAPIStreamEvents.RESPONSE_CREATED: ResponseCreatedEvent,
    ResponsesAPIStreamEvents.RESPONSE_IN_PROGRESS: ResponseInProgressEvent,
    ResponsesAPIStreamEvents.RESPONSE_COMPLETED: ResponseCompletedEvent,
    ResponsesAPIStreamEvents.RESPONSE_FAILED: ResponseFailedEvent,
    ResponsesAPIStreamEvents.RESPONSE_INCOMPLETE: ResponseIncompleteEvent,
    ResponsesAPIStreamEvents.RESPONSE_PART_ADDED: ResponsePartAddedEvent,
    ResponsesAPIStreamEvents.REASONING_SUMMARY_TEXT_DELTA: ReasoningSummaryTextDeltaEvent,
    ResponsesAPIStreamEvents.REASONING_SUMMARY_TEXT_DONE: ReasoningSummaryTextDoneEvent,
    ResponsesAPIStreamEvents.REASONING_SUMMARY_PART_DONE: ReasoningSummaryPartDoneEvent,
    ResponsesAPIStreamEvents.OUTPUT_ITEM_ADDED: OutputItemAddedEvent,
    ResponsesAPIStreamEvents.OUTPUT_ITEM_DONE: OutputItemDoneEvent,
    ResponsesAPIStreamEvents.CONTENT_PART_ADDED: ContentPartAddedEvent,
    ResponsesAPIStreamEvents.CONTENT_PART_DONE: ContentPartDoneEvent,
    ResponsesAPIStreamEvents.OUTPUT_TEXT_DELTA: OutputTextDeltaEvent,
    ResponsesAPIStreamEvents.OUTPUT_TEXT_ANNOTATION_ADDED: OutputTextAnnotationAddedEvent,
    ResponsesAPIStreamEvents.OUTPUT_TEXT_DONE: OutputTextDoneEvent,
    ResponsesAPIStreamEvents.REFUSAL_DELTA: RefusalDeltaEvent,
    ResponsesAPIStreamEvents.REFUSAL_DONE: RefusalDoneEvent,
    ResponsesAPIStreamEvents.FUNCTION_CALL_ARGUMENTS_DELTA: FunctionCallArgumentsDeltaEvent,
    ResponsesAPIStreamEvents.FUNCTION_CALL_ARGUMENTS_DONE: FunctionCallArgumentsDoneEvent,
    ResponsesAPIStreamEvents.FILE_SEARCH_CALL_IN_PROGRESS: FileSearchCallInProgressEvent,
    ResponsesAPIStreamEvents.FILE_SEARCH_CALL_SEARCHING: FileSearchCallSearchingEvent,
    ResponsesAPIStreamEvents.FILE_SEARCH_CALL_COMPLETED: FileSearchCallCompletedEvent,
    ResponsesAPIStreamEvents.WEB_SEARCH_CALL_IN_PROGRESS: WebSearchCallInProgressEvent,
    ResponsesAPIStreamEvents.WEB_SEARCH_CALL_SEARCHING: WebSearchCallSearchingEvent,
    ResponsesAPIStreamEvents.WEB_SEARCH_CALL_COMPLETED: WebSearchCallCompletedEvent,
    ResponsesAPIStreamEvents.MCP_LIST_TOOLS_IN_PROGRESS: MCPListToolsInProgressEvent,
    ResponsesAPIStreamEvents.MCP_LIST_TOOLS_COMPLETED: MCPListToolsCompletedEvent,
    ResponsesAPIStreamEvents.MCP_LIST_TOOLS_FAILED: MCPListToolsFailedEvent,
    ResponsesAPIStreamEvents.MCP_CALL_IN_PROGRESS: MCPCallInProgressEvent,
    ResponsesAPIStreamEvents.MCP_CALL_ARGUMENTS_DELTA: MCPCallArgumentsDeltaEvent,
    ResponsesAPIStreamEvents.MCP_CALL_ARGUMENTS_DONE: MCPCallArgumentsDoneEvent,
    ResponsesAPIStreamEvents.MCP_CALL_COMPLETED: MCPCallCompletedEvent,
    ResponsesAPIStreamEvents.MCP_CALL_FAILED: MCPCallFailedEvent,
    ResponsesAPIStreamEvents.IMAGE_GENERATION_PARTIAL_IMAGE: ImageGenerationPartialImageEvent,
    ResponsesAPIStreamEvents.ERROR: ErrorEvent,
}


def run(value: dict[str, Json]) -> Json:
    try:
        event_type = ResponsesAPIStreamEvents(cast(str, value["type"]))
    except ValueError:
        model = GenericEvent
    else:
        model = EVENT_MODELS.get(event_type, GenericEvent)
    event = model.model_validate(value)
    return cast(Json, event.model_dump(exclude_none=False))
