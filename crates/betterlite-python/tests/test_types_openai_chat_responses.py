import subprocess
import sys
from typing import Final, TypedDict, get_args, get_origin

import pytest
from pydantic import JsonValue, TypeAdapter

from betterlite.types.llms.openai.chat import ResponsesAPIStreamEvents
from betterlite.types.llms.openai.chat.responses_request import ALL_RESPONSES_API_TOOL_PARAMS

NAMES: Final = frozenset({
    "ALL_RESPONSES_API_TOOL_PARAMS", "AllEmbeddingInputValues",
    "ChatCompletionAssistantContentValue", "ChatCompletionDeltaChunk",
    "ChatCompletionNamedToolChoiceParam", "ChatCompletionRequest", "ChatCompletionResponseMessage",
    "ChatCompletionToolChoiceFunctionParam", "ChatCompletionToolChoiceObjectParam",
    "ChatCompletionToolChoiceStringValues", "ChatCompletionToolChoiceValues",
    "ChatCompletionToolParam", "ChatCompletionToolParamFunctionChunk", "ChatCompletionUsageBlock",
    "ComputerToolParam", "ContentPartAddedEvent", "ContentPartDoneEvent",
    "ContentPartDonePartOutputText", "ContentPartDonePartReasoningText",
    "ContentPartDonePartRefusal", "ContextManagementEntry", "CustomToolCallOutputItem",
    "ErrorEvent", "ErrorEventError", "FileSearchCallCompletedEvent",
    "FileSearchCallInProgressEvent", "FileSearchCallSearchingEvent", "FineTuningJobCreate",
    "Function", "FunctionCallArgumentsDeltaEvent", "FunctionCallArgumentsDoneEvent", "GenericEvent",
    "GenericResponseOutputItem", "Hyperparameters", "ImageGenerationPartialImageEvent",
    "InputTokensDetails", "LiteLLMFineTuningJobCreate", "MCPCallArgumentsDeltaEvent",
    "MCPCallArgumentsDoneEvent", "MCPCallCompletedEvent", "MCPCallFailedEvent",
    "MCPCallInProgressEvent", "MCPListToolsCompletedEvent", "MCPListToolsFailedEvent",
    "MCPListToolsInProgressEvent", "OpenAIAudioTranscriptionOptionalParams",
    "OpenAIChatCompletionChunk", "OpenAIChatCompletionLogprobsContent",
    "OpenAIChatCompletionLogprobsContentTopLogprobs", "OpenAIChatCompletionToolParam",
    "OpenAIImageEditOptionalParams", "OpenAIImageGenerationOptionalParams",
    "OpenAIImageVariationOptionalParams", "OutputCodeInterpreterCall", "OutputFunctionToolCall",
    "OutputImageGenerationCall", "OutputItemAddedEvent", "OutputItemDoneEvent",
    "OutputTextAnnotationAddedEvent", "OutputTextDeltaEvent", "OutputTextDoneEvent",
    "OutputTokensDetails", "PART_UNION_TYPES", "PromptObject", "ReasoningSummaryPartDoneEvent",
    "ReasoningSummaryTextDeltaEvent", "ReasoningSummaryTextDoneEvent", "RefusalDeltaEvent",
    "RefusalDoneEvent", "ResponseCompletedEvent", "ResponseCreatedEvent", "ResponseFailedEvent",
    "ResponseInProgressEvent", "ResponseIncompleteEvent", "ResponsePartAddedEvent",
    "ResponsesAPIOptionalRequestParams", "ResponsesAPIRequestParams", "ResponsesAPIStreamEvents",
    "ResponsesAPIStreamOptions", "ResponsesAPIStreamingResponse", "ResponsesTerminalEvent",
    "ShellToolParam", "WebSearchCallCompletedEvent", "WebSearchCallInProgressEvent",
    "WebSearchCallSearchingEvent",
})
TAGGED: Final = (
    "ResponseCreatedEvent", "ResponseInProgressEvent", "ResponseCompletedEvent",
    "ResponseFailedEvent", "ResponseIncompleteEvent", "ResponsePartAddedEvent",
    "ReasoningSummaryTextDeltaEvent", "ReasoningSummaryTextDoneEvent",
    "ReasoningSummaryPartDoneEvent", "OutputItemAddedEvent", "OutputItemDoneEvent",
    "ContentPartAddedEvent", "ContentPartDoneEvent", "OutputTextDeltaEvent",
    "OutputTextAnnotationAddedEvent", "OutputTextDoneEvent", "RefusalDeltaEvent",
    "RefusalDoneEvent", "FunctionCallArgumentsDeltaEvent", "FunctionCallArgumentsDoneEvent",
    "FileSearchCallInProgressEvent", "FileSearchCallSearchingEvent", "FileSearchCallCompletedEvent",
    "WebSearchCallInProgressEvent", "WebSearchCallSearchingEvent", "WebSearchCallCompletedEvent",
    "MCPListToolsInProgressEvent", "MCPListToolsCompletedEvent", "MCPListToolsFailedEvent",
    "MCPCallInProgressEvent", "MCPCallArgumentsDeltaEvent", "MCPCallArgumentsDoneEvent",
    "MCPCallCompletedEvent", "MCPCallFailedEvent", "ImageGenerationPartialImageEvent", "ErrorEvent",
)
CHOSEN: Final = (*TAGGED, "ContentPartDoneEvent", "ContentPartDoneEvent")
UNION_MEMBERS: Final = (*TAGGED, "GenericEvent", "BaseLiteLLMOpenAIResponseObject")
EVENT_MEMBERS: Final = {
    "RESPONSE_CREATED": "response.created",
    "RESPONSE_IN_PROGRESS": "response.in_progress",
    "RESPONSE_COMPLETED": "response.completed",
    "RESPONSE_FAILED": "response.failed",
    "RESPONSE_INCOMPLETE": "response.incomplete",
    "RESPONSE_PART_ADDED": "response.reasoning_summary_part.added",
    "REASONING_SUMMARY_TEXT_DELTA": "response.reasoning_summary_text.delta",
    "REASONING_SUMMARY_TEXT_DONE": "response.reasoning_summary_text.done",
    "REASONING_SUMMARY_PART_DONE": "response.reasoning_summary_part.done",
    "OUTPUT_ITEM_ADDED": "response.output_item.added",
    "OUTPUT_ITEM_DONE": "response.output_item.done",
    "CONTENT_PART_ADDED": "response.content_part.added",
    "CONTENT_PART_DONE": "response.content_part.done",
    "OUTPUT_TEXT_DELTA": "response.output_text.delta",
    "OUTPUT_TEXT_ANNOTATION_ADDED": "response.output_text.annotation.added",
    "OUTPUT_TEXT_DONE": "response.output_text.done",
    "REFUSAL_DELTA": "response.refusal.delta",
    "REFUSAL_DONE": "response.refusal.done",
    "FUNCTION_CALL_ARGUMENTS_DELTA": "response.function_call_arguments.delta",
    "FUNCTION_CALL_ARGUMENTS_DONE": "response.function_call_arguments.done",
    "CUSTOM_TOOL_CALL_INPUT_DELTA": "response.custom_tool_call_input.delta",
    "CUSTOM_TOOL_CALL_INPUT_DONE": "response.custom_tool_call_input.done",
    "FILE_SEARCH_CALL_IN_PROGRESS": "response.file_search_call.in_progress",
    "FILE_SEARCH_CALL_SEARCHING": "response.file_search_call.searching",
    "FILE_SEARCH_CALL_COMPLETED": "response.file_search_call.completed",
    "WEB_SEARCH_CALL_IN_PROGRESS": "response.web_search_call.in_progress",
    "WEB_SEARCH_CALL_SEARCHING": "response.web_search_call.searching",
    "WEB_SEARCH_CALL_COMPLETED": "response.web_search_call.completed",
    "MCP_LIST_TOOLS_IN_PROGRESS": "response.mcp_list_tools.in_progress",
    "MCP_LIST_TOOLS_COMPLETED": "response.mcp_list_tools.completed",
    "MCP_LIST_TOOLS_FAILED": "response.mcp_list_tools.failed",
    "MCP_CALL_IN_PROGRESS": "response.mcp_call.in_progress",
    "MCP_CALL_ARGUMENTS_DELTA": "response.mcp_call_arguments.delta",
    "MCP_CALL_ARGUMENTS_DONE": "response.mcp_call_arguments.done",
    "MCP_CALL_COMPLETED": "response.mcp_call.completed",
    "MCP_CALL_FAILED": "response.mcp_call.failed",
    "IMAGE_GENERATION_PARTIAL_IMAGE": "image_generation.partial_image",
    "SHELL_CALL_IN_PROGRESS": "response.shell_call.in_progress",
    "SHELL_CALL_COMPLETED": "response.shell_call.completed",
    "SHELL_CALL_OUTPUT": "response.shell_call_output.done",
    "ERROR": "error",
}
OPTION_LITERALS: Final = {
    "ChatCompletionToolChoiceStringValues": ("none", "auto", "required"),
    "OpenAIAudioTranscriptionOptionalParams": (
        "language",
        "prompt",
        "temperature",
        "response_format",
        "timestamp_granularities",
        "include",
    ),
    "OpenAIImageVariationOptionalParams": ("n", "size", "response_format", "user"),
    "OpenAIImageGenerationOptionalParams": (
        "background",
        "moderation",
        "n",
        "output_compression",
        "output_format",
        "quality",
        "response_format",
        "size",
        "style",
        "user",
        "seed",
        "safety_tolerance",
        "prompt_upsampling",
        "raw",
        "num_images",
        "image_url",
        "image_prompt_strength",
        "aspect_ratio",
        "width",
        "height",
        "guidance",
        "steps",
        "imageConfig",
    ),
    "OpenAIImageEditOptionalParams": (
        "background",
        "n",
        "maskoutput_compression",
        "output_format",
        "quality",
        "partial_images",
        "response_format",
        "size",
        "style",
        "user",
    ),
}
DESCRIBE_SCRIPT = """
import json
import sys
from enum import Enum
from types import UnionType
from typing import Annotated, Literal, Union, get_args, get_origin, get_type_hints

from pydantic import BaseModel, Discriminator, TypeAdapter
from typing_extensions import is_typeddict

import betterlite.types.llms.openai.chat as ours
import litellm.types.llms.openai as theirs

MAPPED_NAMES = {"ToolParam", "ResponseInputParam", "ResponseIncludable"}
MAPPED_FIELDS = {
    "OpenAIChatCompletionChunk": ("service_tier",),
    "ComputerToolParam": ("environment", "type"),
    "ShellToolParam": ("type",),
    "OpenAIChatCompletionToolParam": ("type",),
    "ChatCompletionToolParam": ("type",),
}
BARE_ALIASES = ["AllEmbeddingInputValues"]

TAGGED = [
    "ResponseCreatedEvent",
    "ResponseInProgressEvent",
    "ResponseCompletedEvent",
    "ResponseFailedEvent",
    "ResponseIncompleteEvent",
    "ResponsePartAddedEvent",
    "ReasoningSummaryTextDeltaEvent",
    "ReasoningSummaryTextDoneEvent",
    "ReasoningSummaryPartDoneEvent",
    "OutputItemAddedEvent",
    "OutputItemDoneEvent",
    "ContentPartAddedEvent",
    "ContentPartDoneEvent",
    "OutputTextDeltaEvent",
    "OutputTextAnnotationAddedEvent",
    "OutputTextDoneEvent",
    "RefusalDeltaEvent",
    "RefusalDoneEvent",
    "FunctionCallArgumentsDeltaEvent",
    "FunctionCallArgumentsDoneEvent",
    "FileSearchCallInProgressEvent",
    "FileSearchCallSearchingEvent",
    "FileSearchCallCompletedEvent",
    "WebSearchCallInProgressEvent",
    "WebSearchCallSearchingEvent",
    "WebSearchCallCompletedEvent",
    "MCPListToolsInProgressEvent",
    "MCPListToolsCompletedEvent",
    "MCPListToolsFailedEvent",
    "MCPCallInProgressEvent",
    "MCPCallArgumentsDeltaEvent",
    "MCPCallArgumentsDoneEvent",
    "MCPCallCompletedEvent",
    "MCPCallFailedEvent",
    "ImageGenerationPartialImageEvent",
    "ErrorEvent",
]

BASE_RESPONSE = {
    "id": "resp_1",
    "created_at": 1741476542,
    "model": "gpt-4o",
    "object": "response",
    "status": "completed",
    "output": [],
}

SAMPLES_BY_CLASS = {
    "ResponseCreatedEvent": {"type": "response.created", "response": BASE_RESPONSE},
    "ResponseInProgressEvent": {"type": "response.in_progress", "response": BASE_RESPONSE},
    "ResponseCompletedEvent": {"type": "response.completed", "response": BASE_RESPONSE},
    "ResponseFailedEvent": {"type": "response.failed", "response": BASE_RESPONSE},
    "ResponseIncompleteEvent": {"type": "response.incomplete", "response": BASE_RESPONSE},
    "ResponsePartAddedEvent": {
        "type": "response.reasoning_summary_part.added",
        "item_id": "item_1",
        "output_index": 0,
        "part": {"type": "summary_text", "text": "thinking"},
    },
    "ReasoningSummaryTextDeltaEvent": {
        "type": "response.reasoning_summary_text.delta",
        "item_id": "item_1",
        "output_index": 0,
        "delta": "more",
    },
    "ReasoningSummaryTextDoneEvent": {
        "type": "response.reasoning_summary_text.done",
        "item_id": "item_1",
        "output_index": 0,
        "sequence_number": 4,
        "text": "thinking more",
    },
    "ReasoningSummaryPartDoneEvent": {
        "type": "response.reasoning_summary_part.done",
        "item_id": "item_1",
        "output_index": 0,
        "sequence_number": 5,
        "part": {"type": "summary_text", "text": "thinking more"},
    },
    "OutputItemAddedEvent": {
        "type": "response.output_item.added",
        "output_index": 0,
        "item": {"id": "item_1", "type": "message"},
    },
    "OutputItemDoneEvent": {
        "type": "response.output_item.done",
        "output_index": 0,
        "item": {"id": "item_1", "type": "message"},
    },
    "ContentPartAddedEvent": {
        "type": "response.content_part.added",
        "item_id": "item_1",
        "output_index": 0,
        "content_index": 0,
        "part": {"type": "output_text", "text": "hi", "annotations": []},
    },
    "ContentPartDoneEvent": {
        "type": "response.content_part.done",
        "item_id": "item_1",
        "output_index": 0,
        "content_index": 0,
        "part": {"type": "output_text", "text": "hi", "annotations": [], "logprobs": []},
    },
    "OutputTextDeltaEvent": {
        "type": "response.output_text.delta",
        "item_id": "item_1",
        "output_index": 0,
        "content_index": 0,
        "delta": "hello",
    },
    "OutputTextAnnotationAddedEvent": {
        "type": "response.output_text.annotation.added",
        "item_id": "item_1",
        "output_index": 0,
        "content_index": 0,
        "annotation_index": 0,
        "annotation": {"type": "url_citation", "url": "https://example.com"},
    },
    "OutputTextDoneEvent": {
        "type": "response.output_text.done",
        "item_id": "item_1",
        "output_index": 0,
        "content_index": 0,
        "text": "hello",
    },
    "RefusalDeltaEvent": {
        "type": "response.refusal.delta",
        "item_id": "item_1",
        "output_index": 0,
        "content_index": 0,
        "delta": "no",
    },
    "RefusalDoneEvent": {
        "type": "response.refusal.done",
        "item_id": "item_1",
        "output_index": 0,
        "content_index": 0,
        "refusal": "no",
    },
    "FunctionCallArgumentsDeltaEvent": {
        "type": "response.function_call_arguments.delta",
        "item_id": "item_1",
        "output_index": 0,
        "delta": '{"city":',
    },
    "FunctionCallArgumentsDoneEvent": {
        "type": "response.function_call_arguments.done",
        "item_id": "item_1",
        "output_index": 0,
        "arguments": '{"city": "Paris"}',
    },
    "FileSearchCallInProgressEvent": {
        "type": "response.file_search_call.in_progress",
        "output_index": 0,
        "item_id": "item_1",
    },
    "FileSearchCallSearchingEvent": {
        "type": "response.file_search_call.searching",
        "output_index": 0,
        "item_id": "item_1",
    },
    "FileSearchCallCompletedEvent": {
        "type": "response.file_search_call.completed",
        "output_index": 0,
        "item_id": "item_1",
    },
    "WebSearchCallInProgressEvent": {
        "type": "response.web_search_call.in_progress",
        "output_index": 0,
        "item_id": "item_1",
    },
    "WebSearchCallSearchingEvent": {
        "type": "response.web_search_call.searching",
        "output_index": 0,
        "item_id": "item_1",
    },
    "WebSearchCallCompletedEvent": {
        "type": "response.web_search_call.completed",
        "output_index": 0,
        "item_id": "item_1",
    },
    "MCPListToolsInProgressEvent": {
        "type": "response.mcp_list_tools.in_progress",
        "sequence_number": 1,
        "output_index": 0,
        "item_id": "item_1",
    },
    "MCPListToolsCompletedEvent": {
        "type": "response.mcp_list_tools.completed",
        "sequence_number": 2,
        "output_index": 0,
        "item_id": "item_1",
    },
    "MCPListToolsFailedEvent": {
        "type": "response.mcp_list_tools.failed",
        "sequence_number": 3,
        "output_index": 0,
        "item_id": "item_1",
    },
    "MCPCallInProgressEvent": {
        "type": "response.mcp_call.in_progress",
        "sequence_number": 4,
        "output_index": 0,
        "item_id": "item_1",
    },
    "MCPCallArgumentsDeltaEvent": {
        "type": "response.mcp_call_arguments.delta",
        "output_index": 0,
        "item_id": "item_1",
        "delta": '{"q":',
        "sequence_number": 5,
    },
    "MCPCallArgumentsDoneEvent": {
        "type": "response.mcp_call_arguments.done",
        "output_index": 0,
        "item_id": "item_1",
        "arguments": '{"q": "x"}',
        "sequence_number": 6,
    },
    "MCPCallCompletedEvent": {
        "type": "response.mcp_call.completed",
        "sequence_number": 7,
        "item_id": "item_1",
        "output_index": 0,
    },
    "MCPCallFailedEvent": {
        "type": "response.mcp_call.failed",
        "sequence_number": 8,
        "item_id": "item_1",
        "output_index": 0,
    },
    "ImageGenerationPartialImageEvent": {
        "type": "image_generation.partial_image",
        "partial_image_index": 0,
        "b64_json": "aGk=",
    },
    "ErrorEvent": {
        "type": "error",
        "sequence_number": 9,
        "error": {
            "type": "invalid_request_error",
            "code": "bad",
            "message": "bad input",
            "param": {"field": "input"},
        },
    },
}

SAMPLES = [SAMPLES_BY_CLASS[name] for name in TAGGED] + [
    {
        "type": "response.content_part.done",
        "item_id": "item_1",
        "output_index": 0,
        "content_index": 0,
        "part": {"type": "refusal", "refusal": "no"},
    },
    {
        "type": "response.content_part.done",
        "item_id": "item_1",
        "output_index": 0,
        "content_index": 0,
        "part": {"type": "reasoning_text", "reasoning": "why"},
    },
]


class Reasoning(BaseModel):
    effort: str
    summary: str | None = None


def literal_members(annotation):
    if get_origin(annotation) is Literal:
        return [
            json.dumps(member.value if isinstance(member, Enum) else member, sort_keys=True)
            for member in get_args(annotation)
        ]
    values = []
    if get_origin(annotation) is not None:
        for member in get_args(annotation):
            values.extend(literal_members(member))
    return values


def described_fields(annotations, names, required, defaults, mapped_fields):
    literals = {}
    for name in names:
        annotation = annotations.get(name)
        if annotation is None or name in mapped_fields:
            continue
        values = literal_members(annotation)
        if values:
            literals[name] = values
    return {
        "keys": sorted(names),
        "required": sorted(required),
        "defaults": defaults,
        "literals": literals,
    }


def typeddict_shape(value):
    described = described_fields(
        get_type_hints(value, include_extras=True),
        value.__required_keys__ | value.__optional_keys__,
        value.__required_keys__,
        {},
        MAPPED_FIELDS.get(value.__name__, ()),
    )
    described["kind"] = "typeddict"
    return described


def model_shape(value):
    fields = value.model_fields
    annotations = {
        name: field.annotation
        for name, field in fields.items()
        if field.annotation is not None
    }
    described = described_fields(
        annotations,
        fields.keys(),
        [name for name, field in fields.items() if field.is_required()],
        {
            name: json.dumps(field.get_default(call_default_factory=False), sort_keys=True)
            for name, field in fields.items()
            if not field.is_required()
        },
        MAPPED_FIELDS.get(value.__name__, ()),
    )
    described["kind"] = "model"
    return described


def member_label(member):
    if get_origin(member) is Literal:
        return "literal"
    name = getattr(member, "__name__", None)
    module = getattr(member, "__module__", "") or ""
    if name in MAPPED_NAMES or module.startswith("openai") or get_origin(member) is not None:
        return "json"
    if name is None:
        return "literal"
    return name


def alias_members(value):
    labels = []
    for member in get_args(value):
        label = member_label(member)
        if not labels or labels[-1] != label:
            labels.append(label)
    return labels


def alias_shape(value):
    origin = get_origin(value)
    if origin is Annotated:
        return {"kind": "annotated"}
    if origin is Union or origin is UnionType:
        return {"kind": "union", "members": alias_members(value)}
    if origin is list:
        return {"kind": "list"}
    return {"kind": "literal", "values": literal_members(value)}


def member_reprs(module):
    return {n: [repr(m) for m in get_args(getattr(module, n))] for n in BARE_ALIASES}


def shape_of(value):
    if is_typeddict(value):
        return typeddict_shape(value)
    if isinstance(value, type) and issubclass(value, Enum):
        return {
            "kind": "enum",
            "enum_members": [[name, member.value] for name, member in value.__members__.items()],
        }
    if isinstance(value, type) and issubclass(value, BaseModel):
        return model_shape(value)
    return alias_shape(value)


EMPTY_SHAPE = {
    "kind": "",
    "keys": [],
    "required": [],
    "defaults": {},
    "literals": {},
    "members": [],
    "enum_members": [],
    "values": [],
}


def filled(shape):
    return EMPTY_SHAPE | shape


def describe(module, names):
    return {
        name: None if not hasattr(module, name) else filled(shape_of(getattr(module, name)))
        for name in names
    }


def union_shape(alias):
    members = get_args(alias)
    return {
        "members": [member.__name__ for member in get_args(members[0])],
        "discriminators": [getattr(member, "discriminator", None) for member in members[1:]],
    }


def adapter_error(alias):
    try:
        TypeAdapter(alias)
    except Exception as error:
        return type(error).__name__
    return ""


def select(module):
    classes = [getattr(module, name) for name in TAGGED]
    adapter = TypeAdapter(Annotated[Union[tuple(classes)], Discriminator("type")])
    selected = [adapter.validate_python(sample) for sample in SAMPLES]
    return {
        "chosen": [type(value).__name__ for value in selected],
        "dumps": [value.model_dump(mode="json") for value in selected],
        "by_class": {
            name: getattr(module, name)
            .model_validate(SAMPLES_BY_CLASS[name])
            .model_dump(mode="json")
            for name in TAGGED
        },
    }


def unknown_type(module):
    generic = module.GenericEvent.model_validate({"type": "brand.new", "payload": [1]})
    return {"type_name": type(generic).__name__, "dump": generic.model_dump(mode="json")}


def models_dump(module):
    completed = module.ResponseCompletedEvent(
        type=module.ResponsesAPIStreamEvents.RESPONSE_COMPLETED,
        response={
            "id": "resp_1",
            "created_at": 1741476542,
            "model": "gpt-4o",
            "object": "response",
            "status": "completed",
            "output": [],
            "instructions": "be nice",
            "metadata": {"team": "core"},
            "provider_extra": {"region": "eu"},
            "reasoning": Reasoning(effort="high"),
            "usage": {
                "input_tokens": 11,
                "output_tokens": 7,
                "total_tokens": 18,
                "cost": {"total_cost": 0.25},
                "input_tokens_details": {"cached_tokens": 5, "text_tokens": 6},
                "output_tokens_details": {"reasoning_tokens": 2},
            },
        },
        custom_event="kept",
    )
    usage = completed.response.usage
    chunk = module.OpenAIChatCompletionChunk(
        id="chatcmpl-1",
        choices=[],
        created=1741476542,
        model="gpt-4o",
        object="stale",
        custom="kept",
    )
    fine_tuning = module.FineTuningJobCreate(
        model="gpt-3.5-turbo",
        training_file="file-train",
        validation_file="file-val",
        hyperparameters={"batch_size": "auto", "extra_hp": 1},
        suffix="mine",
        integrations=["slack"],
        seed=42,
        dropped="ignored",
    )
    lite_fine_tuning = module.LiteLLMFineTuningJobCreate(
        model="gpt-3.5-turbo",
        training_file="file-train",
        custom_llm_provider="azure",
        derived="kept",
    )
    hyperparameters = module.Hyperparameters(n_epochs=3, custom_hp=True)
    fresh = module.ResponseCompletedEvent(
        type=module.ResponsesAPIStreamEvents.RESPONSE_COMPLETED,
        response={"id": "resp_2", "created_at": 1, "output": []},
    )
    completed._hidden_params["tracked"] = 1
    return {
        "completed": completed.model_dump(mode="json"),
        "response": completed.response.model_dump(mode="json"),
        "usage": {} if usage is None else usage.model_dump(mode="json"),
        "usage_cost": None if usage is None else usage.cost,
        "usage_type": "" if usage is None else type(usage).__name__,
        "reasoning": completed.response.reasoning,
        "chunk": chunk.model_dump(mode="json"),
        "fine_tuning": fine_tuning.model_dump(mode="json"),
        "lite_fine_tuning": lite_fine_tuning.model_dump(mode="json"),
        "hyperparameters": hyperparameters.model_dump(mode="json"),
        "hidden_params": completed._hidden_params,
        "hidden_params_isolated": fresh._hidden_params,
        "hidden_params_dumped": "_hidden_params" in completed.model_dump(mode="json"),
    }


names = sys.argv[1:]
print(
    json.dumps(
        {
            "names": {
                "litellm": describe(theirs, names),
                "betterlite": describe(ours, names),
            },
            "aliases": {
                "litellm": member_reprs(theirs),
                "betterlite": member_reprs(ours),
            },
            "union": {
                "litellm": union_shape(theirs.ResponsesAPIStreamingResponse),
                "betterlite": union_shape(ours.ResponsesAPIStreamingResponse),
            },
            "adapter": {
                "litellm": adapter_error(theirs.ResponsesAPIStreamingResponse),
                "betterlite": adapter_error(ours.ResponsesAPIStreamingResponse),
            },
            "events": {"litellm": select(theirs), "betterlite": select(ours)},
            "unknown": {"litellm": unknown_type(theirs), "betterlite": unknown_type(ours)},
            "models": {"litellm": models_dump(theirs), "betterlite": models_dump(ours)},
        }
    )
)
"""


class Shape(TypedDict):
    kind: str
    keys: list[str]
    required: list[str]
    defaults: dict[str, str]
    literals: dict[str, list[str]]
    members: list[str]
    enum_members: list[tuple[str, str]]
    values: list[str]


class Names(TypedDict):
    litellm: dict[str, Shape | None]
    betterlite: dict[str, Shape | None]


class UnionShape(TypedDict):
    members: list[str]
    discriminators: list[str]


class Unions(TypedDict):
    litellm: UnionShape
    betterlite: UnionShape


class Adapters(TypedDict):
    litellm: str
    betterlite: str


class Selection(TypedDict):
    chosen: list[str]
    dumps: list[dict[str, JsonValue]]
    by_class: dict[str, dict[str, JsonValue]]


class Events(TypedDict):
    litellm: Selection
    betterlite: Selection


class Unknown(TypedDict):
    type_name: str
    dump: dict[str, JsonValue]


class Unknowns(TypedDict):
    litellm: Unknown
    betterlite: Unknown


class Models(TypedDict):
    completed: dict[str, JsonValue]
    response: dict[str, JsonValue]
    usage: dict[str, JsonValue]
    usage_cost: float | None
    usage_type: str
    reasoning: dict[str, JsonValue] | None
    chunk: dict[str, JsonValue]
    fine_tuning: dict[str, JsonValue]
    lite_fine_tuning: dict[str, JsonValue]
    hyperparameters: dict[str, JsonValue]
    hidden_params: dict[str, JsonValue]
    hidden_params_isolated: dict[str, JsonValue]
    hidden_params_dumped: bool


class ModelsBySide(TypedDict):
    litellm: Models
    betterlite: Models


class Report(TypedDict):
    names: Names
    aliases: dict[str, dict[str, list[str]]]
    union: Unions
    adapter: Adapters
    events: Events
    unknown: Unknowns
    models: ModelsBySide


@pytest.fixture(scope="module")
def report() -> Report:
    completed = subprocess.run(
        [sys.executable, "-c", DESCRIBE_SCRIPT, *sorted(NAMES)],
        check=True,
        capture_output=True,
        text=True,
    )
    parsed: Report = TypeAdapter(Report).validate_json(completed.stdout)
    return parsed


def test_openai_chat_names(report: Report) -> None:
    litellm_shapes = report["names"]["litellm"]
    betterlite_shapes = report["names"]["betterlite"]
    assert set(betterlite_shapes) == NAMES
    assert set(litellm_shapes) == NAMES
    assert None not in betterlite_shapes.values()
    assert None not in litellm_shapes.values()
    assert betterlite_shapes == litellm_shapes
    aliases = report["aliases"]
    assert aliases["betterlite"] == aliases["litellm"]
    ours = aliases["betterlite"]["AllEmbeddingInputValues"]
    assert ours == ["<class 'str'>", "list[str]", "list[int]", "list[list[int]]"]
    for name, expected in OPTION_LITERALS.items():
        shape = betterlite_shapes[name]
        assert shape is not None
        assert [value.strip('"') for value in shape["values"]] == list(expected)
    event_shape = betterlite_shapes["ResponsesAPIStreamEvents"]
    assert event_shape is not None
    assert event_shape["kind"] == "enum"
    assert dict(event_shape["enum_members"]) == EVENT_MEMBERS
    assert all(member == member.value for member in ResponsesAPIStreamEvents)
    tool_choice = betterlite_shapes["ChatCompletionToolChoiceValues"]
    assert tool_choice is not None
    assert tool_choice["members"] == ["literal", "ChatCompletionToolChoiceObjectParam"]
    tools = betterlite_shapes["ALL_RESPONSES_API_TOOL_PARAMS"]
    assert tools is not None
    assert tools["members"] == ["json", "ComputerToolParam", "ShellToolParam"]
    tool_members: tuple[JsonValue, ...] = get_args(ALL_RESPONSES_API_TOOL_PARAMS)
    tool_origin: type | None = get_origin(tool_members[0])
    assert tool_origin is dict
    terminal = betterlite_shapes["ResponsesTerminalEvent"]
    assert terminal is not None
    assert terminal["members"] == [
        "ResponseCompletedEvent",
        "ResponseIncompleteEvent",
        "ResponseFailedEvent",
    ]
    parts = betterlite_shapes["PART_UNION_TYPES"]
    assert parts is not None
    assert parts["members"] == [
        "ContentPartDonePartOutputText",
        "ContentPartDonePartRefusal",
        "ContentPartDonePartReasoningText",
    ]
    chunk = betterlite_shapes["OpenAIChatCompletionChunk"]
    assert chunk is not None
    assert chunk["required"] == ["choices", "created", "id", "model", "object"]


def test_openai_chat_models(report: Report) -> None:
    dumps = report["models"]
    assert dumps["betterlite"] == dumps["litellm"]
    ours = dumps["betterlite"]
    completed = ours["completed"]
    assert completed["type"] == "response.completed"
    assert completed["custom_event"] == "kept"
    assert "_hidden_params" not in completed
    assert ours["hidden_params"] == {"tracked": 1}
    assert ours["hidden_params_isolated"] == {}
    assert not ours["hidden_params_dumped"]
    assert ours["response"]["provider_extra"] == {"region": "eu"}
    assert ours["response"]["output"] == []
    assert ours["reasoning"] == {"effort": "high", "summary": None}
    assert ours["usage_type"] == "ResponseAPIUsage"
    assert ours["usage"] == {
        "cost": 0.25,
        "input_tokens": 11,
        "input_tokens_details": {
            "audio_tokens": None,
            "cached_tokens": 5,
            "cached_tokens_details": None,
            "image_tokens": None,
            "text_tokens": 6,
            "video_tokens": None,
        },
        "output_tokens": 7,
        "output_tokens_details": {
            "audio_tokens": None,
            "reasoning_tokens": 2,
            "text_tokens": None,
        },
        "total_tokens": 18,
    }
    assert ours["chunk"] == {
        "choices": [],
        "created": 1741476542,
        "custom": "kept",
        "id": "chatcmpl-1",
        "model": "gpt-4o",
        "moderation": None,
        "object": "chat.completion.chunk",
        "service_tier": None,
        "system_fingerprint": None,
        "usage": None,
    }
    assert ours["fine_tuning"] == {
        "hyperparameters": {
            "batch_size": "auto",
            "extra_hp": 1,
            "learning_rate_multiplier": None,
            "n_epochs": None,
        },
        "integrations": ["slack"],
        "model": "gpt-3.5-turbo",
        "seed": 42,
        "suffix": "mine",
        "training_file": "file-train",
        "validation_file": "file-val",
    }
    assert ours["lite_fine_tuning"] == {
        "custom_llm_provider": "azure",
        "derived": "kept",
        "hyperparameters": None,
        "integrations": None,
        "model": "gpt-3.5-turbo",
        "seed": None,
        "suffix": None,
        "training_file": "file-train",
        "validation_file": None,
    }
    assert ours["hyperparameters"] == {
        "batch_size": None,
        "custom_hp": True,
        "learning_rate_multiplier": None,
        "n_epochs": 3,
    }


def test_openai_chat_events(report: Report) -> None:
    union = report["union"]
    assert union["betterlite"] == union["litellm"]
    assert union["betterlite"]["members"] == list(UNION_MEMBERS)
    assert union["betterlite"]["discriminators"] == ["type"]
    assert report["adapter"] == {
        "litellm": "PydanticUserError",
        "betterlite": "PydanticUserError",
    }
    alias = report["names"]["betterlite"]["ResponsesAPIStreamingResponse"]
    assert alias is not None
    assert alias["kind"] == "annotated"
    events = report["events"]
    assert events["betterlite"] == events["litellm"]
    assert events["betterlite"]["chosen"] == list(CHOSEN)
    dumps = events["betterlite"]["dumps"]
    by_class = events["betterlite"]["by_class"]
    assert set(by_class) == set(TAGGED)
    assert by_class["OutputItemDoneEvent"] == {
        "type": "response.output_item.done",
        "output_index": 0,
        "sequence_number": 1,
        "item": {"id": "item_1", "type": "message"},
    }
    assert by_class["ReasoningSummaryTextDeltaEvent"] == {
        "type": "response.reasoning_summary_text.delta",
        "item_id": "item_1",
        "output_index": 0,
        "summary_index": 0,
        "delta": "more",
    }
    assert by_class["ErrorEvent"] == {
        "type": "error",
        "sequence_number": 9,
        "error": {
            "type": "invalid_request_error",
            "code": "bad",
            "message": "bad input",
            "param": {"field": "input"},
        },
    }
    content_parts = [
        dump for dump in dumps if dump["type"] == "response.content_part.done"
    ]
    assert [dump["part"] for dump in content_parts] == [
        {"type": "output_text", "text": "hi", "annotations": [], "logprobs": []},
        {"type": "refusal", "refusal": "no"},
        {"type": "reasoning_text", "reasoning": "why"},
    ]
    assert report["unknown"]["betterlite"] == report["unknown"]["litellm"]
    assert report["unknown"]["betterlite"]["type_name"] == "GenericEvent"
    assert report["unknown"]["betterlite"]["dump"] == {"type": "brand.new", "payload": [1]}
