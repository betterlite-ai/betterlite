import importlib
import json
from types import UnionType
from typing import Any, Literal, Union, get_args, get_origin

import pytest
from pydantic import JsonValue, TypeAdapter
from typing_extensions import get_type_hints, is_typeddict

from betterlite.types.llms.bedrock.converse.citations import CitationsContentBlock
from betterlite.types.llms.bedrock.converse.messages import ContentBlock, ConverseResponseBlock
from betterlite.types.llms.bedrock.converse.server_sent_event import (
    ServerSentEvent,
    ServerSentEventFields,
)
from betterlite.types.llms.bedrock.converse.tools import BedrockToolParameters, BedrockToolSpec

PACKAGE = "betterlite.types.llms.bedrock.converse"
theirs = importlib.import_module("litellm.types.llms.bedrock")
openai = importlib.import_module("litellm.types.llms.openai")
content = importlib.import_module(f"{PACKAGE}.content")
citations = importlib.import_module(f"{PACKAGE}.citations")
messages = importlib.import_module(f"{PACKAGE}.messages")
tools = importlib.import_module(f"{PACKAGE}.tools")
request = importlib.import_module(f"{PACKAGE}.request")
stream_events = importlib.import_module(f"{PACKAGE}.stream_events")
tool_call_chunk = importlib.import_module(f"{PACKAGE}.tool_call_chunk")
sent_event = importlib.import_module(f"{PACKAGE}.server_sent_event")

TYPED_DICTS = {
    "CachePointBlock": (content.CachePointBlock, theirs.CachePointBlock),
    "SystemContentBlock": (content.SystemContentBlock, theirs.SystemContentBlock),
    "SourceBlock": (content.SourceBlock, theirs.SourceBlock),
    "ImageBlock": (content.ImageBlock, theirs.ImageBlock),
    "VideoBlock": (content.VideoBlock, theirs.VideoBlock),
    "DocumentBlock": (content.DocumentBlock, theirs.DocumentBlock),
    "SearchResultBlock": (content.SearchResultBlock, theirs.SearchResultBlock),
    "ToolResultContentBlock": (content.ToolResultContentBlock, theirs.ToolResultContentBlock),
    "ToolResultBlock": (content.ToolResultBlock, theirs.ToolResultBlock),
    "ToolUseBlock": (content.ToolUseBlock, theirs.ToolUseBlock),
    "BedrockConverseReasoningTextBlock": (
        content.BedrockConverseReasoningTextBlock,
        theirs.BedrockConverseReasoningTextBlock,
    ),
    "BedrockConverseReasoningContentBlock": (
        content.BedrockConverseReasoningContentBlock,
        theirs.BedrockConverseReasoningContentBlock,
    ),
    "BedrockConverseReasoningContentBlockDelta": (
        content.BedrockConverseReasoningContentBlockDelta,
        theirs.BedrockConverseReasoningContentBlockDelta,
    ),
    "BedrockConverseGptReasoningEffortBlock": (
        content.BedrockConverseGptReasoningEffortBlock,
        theirs.BedrockConverseGptReasoningEffortBlock,
    ),
    "GuardrailConverseTextBlock": (
        content.GuardrailConverseTextBlock,
        theirs.GuardrailConverseTextBlock,
    ),
    "GuardrailConverseContentBlock": (
        content.GuardrailConverseContentBlock,
        theirs.GuardrailConverseContentBlock,
    ),
    "CitationWebLocationBlock": (
        citations.CitationWebLocationBlock,
        theirs.CitationWebLocationBlock,
    ),
    "CitationSearchResultLocationBlock": (
        citations.CitationSearchResultLocationBlock,
        theirs.CitationSearchResultLocationBlock,
    ),
    "CitationLocationBlock": (citations.CitationLocationBlock, theirs.CitationLocationBlock),
    "CitationReferenceBlock": (citations.CitationReferenceBlock, theirs.CitationReferenceBlock),
    "CitationGeneratedContentBlock": (
        citations.CitationGeneratedContentBlock,
        theirs.CitationGeneratedContentBlock,
    ),
    "CitationsContentBlock": (citations.CitationsContentBlock, theirs.CitationsContentBlock),
    "ContentBlock": (messages.ContentBlock, theirs.ContentBlock),
    "MessageBlock": (messages.MessageBlock, theirs.MessageBlock),
    "ConverseMetricsBlock": (messages.ConverseMetricsBlock, theirs.ConverseMetricsBlock),
    "ConverseResponseOutputBlock": (
        messages.ConverseResponseOutputBlock,
        theirs.ConverseResponseOutputBlock,
    ),
    "CacheDetailBlock": (messages.CacheDetailBlock, theirs.CacheDetailBlock),
    "ConverseTokenUsageBlock": (messages.ConverseTokenUsageBlock, theirs.ConverseTokenUsageBlock),
    "ServiceTierBlock": (messages.ServiceTierBlock, theirs.ServiceTierBlock),
    "ConverseResponseBlock": (messages.ConverseResponseBlock, theirs.ConverseResponseBlock),
    "ToolJsonSchemaBlock": (tools.ToolJsonSchemaBlock, theirs.ToolJsonSchemaBlock),
    "ToolInputSchemaBlock": (tools.ToolInputSchemaBlock, theirs.ToolInputSchemaBlock),
    "ToolSpecBlock": (tools.ToolSpecBlock, theirs.ToolSpecBlock),
    "SystemToolBlock": (tools.SystemToolBlock, theirs.SystemToolBlock),
    "ToolBlock": (tools.ToolBlock, theirs.ToolBlock),
    "SpecificToolChoiceBlock": (tools.SpecificToolChoiceBlock, theirs.SpecificToolChoiceBlock),
    "ToolChoiceValuesBlock": (tools.ToolChoiceValuesBlock, theirs.ToolChoiceValuesBlock),
    "ToolConfigBlock": (tools.ToolConfigBlock, theirs.ToolConfigBlock),
    "GuardrailConfigBlock": (request.GuardrailConfigBlock, theirs.GuardrailConfigBlock),
    "InferenceConfig": (request.InferenceConfig, theirs.InferenceConfig),
    "PerformanceConfigBlock": (request.PerformanceConfigBlock, theirs.PerformanceConfigBlock),
    "JsonSchemaDefinition": (request.JsonSchemaDefinition, theirs.JsonSchemaDefinition),
    "OutputFormatStructure": (request.OutputFormatStructure, theirs.OutputFormatStructure),
    "OutputFormat": (request.OutputFormat, theirs.OutputFormat),
    "OutputConfigBlock": (request.OutputConfigBlock, theirs.OutputConfigBlock),
    "CommonRequestObject": (request.CommonRequestObject, theirs.CommonRequestObject),
    "RequestObject": (request.RequestObject, theirs.RequestObject),
    "BedrockInvokeNovaRequest": (request.BedrockInvokeNovaRequest, theirs.BedrockInvokeNovaRequest),
    "ToolBlockDeltaEvent": (stream_events.ToolBlockDeltaEvent, theirs.ToolBlockDeltaEvent),
    "ToolUseBlockStartEvent": (
        stream_events.ToolUseBlockStartEvent,
        theirs.ToolUseBlockStartEvent,
    ),
    "ContentBlockStartEvent": (stream_events.ContentBlockStartEvent, theirs.ContentBlockStartEvent),
    "ContentBlockDeltaEvent": (stream_events.ContentBlockDeltaEvent, theirs.ContentBlockDeltaEvent),
    "GenericStreamingChunk": (stream_events.GenericStreamingChunk, theirs.GenericStreamingChunk),
    "Document": (stream_events.Document, theirs.Document),
    "ChatCompletionToolCallFunctionChunk": (
        tool_call_chunk.ChatCompletionToolCallFunctionChunk,
        openai.ChatCompletionToolCallFunctionChunk,
    ),
    "ChatCompletionToolCallChunk": (
        tool_call_chunk.ChatCompletionToolCallChunk,
        openai.ChatCompletionToolCallChunk,
    ),
}

LITERAL_ALIASES = {
    "BedrockImageTypes": (content.BedrockImageTypes, theirs.BedrockImageTypes),
    "BedrockVideoTypes": (content.BedrockVideoTypes, theirs.BedrockVideoTypes),
    "BedrockDocumentTypes": (content.BedrockDocumentTypes, theirs.BedrockDocumentTypes),
}

REQUIRED_KEYS = {
    "ContentBlock": frozenset[str](),
    "GenericStreamingChunk": frozenset(
        {"text", "tool_use", "is_finished", "finish_reason", "usage", "index"},
    ),
    "Document": frozenset({"title", "snippet"}),
    "ConverseResponseBlock": frozenset({"output", "stopReason", "usage"}),
    "ConverseTokenUsageBlock": frozenset({"inputTokens", "outputTokens"}),
    "RequestObject": frozenset({"messages"}),
    "ToolConfigBlock": frozenset({"tools"}),
}

READ_ONLY_KEYS = {
    "BedrockConverseGptReasoningEffortBlock": frozenset({"effort"}),
    "CacheDetailBlock": frozenset({"inputTokens", "ttl"}),
    "ConverseTokenUsageBlock": frozenset(
        {
            "inputTokens",
            "outputTokens",
            "totalTokens",
            "cacheReadInputTokenCount",
            "cacheReadInputTokens",
            "cacheWriteInputTokenCount",
            "cacheWriteInputTokens",
            "cacheDetails",
        },
    ),
    "ToolSpecBlock": frozenset[str](),
}

CITATIONS: CitationsContentBlock = {
    "content": [{"text": "The grounded answer text"}],
    "citations": [
        {
            "location": {"web": {"url": "https://example.com/a", "domain": "example.com"}},
            "source": "https://example.com/a",
            "title": "A web page",
        },
        {
            "location": {"searchResultLocation": {"start": 0, "end": 42, "searchResultIndex": 1}},
            "source": "https://example.com/b",
            "title": "A search result",
        },
    ],
}

CONTENT: ContentBlock = {
    "text": "hello",
    "image": {"format": "png", "source": {"bytes": None}},
    "video": {"format": "mp4", "source": {"bytes": "AAAA"}},
    "document": {"format": "pdf", "source": {"bytes": "BBBB"}, "name": "doc"},
    "toolResult": {
        "toolUseId": "t1",
        "status": "success",
        "content": [
            {"text": "ok", "json": {"a": 1}},
            {
                "searchResult": {
                    "source": "s",
                    "title": "t",
                    "content": [{"type": "text"}],
                    "citations": {"enabled": True},
                },
            },
        ],
    },
    "toolUse": {"input": {"city": "Paris"}, "name": "weather", "toolUseId": "t1"},
    "cachePoint": {"type": "default", "ttl": "1h"},
    "reasoningContent": {
        "reasoningText": {"text": "thinking", "signature": "sig"},
        "redactedContent": "xxx",
    },
    "guardContent": {"text": {"text": "guarded"}},
    "citationsContent": CITATIONS,
}

RESPONSE: ConverseResponseBlock = {
    "additionalModelResponseFields": {"extra": [1, 2]},
    "metrics": {"latencyMs": 12.5},
    "output": {"message": {"role": "assistant", "content": [CONTENT]}},
    "stopReason": "end_turn",
    "usage": {
        "inputTokens": 1,
        "outputTokens": 2,
        "totalTokens": 3,
        "cacheReadInputTokens": 4,
        "cacheWriteInputTokens": 5,
        "cacheDetails": [{"inputTokens": 5, "ttl": "5m"}],
    },
    "serviceTier": {"type": "flex"},
}

TOOL_SPEC_CASES: list[tuple[BedrockToolParameters, bool | None, bool]] = [
    ({"type": "object"}, None, False),
    ({"type": "object"}, None, True),
    (
        {
            "type": "object",
            "properties": {"city": {"type": "string"}},
            "required": ["city"],
            "additionalProperties": False,
        },
        True,
        True,
    ),
    (
        {
            "type": "object",
            "properties": {"city": {"type": "string"}},
            "required": ["city"],
            "additionalProperties": False,
        },
        True,
        False,
    ),
    ({"type": "object", "additionalProperties": None}, False, True),
    ({"type": "object", "additionalProperties": True}, False, True),
    ({"type": "object", "properties": {}, "required": []}, None, True),
]

SSE_FIELDS: list[ServerSentEventFields] = [
    {},
    {"event": "message", "data": '{"a": 1}', "id": "7", "retry": 3000},
    {"event": "", "data": "x"},
    {"event": None, "data": None, "id": None, "retry": None},
    {"data": "plain text", "id": "", "retry": 0},
]


def shape(annotation: type) -> str:
    origin = get_origin(annotation)
    arguments = get_args(annotation)
    if origin is Literal:
        return f"Literal{list(arguments)}"
    if origin in (Union, UnionType):
        return "|".join(sorted(shape(argument) for argument in arguments))
    if is_typeddict(annotation):
        return f"TypedDict:{annotation.__name__}"
    if origin is dict and arguments[1] in (JsonValue, Any):
        return "dict"
    if origin is None:
        return annotation.__name__
    return origin.__name__ + "[" + ",".join(shape(argument) for argument in arguments) + "]"


@pytest.mark.parametrize("name", TYPED_DICTS)
def test_names_typed_dict_matches(name: str) -> None:
    ours, reference = TYPED_DICTS[name]

    assert ours.__required_keys__ == reference.__required_keys__
    assert ours.__optional_keys__ == reference.__optional_keys__
    assert ours.__readonly_keys__ == reference.__readonly_keys__
    assert ours.__total__ == reference.__total__
    our_hints = get_type_hints(ours)
    reference_hints = get_type_hints(reference)
    assert list(our_hints) == list(reference_hints)
    for key, hint in reference_hints.items():
        assert shape(our_hints[key]) == shape(hint), key


@pytest.mark.parametrize("name", REQUIRED_KEYS)
def test_names_required_keys(name: str) -> None:
    ours, reference = TYPED_DICTS[name]

    assert ours.__required_keys__ == REQUIRED_KEYS[name]
    assert reference.__required_keys__ == REQUIRED_KEYS[name]


@pytest.mark.parametrize("name", READ_ONLY_KEYS)
def test_names_read_only_keys(name: str) -> None:
    ours, reference = TYPED_DICTS[name]

    assert ours.__readonly_keys__ == READ_ONLY_KEYS[name]
    assert reference.__readonly_keys__ == READ_ONLY_KEYS[name]


@pytest.mark.parametrize("name", LITERAL_ALIASES)
def test_names_literal_aliases(name: str) -> None:
    ours, reference = LITERAL_ALIASES[name]

    assert get_args(ours) == get_args(reference)
    assert get_origin(ours) is Literal


def test_names_content_optional() -> None:
    assert ContentBlock.__required_keys__ == frozenset()
    assert theirs.ContentBlock.__required_keys__ == frozenset()
    assert ContentBlock.__optional_keys__ == frozenset(
        {
            "text",
            "image",
            "video",
            "document",
            "toolResult",
            "toolUse",
            "cachePoint",
            "reasoningContent",
            "guardContent",
            "citationsContent",
        },
    )


def test_names_document_vs_block() -> None:
    assert stream_events.Document.__required_keys__ == frozenset({"title", "snippet"})
    assert content.DocumentBlock.__required_keys__ == frozenset({"format", "source", "name"})
    assert stream_events.Document is not content.DocumentBlock


def test_names_citations_validate() -> None:
    for module in (citations, theirs):
        adapter = TypeAdapter(module.CitationsContentBlock)

        assert adapter.validate_python(CITATIONS) == CITATIONS


def test_names_blocks_validate() -> None:
    for module in (messages, theirs):
        content_adapter = TypeAdapter(module.ContentBlock)
        response_adapter = TypeAdapter(module.ConverseResponseBlock)

        assert content_adapter.validate_python(CONTENT) == CONTENT
        assert response_adapter.validate_python(RESPONSE) == RESPONSE


@pytest.mark.parametrize("case", TOOL_SPEC_CASES)
def test_names_tool_spec_matches(
    case: tuple[BedrockToolParameters, bool | None, bool],
) -> None:
    parameters, strict, supports_strict_tools = case

    ours = BedrockToolSpec(
        name="weather",
        description="Get the weather",
        parameters=parameters,
        strict=strict,
        supports_strict_tools=supports_strict_tools,
    )
    reference = theirs.BedrockToolSpec(
        name="weather",
        description="Get the weather",
        parameters=parameters,
        strict=strict,
        supports_strict_tools=supports_strict_tools,
    )

    assert json.dumps(ours) == json.dumps(reference)
    assert list(ours) == ["toolSpec"]
    assert dict in BedrockToolSpec.__mro__


def test_names_tool_spec_type() -> None:
    for build in (theirs.BedrockToolSpec, tools.BedrockToolSpec):
        with pytest.raises(KeyError):
            build(
                name="n",
                description="d",
                parameters={},
                strict=None,
                supports_strict_tools=False,
            )


@pytest.mark.parametrize("fields", SSE_FIELDS)
def test_sse_members_match_litellm(fields: ServerSentEventFields) -> None:
    ours = ServerSentEvent(**fields)
    reference = theirs.ServerSentEvent(**fields)

    assert ours.event == reference.event
    assert ours.id == reference.id
    assert ours.retry == reference.retry
    assert ours.data == reference.data
    assert repr(ours) == repr(reference)


def test_sse_defaults() -> None:
    assert ServerSentEvent().event is None
    assert ServerSentEvent().data == ""
    assert ServerSentEvent(data=None).data == ""
    assert ServerSentEvent(event="").event is None
    assert ServerSentEvent(event="e").event == "e"
    assert ServerSentEvent(id="").id == ""
    assert ServerSentEvent(retry=0).retry == 0


def test_sse_repr_text() -> None:
    event = ServerSentEvent(event="message", data="hi", id="7", retry=3)

    assert repr(event) == "ServerSentEvent(event=message, data=hi, id=7, retry=3)"
    assert repr(ServerSentEvent()) == "ServerSentEvent(event=None, data=, id=None, retry=None)"


@pytest.mark.parametrize("data", ['{"a": [1, 2.5, null]}', "[]", '"text"', "7", "null", "true"])
def test_sse_json_parses(data: str) -> None:
    ours = ServerSentEvent(data=data).json()
    reference = theirs.ServerSentEvent(data=data).json()

    assert ours == reference


@pytest.mark.parametrize("data", [None, "", "not json", "{"])
def test_sse_json_rejects(data: str | None) -> None:
    with pytest.raises(json.JSONDecodeError):
        theirs.ServerSentEvent(data=data).json()
    with pytest.raises(json.JSONDecodeError):
        ServerSentEvent(data=data).json()


def test_sse_rejects_bad_arguments() -> None:
    for build in (theirs.ServerSentEvent, sent_event.ServerSentEvent):
        with pytest.raises(TypeError, match="bogus"):
            build(bogus=1)
        with pytest.raises(TypeError):
            build("event")


def test_sse_members_read_only() -> None:
    for build in (theirs.ServerSentEvent, sent_event.ServerSentEvent):
        event = build(event="e", data="d", id="i", retry=1)
        with pytest.raises(AttributeError):
            event.event = "changed"
        with pytest.raises(AttributeError):
            event.data = "changed"
        with pytest.raises(AttributeError):
            event.id = "changed"
        with pytest.raises(AttributeError):
            event.retry = 2
