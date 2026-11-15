from typing import Literal, TypeAlias

from pydantic import JsonValue

from betterlite.types.llms.base import BaseLiteLLMOpenAIResponseObject
from betterlite.types.llms.openai.chat.event_types import ResponsesAPIStreamEvents
from betterlite.types.llms.openai.chat.logprobs import (
    OpenAIChatCompletionLogprobsContent,
)


# ast-grep-ignore: no-long-names
class ResponsePartAddedEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.RESPONSE_PART_ADDED]
    item_id: str
    output_index: int
    part: dict[str, JsonValue]


# ast-grep-ignore: no-long-names
class ReasoningSummaryTextDeltaEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.REASONING_SUMMARY_TEXT_DELTA]
    item_id: str
    output_index: int
    summary_index: int = 0
    delta: str


# ast-grep-ignore: no-long-names
class ReasoningSummaryTextDoneEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.REASONING_SUMMARY_TEXT_DONE]
    item_id: str
    output_index: int
    sequence_number: int
    summary_index: int = 0
    text: str


# ast-grep-ignore: no-long-names
class ReasoningSummaryPartDoneEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.REASONING_SUMMARY_PART_DONE]
    item_id: str
    output_index: int
    sequence_number: int
    summary_index: int = 0
    part: BaseLiteLLMOpenAIResponseObject  # ast-grep-ignore: no-long-names


# ast-grep-ignore: no-long-names
class OutputItemAddedEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.OUTPUT_ITEM_ADDED]
    output_index: int
    item: BaseLiteLLMOpenAIResponseObject | None  # ast-grep-ignore: no-long-names


# ast-grep-ignore: no-long-names
class OutputItemDoneEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.OUTPUT_ITEM_DONE]
    output_index: int
    sequence_number: int = 1
    item: BaseLiteLLMOpenAIResponseObject  # ast-grep-ignore: no-long-names


# ast-grep-ignore: no-long-names
class ContentPartAddedEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.CONTENT_PART_ADDED]
    item_id: str
    output_index: int
    content_index: int
    part: BaseLiteLLMOpenAIResponseObject  # ast-grep-ignore: no-long-names


# ast-grep-ignore: no-long-names
class ContentPartDonePartOutputText(BaseLiteLLMOpenAIResponseObject):
    type: Literal["output_text"]
    text: str
    annotations: list[BaseLiteLLMOpenAIResponseObject]  # ast-grep-ignore: no-long-names
    logprobs: (
        list[OpenAIChatCompletionLogprobsContent] | None  # ast-grep-ignore: no-long-names
    )


# ast-grep-ignore: no-long-names
class ContentPartDonePartRefusal(BaseLiteLLMOpenAIResponseObject):
    type: Literal["refusal"]
    refusal: str


# ast-grep-ignore: no-long-names
class ContentPartDonePartReasoningText(BaseLiteLLMOpenAIResponseObject):
    type: Literal["reasoning_text"]
    reasoning: str


PART_UNION_TYPES: TypeAlias = (
    ContentPartDonePartOutputText
    | ContentPartDonePartRefusal
    # ast-grep-ignore: no-long-names
    | ContentPartDonePartReasoningText
)


# ast-grep-ignore: no-long-names
class ContentPartDoneEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.CONTENT_PART_DONE]
    item_id: str
    output_index: int
    content_index: int
    part: PART_UNION_TYPES


# ast-grep-ignore: no-long-names
class OutputTextDeltaEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.OUTPUT_TEXT_DELTA]
    item_id: str
    output_index: int
    content_index: int
    delta: str


# ast-grep-ignore: no-long-names
class OutputTextAnnotationAddedEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.OUTPUT_TEXT_ANNOTATION_ADDED]
    item_id: str
    output_index: int
    content_index: int
    annotation_index: int
    annotation: dict[str, JsonValue]


# ast-grep-ignore: no-long-names
class OutputTextDoneEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.OUTPUT_TEXT_DONE]
    item_id: str
    output_index: int
    content_index: int
    text: str


# ast-grep-ignore: no-long-names
class RefusalDeltaEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.REFUSAL_DELTA]
    item_id: str
    output_index: int
    content_index: int
    delta: str


# ast-grep-ignore: no-long-names
class RefusalDoneEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.REFUSAL_DONE]
    item_id: str
    output_index: int
    content_index: int
    refusal: str


# ast-grep-ignore: no-long-names
class FunctionCallArgumentsDeltaEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.FUNCTION_CALL_ARGUMENTS_DELTA]
    item_id: str
    output_index: int
    delta: str


# ast-grep-ignore: no-long-names
class FunctionCallArgumentsDoneEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.FUNCTION_CALL_ARGUMENTS_DONE]
    item_id: str
    output_index: int
    arguments: str


# ast-grep-ignore: no-long-names
class ImageGenerationPartialImageEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.IMAGE_GENERATION_PARTIAL_IMAGE]
    partial_image_index: int
    b64_json: str


# ast-grep-ignore: no-long-names
class ErrorEventError(BaseLiteLLMOpenAIResponseObject):
    type: str
    code: str
    message: str
    param: str | dict[str, JsonValue] | None = None


# ast-grep-ignore: no-long-names
class ErrorEvent(BaseLiteLLMOpenAIResponseObject):
    type: Literal[ResponsesAPIStreamEvents.ERROR]
    sequence_number: int
    error: ErrorEventError
