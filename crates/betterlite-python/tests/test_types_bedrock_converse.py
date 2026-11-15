import importlib
import json
from typing import ClassVar, Protocol, TypeVar, get_args, get_origin

import pytest
from pydantic import JsonValue, TypeAdapter
from typing_extensions import get_type_hints, is_typeddict

from betterlite.types.llms.bedrock.converse import citations, messages, tools

PACKAGE = "betterlite.types.llms.bedrock.converse"
sent_event = importlib.import_module(f"{PACKAGE}.server_sent_event")
our_tools = importlib.import_module(f"{PACKAGE}.tools")


class TypedDictClass(Protocol):
    __required_keys__: ClassVar[frozenset[str]]
    __optional_keys__: ClassVar[frozenset[str]]
    __readonly_keys__: ClassVar[frozenset[str]]
    __total__: ClassVar[bool]


class Blocks(Protocol):
    def __getattr__(self, name: str) -> TypedDictClass: ...


T_co = TypeVar("T_co", covariant=True)


class ClassesOf(Protocol[T_co]):
    def __getattr__(self, name: str) -> type[T_co]: ...


class EventInstance(Protocol):
    event: str | None
    id: str | None
    retry: int | None
    data: str

    def json(self) -> JsonValue: ...


class EventFactory(Protocol):
    def __call__(self, *args: str, **fields: str | int | None) -> EventInstance: ...


class ToolSpecFactory(Protocol):
    def __call__(
        self,
        *,
        name: str,
        description: str,
        parameters: dict[str, JsonValue],
        strict: bool | None,
        supports_strict_tools: bool,
    ) -> dict[str, JsonValue]: ...


theirs: Blocks = importlib.import_module("litellm.types.llms.bedrock")
openai: Blocks = importlib.import_module("litellm.types.llms.openai")
bedrock_citations: ClassesOf[citations.CitationsContentBlock] = importlib.import_module(
    "litellm.types.llms.bedrock",
)
bedrock_content: ClassesOf[messages.ContentBlock] = importlib.import_module(
    "litellm.types.llms.bedrock",
)
bedrock_response: ClassesOf[messages.ConverseResponseBlock] = importlib.import_module(
    "litellm.types.llms.bedrock",
)
ours_content: Blocks = importlib.import_module(f"{PACKAGE}.content")
ours_citations: Blocks = importlib.import_module(f"{PACKAGE}.citations")
ours_messages: Blocks = importlib.import_module(f"{PACKAGE}.messages")
ours_tools: Blocks = importlib.import_module(f"{PACKAGE}.tools")
ours_request: Blocks = importlib.import_module(f"{PACKAGE}.request")
ours_stream_events: Blocks = importlib.import_module(f"{PACKAGE}.stream_events")
ours_tool_call_chunk: Blocks = importlib.import_module(f"{PACKAGE}.tool_call_chunk")
bedrock_module = importlib.import_module("litellm.types.llms.bedrock")
OUR_EVENT: EventFactory = sent_event.ServerSentEvent
REFERENCE_EVENT: EventFactory = bedrock_module.ServerSentEvent
OUR_TOOL_SPEC: ToolSpecFactory = our_tools.BedrockToolSpec
REFERENCE_TOOL_SPEC: ToolSpecFactory = bedrock_module.BedrockToolSpec

OURS: dict[str, TypedDictClass] = {
    "CachePointBlock": ours_content.CachePointBlock,
    "SystemContentBlock": ours_content.SystemContentBlock,
    "SourceBlock": ours_content.SourceBlock,
    "ImageBlock": ours_content.ImageBlock,
    "VideoBlock": ours_content.VideoBlock,
    "DocumentBlock": ours_content.DocumentBlock,
    "SearchResultBlock": ours_content.SearchResultBlock,
    "ToolResultContentBlock": ours_content.ToolResultContentBlock,
    "ToolResultBlock": ours_content.ToolResultBlock,
    "ToolUseBlock": ours_content.ToolUseBlock,
    "BedrockConverseReasoningTextBlock": ours_content.BedrockConverseReasoningTextBlock,
    "BedrockConverseReasoningContentBlock": ours_content.BedrockConverseReasoningContentBlock,
    "BedrockConverseReasoningContentBlockDelta": (
        ours_content.BedrockConverseReasoningContentBlockDelta
    ),
    "BedrockConverseGptReasoningEffortBlock": ours_content.BedrockConverseGptReasoningEffortBlock,
    "GuardrailConverseTextBlock": ours_content.GuardrailConverseTextBlock,
    "GuardrailConverseContentBlock": ours_content.GuardrailConverseContentBlock,
    "CitationWebLocationBlock": ours_citations.CitationWebLocationBlock,
    "CitationSearchResultLocationBlock": ours_citations.CitationSearchResultLocationBlock,
    "CitationLocationBlock": ours_citations.CitationLocationBlock,
    "CitationReferenceBlock": ours_citations.CitationReferenceBlock,
    "CitationGeneratedContentBlock": ours_citations.CitationGeneratedContentBlock,
    "CitationsContentBlock": ours_citations.CitationsContentBlock,
    "ContentBlock": ours_messages.ContentBlock,
    "MessageBlock": ours_messages.MessageBlock,
    "ConverseMetricsBlock": ours_messages.ConverseMetricsBlock,
    "ConverseResponseOutputBlock": ours_messages.ConverseResponseOutputBlock,
    "CacheDetailBlock": ours_messages.CacheDetailBlock,
    "ConverseTokenUsageBlock": ours_messages.ConverseTokenUsageBlock,
    "ServiceTierBlock": ours_messages.ServiceTierBlock,
    "ConverseResponseBlock": ours_messages.ConverseResponseBlock,
    "ToolJsonSchemaBlock": ours_tools.ToolJsonSchemaBlock,
    "ToolInputSchemaBlock": ours_tools.ToolInputSchemaBlock,
    "ToolSpecBlock": ours_tools.ToolSpecBlock,
    "SystemToolBlock": ours_tools.SystemToolBlock,
    "ToolBlock": ours_tools.ToolBlock,
    "SpecificToolChoiceBlock": ours_tools.SpecificToolChoiceBlock,
    "ToolChoiceValuesBlock": ours_tools.ToolChoiceValuesBlock,
    "ToolConfigBlock": ours_tools.ToolConfigBlock,
    "GuardrailConfigBlock": ours_request.GuardrailConfigBlock,
    "InferenceConfig": ours_request.InferenceConfig,
    "PerformanceConfigBlock": ours_request.PerformanceConfigBlock,
    "JsonSchemaDefinition": ours_request.JsonSchemaDefinition,
    "OutputFormatStructure": ours_request.OutputFormatStructure,
    "OutputFormat": ours_request.OutputFormat,
    "OutputConfigBlock": ours_request.OutputConfigBlock,
    "CommonRequestObject": ours_request.CommonRequestObject,
    "RequestObject": ours_request.RequestObject,
    "BedrockInvokeNovaRequest": ours_request.BedrockInvokeNovaRequest,
    "ToolBlockDeltaEvent": ours_stream_events.ToolBlockDeltaEvent,
    "ToolUseBlockStartEvent": ours_stream_events.ToolUseBlockStartEvent,
    "ContentBlockStartEvent": ours_stream_events.ContentBlockStartEvent,
    "ContentBlockDeltaEvent": ours_stream_events.ContentBlockDeltaEvent,
    "GenericStreamingChunk": ours_stream_events.GenericStreamingChunk,
    "Document": ours_stream_events.Document,
    "ChatCompletionToolCallFunctionChunk": ours_tool_call_chunk.ChatCompletionToolCallFunctionChunk,
    "ChatCompletionToolCallChunk": ours_tool_call_chunk.ChatCompletionToolCallChunk,
}

REFERENCES: dict[str, TypedDictClass] = {
    "CachePointBlock": theirs.CachePointBlock,
    "SystemContentBlock": theirs.SystemContentBlock,
    "SourceBlock": theirs.SourceBlock,
    "ImageBlock": theirs.ImageBlock,
    "VideoBlock": theirs.VideoBlock,
    "DocumentBlock": theirs.DocumentBlock,
    "SearchResultBlock": theirs.SearchResultBlock,
    "ToolResultContentBlock": theirs.ToolResultContentBlock,
    "ToolResultBlock": theirs.ToolResultBlock,
    "ToolUseBlock": theirs.ToolUseBlock,
    "BedrockConverseReasoningTextBlock": theirs.BedrockConverseReasoningTextBlock,
    "BedrockConverseReasoningContentBlock": theirs.BedrockConverseReasoningContentBlock,
    "BedrockConverseReasoningContentBlockDelta": theirs.BedrockConverseReasoningContentBlockDelta,
    "BedrockConverseGptReasoningEffortBlock": theirs.BedrockConverseGptReasoningEffortBlock,
    "GuardrailConverseTextBlock": theirs.GuardrailConverseTextBlock,
    "GuardrailConverseContentBlock": theirs.GuardrailConverseContentBlock,
    "CitationWebLocationBlock": theirs.CitationWebLocationBlock,
    "CitationSearchResultLocationBlock": theirs.CitationSearchResultLocationBlock,
    "CitationLocationBlock": theirs.CitationLocationBlock,
    "CitationReferenceBlock": theirs.CitationReferenceBlock,
    "CitationGeneratedContentBlock": theirs.CitationGeneratedContentBlock,
    "CitationsContentBlock": theirs.CitationsContentBlock,
    "ContentBlock": theirs.ContentBlock,
    "MessageBlock": theirs.MessageBlock,
    "ConverseMetricsBlock": theirs.ConverseMetricsBlock,
    "ConverseResponseOutputBlock": theirs.ConverseResponseOutputBlock,
    "CacheDetailBlock": theirs.CacheDetailBlock,
    "ConverseTokenUsageBlock": theirs.ConverseTokenUsageBlock,
    "ServiceTierBlock": theirs.ServiceTierBlock,
    "ConverseResponseBlock": theirs.ConverseResponseBlock,
    "ToolJsonSchemaBlock": theirs.ToolJsonSchemaBlock,
    "ToolInputSchemaBlock": theirs.ToolInputSchemaBlock,
    "ToolSpecBlock": theirs.ToolSpecBlock,
    "SystemToolBlock": theirs.SystemToolBlock,
    "ToolBlock": theirs.ToolBlock,
    "SpecificToolChoiceBlock": theirs.SpecificToolChoiceBlock,
    "ToolChoiceValuesBlock": theirs.ToolChoiceValuesBlock,
    "ToolConfigBlock": theirs.ToolConfigBlock,
    "GuardrailConfigBlock": theirs.GuardrailConfigBlock,
    "InferenceConfig": theirs.InferenceConfig,
    "PerformanceConfigBlock": theirs.PerformanceConfigBlock,
    "JsonSchemaDefinition": theirs.JsonSchemaDefinition,
    "OutputFormatStructure": theirs.OutputFormatStructure,
    "OutputFormat": theirs.OutputFormat,
    "OutputConfigBlock": theirs.OutputConfigBlock,
    "CommonRequestObject": theirs.CommonRequestObject,
    "RequestObject": theirs.RequestObject,
    "BedrockInvokeNovaRequest": theirs.BedrockInvokeNovaRequest,
    "ToolBlockDeltaEvent": theirs.ToolBlockDeltaEvent,
    "ToolUseBlockStartEvent": theirs.ToolUseBlockStartEvent,
    "ContentBlockStartEvent": theirs.ContentBlockStartEvent,
    "ContentBlockDeltaEvent": theirs.ContentBlockDeltaEvent,
    "GenericStreamingChunk": theirs.GenericStreamingChunk,
    "Document": theirs.Document,
    "ChatCompletionToolCallFunctionChunk": openai.ChatCompletionToolCallFunctionChunk,
    "ChatCompletionToolCallChunk": openai.ChatCompletionToolCallChunk,
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

CITATIONS: citations.CitationsContentBlock = {
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

CONTENT: messages.ContentBlock = {
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

RESPONSE: messages.ConverseResponseBlock = {
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

TOOL_SPEC_CASES: list[tuple[dict[str, JsonValue], bool | None, bool]] = [
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

SSE_FIELDS: list[dict[str, str | int | None]] = [
    {},
    {"event": "message", "data": '{"a": 1}', "id": "7", "retry": 3000},
    {"event": "", "data": "x"},
    {"event": None, "data": None, "id": None, "retry": None},
    {"data": "plain text", "id": "", "retry": 0},
]


def shape(annotation: type) -> str:
    origin: type | None = get_origin(annotation)
    arguments: tuple[type, ...] = get_args(annotation)
    if is_typeddict(annotation):
        return f"TypedDict:{annotation.__name__}"
    if origin is None:
        return annotation.__name__
    name = origin.__name__
    if name == "Literal":
        return f"Literal{list(arguments)}"
    if name in ("Union", "UnionType"):
        return "|".join(sorted(shape(argument) for argument in arguments))
    if name == "dict" and arguments[1].__name__ in ("Any", "JsonValue"):
        return "dict"
    return name + "[" + ",".join(shape(argument) for argument in arguments) + "]"


@pytest.mark.parametrize("name", OURS)
def test_names_typed_dict_matches(name: str) -> None:
    ours = OURS[name]
    reference = REFERENCES[name]
    our_hints: dict[str, type] = get_type_hints(ours)
    reference_hints: dict[str, type] = get_type_hints(reference)

    assert ours.__required_keys__ == reference.__required_keys__
    assert ours.__optional_keys__ == reference.__optional_keys__
    assert ours.__readonly_keys__ == reference.__readonly_keys__
    assert ours.__total__ == reference.__total__
    assert list(our_hints) == list(reference_hints)
    for key, hint in reference_hints.items():
        assert shape(our_hints[key]) == shape(hint), key


@pytest.mark.parametrize("name", REQUIRED_KEYS)
def test_names_required_keys(name: str) -> None:
    assert OURS[name].__required_keys__ == REQUIRED_KEYS[name]
    assert REFERENCES[name].__required_keys__ == REQUIRED_KEYS[name]


@pytest.mark.parametrize("name", READ_ONLY_KEYS)
def test_names_read_only_keys(name: str) -> None:
    assert OURS[name].__readonly_keys__ == READ_ONLY_KEYS[name]
    assert REFERENCES[name].__readonly_keys__ == READ_ONLY_KEYS[name]


@pytest.mark.parametrize("name", ["ImageBlock", "VideoBlock", "DocumentBlock"])
def test_names_literal_values(name: str) -> None:
    our_hints: dict[str, type] = get_type_hints(OURS[name])
    reference_hints: dict[str, type] = get_type_hints(REFERENCES[name])

    assert shape(our_hints["format"]) == shape(reference_hints["format"])
    assert "Literal[" in shape(our_hints["format"])


def test_names_content_optional() -> None:
    assert OURS["ContentBlock"].__required_keys__ == frozenset()
    assert REFERENCES["ContentBlock"].__required_keys__ == frozenset()
    assert OURS["ContentBlock"].__optional_keys__ == frozenset(
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
    assert OURS["Document"].__required_keys__ == frozenset({"title", "snippet"})
    assert OURS["DocumentBlock"].__required_keys__ == frozenset({"format", "source", "name"})
    assert OURS["Document"] is not OURS["DocumentBlock"]


def test_names_citations_validate() -> None:
    ours = TypeAdapter(citations.CitationsContentBlock)
    reference = TypeAdapter(bedrock_citations.CitationsContentBlock)

    assert ours.validate_python(CITATIONS) == CITATIONS
    assert reference.validate_python(CITATIONS) == CITATIONS


def test_names_blocks_validate() -> None:
    our_content = TypeAdapter(messages.ContentBlock)
    our_response = TypeAdapter(messages.ConverseResponseBlock)
    reference_content = TypeAdapter(bedrock_content.ContentBlock)
    reference_response = TypeAdapter(bedrock_response.ConverseResponseBlock)

    assert our_content.validate_python(CONTENT) == CONTENT
    assert reference_content.validate_python(CONTENT) == CONTENT
    assert our_response.validate_python(RESPONSE) == RESPONSE
    assert reference_response.validate_python(RESPONSE) == RESPONSE


@pytest.mark.parametrize("case", TOOL_SPEC_CASES)
def test_names_tool_spec_matches(
    case: tuple[dict[str, JsonValue], bool | None, bool],
) -> None:
    parameters, strict, supports_strict_tools = case

    ours = OUR_TOOL_SPEC(
        name="weather",
        description="Get the weather",
        parameters=parameters,
        strict=strict,
        supports_strict_tools=supports_strict_tools,
    )
    reference = REFERENCE_TOOL_SPEC(
        name="weather",
        description="Get the weather",
        parameters=parameters,
        strict=strict,
        supports_strict_tools=supports_strict_tools,
    )

    assert json.dumps(ours) == json.dumps(reference)
    assert list(ours) == ["toolSpec"]
    assert dict in tools.BedrockToolSpec.__mro__


def test_names_tool_spec_type() -> None:
    for build in (REFERENCE_TOOL_SPEC, OUR_TOOL_SPEC):
        with pytest.raises(KeyError):
            build(
                name="n",
                description="d",
                parameters={},
                strict=None,
                supports_strict_tools=False,
            )


@pytest.mark.parametrize("fields", SSE_FIELDS)
def test_sse_members_match_litellm(fields: dict[str, str | int | None]) -> None:
    ours = OUR_EVENT(**fields)
    reference = REFERENCE_EVENT(**fields)

    assert ours.event == reference.event
    assert ours.id == reference.id
    assert ours.retry == reference.retry
    assert ours.data == reference.data
    assert repr(ours) == repr(reference)


def test_sse_defaults() -> None:
    assert OUR_EVENT().event is None
    assert OUR_EVENT().data == ""
    assert OUR_EVENT(data=None).data == ""
    assert OUR_EVENT(event="").event is None
    assert OUR_EVENT(event="e").event == "e"
    assert OUR_EVENT(id="").id == ""
    assert OUR_EVENT(retry=0).retry == 0


def test_sse_repr_text() -> None:
    event = OUR_EVENT(event="message", data="hi", id="7", retry=3)

    assert repr(event) == "ServerSentEvent(event=message, data=hi, id=7, retry=3)"
    assert repr(OUR_EVENT()) == "ServerSentEvent(event=None, data=, id=None, retry=None)"


@pytest.mark.parametrize("data", ['{"a": [1, 2.5, null]}', "[]", '"text"', "7", "null", "true"])
def test_sse_json_parses(data: str) -> None:
    ours = OUR_EVENT(data=data).json()
    reference = REFERENCE_EVENT(data=data).json()

    assert ours == reference


@pytest.mark.parametrize("data", [None, "", "not json", "{"])
def test_sse_json_rejects(data: str | None) -> None:
    with pytest.raises(json.JSONDecodeError):
        REFERENCE_EVENT(data=data).json()
    with pytest.raises(json.JSONDecodeError):
        OUR_EVENT(data=data).json()


def test_sse_rejects_bad_arguments() -> None:
    for build in (REFERENCE_EVENT, OUR_EVENT):
        with pytest.raises(TypeError, match="bogus"):
            build(bogus=1)
        with pytest.raises(TypeError):
            build("event")


def test_sse_members_read_only() -> None:
    for build in (REFERENCE_EVENT, OUR_EVENT):
        event = build(event="e", data="d", id="i", retry=1)
        with pytest.raises(AttributeError):
            event.event = "changed"
        with pytest.raises(AttributeError):
            event.data = "changed"
        with pytest.raises(AttributeError):
            event.id = "changed"
        with pytest.raises(AttributeError):
            event.retry = 2
