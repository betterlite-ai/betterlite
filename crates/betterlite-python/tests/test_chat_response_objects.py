import json
import logging
import types
import uuid
from abc import ABC
from datetime import UTC, datetime
from typing import Generic, Protocol, Self, TypeAlias, TypeVar, cast, get_args

import httpx
import litellm.types.completion as litellm_completion  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/completion.py#L19
import litellm.types.utils as litellm_utils  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1
import pydantic
import pytest
from openai.types import CompletionUsage
from openai.types.chat import ChatCompletionMessage
from openai.types.chat.chat_completion import Choice
from openai.types.chat.chat_completion_chunk import Choice as ChunkChoice
from openai.types.chat.chat_completion_chunk import ChoiceDelta
from openai.types.completion_usage import PromptTokensDetails
from typing_extensions import override

import betterlite.types.utils as betterlite_utils
from betterlite.types import completion as betterlite_completion
from betterlite.types.utils import (
    AwareDatetime,
    Base64Str,
    JsonValue,
    ResponseObject,
    RootModel,
    ValidationError,
    field_validator,
    model_validator,
)

BoxValue = TypeVar("BoxValue", bound=JsonValue)
LARGE_COUNT = 10
READ_TOKENS = 3
CREATED_TOKENS = 4
CACHE_WRITE_TOKENS = 3
REASONING_TOKENS = 4
TEXT_TOKENS = 6
PROMPT_CACHE_TOKENS = 2
STREAM_INDEX = 2
SERVER_COST = 0.5
DOUBLED_COUNT = 6
BOX_VALUE = 5
SAMPLE_TEXT = "a"
PROVIDER: dict[str, JsonValue] = {"a": 1}
EMPTY_PROVIDER: dict[str, JsonValue] = {}
FUNCTION_CALL: dict[str, JsonValue] = {"name": "f", "arguments": "{}"}
TOOL_CALL: dict[str, JsonValue] = {"id": "1", "function": {"name": "f", "arguments": "{}"}}
CUSTOM_TOOL_CALL: dict[str, JsonValue] = {
    "id": "1",
    "type": "custom",
    "custom": {"name": "n", "input": "i"},
}
TOOL_CALLS: list[JsonValue] = [TOOL_CALL]
EMPTY_LIST: list[JsonValue] = []
ANNOTATIONS: list[JsonValue] = [{"type": "url_citation"}]
IMAGES: list[JsonValue] = [{"image_url": {"url": "u"}, "index": 0, "type": "image_url"}]
MESSAGE_TEXT: dict[str, JsonValue] = {"content": "x"}
DELTA_TEXT: dict[str, JsonValue] = {"content": "y"}
LOGPROBS: dict[str, JsonValue] = {
    "content": [{"token": "a", "logprob": -1.0, "top_logprobs": []}]
}
PROMPT_DETAILS: dict[str, JsonValue] = {"cached_tokens": 2, "audio_tokens": 1}
CACHED_DETAILS: dict[str, JsonValue] = {"cached_tokens": 2}
COMPLETION_DETAILS: dict[str, JsonValue] = {"image_tokens": 3, "audio_tokens": 2}
NESTED_CACHE: dict[str, JsonValue] = {"cache_creation_input_tokens": 7}
SERVER_TOOL_USE: dict[str, JsonValue] = {"web_search_requests": 1}
USAGE_VALUES: dict[str, JsonValue] = {
    "prompt_tokens": 1,
    "completion_tokens": 2,
    "total_tokens": 3,
}
ENHANCEMENTS: dict[str, JsonValue] = {"e": 1}
MISMATCHED_TOKENS = 7
UUID_VERSION = 4
HIDDEN_PARAMS: dict[str, JsonValue] = {"a": 1}
HEADERS: dict[str, JsonValue] = {"h": "v"}
EXTRA_TOOL_USE: dict[str, int] = {"web_search_requests": 1, "web_fetch_requests": 2}
EXTRA_TOKEN_COUNT: dict[str, int] = {"extra": 2}
ARGS: dict[str, JsonValue] = {"a": 1}
FUNCTION_ARGS: dict[str, JsonValue] = {"b": 2}
TOOL_FUNCTION: dict[str, JsonValue] = {"name": "f"}
CHOICE_MESSAGE_LENGTH: dict[str, JsonValue] = {
    "message": {"content": "y"},
    "finish_reason": "length",
}
CHOICES: list[JsonValue] = [CHOICE_MESSAGE_LENGTH]
AUDIO_CHUNK: dict[str, JsonValue] = {"id": "a1", "transcript": "hi"}
CUSTOM_DELTA_CALL: dict[str, JsonValue] = {
    "type": "custom",
    "id": "c",
    "custom": {"name": "n", "input": "i"},
    "function": None,
}


Member: TypeAlias = JsonValue | type | types.UnionType


def parse_object(text: str) -> dict[str, JsonValue]:
    parsed: dict[str, JsonValue] = json.loads(text)
    return parsed


def union_names(alias: Member) -> list[str]:
    members = cast("tuple[Member, ...]", get_args(alias))  # https://github.com/python/typeshed/blob/289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad/stdlib/typing.pyi#L1012
    return [str(member).rsplit(".", 1)[-1].strip("'>)") for member in members]


class Dumpable(Protocol):
    def model_dump(
        self,
        *,
        include: set[str] | None = None,
        exclude: set[str] | None = None,
        exclude_unset: bool = False,
        exclude_none: bool = False,
    ) -> JsonValue: ...

    def model_dump_json(self) -> str: ...

    def dict(self, *, exclude_none: bool = False) -> JsonValue: ...


class RecordingHandler(logging.Handler):
    messages: list[str]

    def __init__(self) -> None:
        super().__init__()
        self.messages = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


class Box(ResponseObject, Generic[BoxValue]):
    value: BoxValue

    def __init__(self, value: BoxValue) -> None:
        super().__init__(value=value)


class Named(ResponseObject, ABC):
    id: str
    name: str


class Payload(ResponseObject):
    data: Base64Str
    seen: AwareDatetime


class Doubled(ResponseObject):
    count: int = 0

    @field_validator("count", mode="before")
    @classmethod
    def double(cls, value: JsonValue) -> JsonValue:
        if value is None:
            return None
        match value:
            case int():
                return value * 2
            case float() | str() | list() | dict():
                return value

    @field_validator("count", mode="after")
    @classmethod
    def reject_negative(cls, value: int) -> int:
        if value < 0:
            raise ValueError("count is negative")
        return value


class Checked(ResponseObject):
    count: int = 0

    @model_validator(mode="after")
    def reject_large(self) -> Self:
        if self.count > LARGE_COUNT:
            raise ValueError("count is too large")
        return self


def assert_same(label: str, reference: Dumpable, ours: Dumpable) -> None:
    assert ours.model_dump() == reference.model_dump(), label
    assert ours.model_dump(exclude_unset=True) == reference.model_dump(exclude_unset=True), label
    assert ours.model_dump(exclude_none=True) == reference.model_dump(exclude_none=True), label


def assert_full_dump(label: str, reference: Dumpable, ours: Dumpable) -> None:
    dumped = reference.model_dump(exclude_unset=False)
    assert ours.model_dump(exclude_unset=False) == dumped, label


def assert_selected(
    label: str, reference: Dumpable, ours: Dumpable, *,
    include: set[str] | None = None, exclude: set[str] | None = None,
) -> None:
    expected = reference.model_dump(include=include, exclude=exclude)
    assert ours.model_dump(include=include, exclude=exclude) == expected, label


def test_access_attribute_and_item() -> None:
    ours = betterlite_utils.Message(content="x", extra_field="e")
    reference = litellm_utils.Message(content="x", extra_field="e")
    assert ours.content == reference.content
    assert ours["content"] == reference.content
    assert ours["extra_field"] == "e"
    assert ours.role == reference.role
    assert_same("access", reference, ours)


def test_access_get_and_contains() -> None:
    ours = betterlite_utils.Choices()
    reference = litellm_utils.Choices()
    assert ours.get("finish_reason") == "stop"
    assert ours.get("missing") is None
    assert "finish_reason" in ours
    assert "finish_reason" in reference
    assert "missing" not in ours
    assert "missing" not in reference
    with pytest.raises(AttributeError):
        ours["missing"]
    with pytest.raises(AttributeError):
        reference["missing"]


def test_access_delete_missing() -> None:
    message = betterlite_utils.Message(content="x")
    del message["missing"]
    message["extra"] = 1
    assert message["extra"] == 1
    del message["extra"]
    del message.content
    assert message.get("content") is None
    reference = litellm_utils.Message(content="x")
    del reference.content
    assert_same("delete", reference, message)


def test_access_dict_parity() -> None:
    ours = betterlite_utils.ModelResponse(id="i", created=5, usage=USAGE_VALUES)
    reference: Dumpable = litellm_utils.ModelResponse(id="i", created=5, usage=USAGE_VALUES)
    assert ours.dict() == reference.dict()  # pyright: ignore[reportDeprecated]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1499-L1504
    assert ours.dict(exclude_none=True) == reference.dict(exclude_none=True)  # pyright: ignore[reportDeprecated]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1499-L1504
    message = betterlite_utils.Message(content="x")
    reference_message: Dumpable = litellm_utils.Message(content="x")
    assert message.dict() == reference_message.dict()  # pyright: ignore[reportDeprecated]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1499-L1504
    assert message.dict(exclude_none=True) == reference_message.dict(exclude_none=True)  # pyright: ignore[reportDeprecated]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1499-L1504
    usage = betterlite_utils.Usage()
    assert parse_object(usage.json(exclude_none=True)) == parse_object(
        litellm_utils.Usage().model_dump_json(exclude_none=True))


def test_dump_message_choices() -> None:
    ours_message = betterlite_utils.Message
    ours_choices = betterlite_utils.Choices
    cases: list[tuple[str, Dumpable, Dumpable]] = [
        ("message empty", litellm_utils.Message(), ours_message()),
        ("message content", litellm_utils.Message(content="hi"), ours_message(content="hi")),
        (
            "message role",
            litellm_utils.Message(content="hi", role="user"),
            ours_message(content="hi", role="user"),
        ),
        ("message none", litellm_utils.Message(content=None), ours_message(content=None)),
        (
            "message reasoning",
            litellm_utils.Message(content="hi", reasoning_content="r"),
            ours_message(content="hi", reasoning_content="r"),
        ),
        (
            "message provider",
            litellm_utils.Message(content="hi", provider_specific_fields=PROVIDER),
            ours_message(content="hi", provider_specific_fields=PROVIDER),
        ),
        (
            "message extra",
            litellm_utils.Message(content="hi", extra_field="x"),
            ours_message(content="hi", extra_field="x"),
        ),
        (
            "message annotations",
            litellm_utils.Message(content="hi", annotations=ANNOTATIONS),  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1415
            ours_message(content="hi", annotations=ANNOTATIONS),
        ),
        (
            "message function call",
            litellm_utils.Message(content=None, function_call=FUNCTION_CALL),
            ours_message(content=None, function_call=FUNCTION_CALL),
        ),
        ("choices empty", litellm_utils.Choices(), ours_choices()),
        (
            "choices tool use",
            litellm_utils.Choices(finish_reason="tool_use"),
            ours_choices(finish_reason="tool_use"),
        ),
        ("choices index", litellm_utils.Choices(index=2), ours_choices(index=2)),
        (
            "choices message",
            litellm_utils.Choices(message=MESSAGE_TEXT),  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1656
            ours_choices(message=MESSAGE_TEXT),
        ),
        (
            "choices enhancements",
            litellm_utils.Choices(enhancements=ENHANCEMENTS),
            ours_choices(enhancements=ENHANCEMENTS),
        ),
        (
            "choices provider empty",
            litellm_utils.Choices(provider_specific_fields=EMPTY_PROVIDER),
            ours_choices(provider_specific_fields=EMPTY_PROVIDER),
        ),
        (
            "choices logprobs",
            litellm_utils.Choices(logprobs=LOGPROBS),  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1648
            ours_choices(logprobs=LOGPROBS),
        ),
    ]
    for label, reference, item in cases:
        assert_same(label, reference, item)


def test_dump_delta_parity() -> None:
    ours = betterlite_utils.Delta
    cases: list[tuple[str, Dumpable, Dumpable]] = [
        ("empty", litellm_utils.Delta(), ours()),
        ("content", litellm_utils.Delta(content="x"), ours(content="x")),
        ("role", litellm_utils.Delta(role="assistant"), ours(role="assistant")),
        (
            "reasoning",
            litellm_utils.Delta(reasoning="r"),
            ours(reasoning="r"),
        ),
        (
            "provider",
            litellm_utils.Delta(provider_specific_fields=PROVIDER),
            ours(provider_specific_fields=PROVIDER),
        ),
        (
            "images empty",
            litellm_utils.Delta(images=EMPTY_LIST),  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1518
            ours(images=EMPTY_LIST),
        ),
        (
            "images and annotations",
            litellm_utils.Delta(images=IMAGES, annotations=ANNOTATIONS),  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1518-L1537
            ours(images=IMAGES, annotations=ANNOTATIONS),
        ),
        (
            "function call",
            litellm_utils.Delta(function_call=FUNCTION_CALL),
            ours(function_call=FUNCTION_CALL),
        ),
        (
            "audio",
            litellm_utils.Delta(audio=dict(AUDIO_CHUNK)),  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1517
            ours(audio=dict(AUDIO_CHUNK)),
        ),
    ]
    for label, reference, item in cases:
        assert_same(label, reference, item)
    stored_call = ours(function_call=FUNCTION_CALL).function_call
    assert stored_call == betterlite_utils.FunctionCall(name="f", arguments="{}")
    stored_reference = litellm_utils.Delta(function_call=FUNCTION_CALL).function_call
    assert stored_reference is not None
    assert stored_reference.name == "f"
    reasoned = betterlite_utils.Delta(reasoning="r")
    reference_reasoned = litellm_utils.Delta(reasoning="r")
    assert reasoned.reasoning_content == reference_reasoned.reasoning_content
    assert "reasoning" not in reasoned
    assert "images" not in betterlite_utils.Delta(images=EMPTY_LIST)
    assert betterlite_utils.Delta(audio=dict(AUDIO_CHUNK)).audio == AUDIO_CHUNK
    extra = betterlite_utils.Delta(foo="bar")
    assert parse_object(extra.model_dump_json())["foo"] == "bar"


def test_dump_usage_parity() -> None:
    ours = betterlite_utils.Usage
    assert_same("empty", litellm_utils.Usage(), ours())
    assert_same(
        "counts",
        litellm_utils.Usage(prompt_tokens=10, completion_tokens=5, total_tokens=15),
        ours(prompt_tokens=10, completion_tokens=5, total_tokens=15),
    )
    assert_same(
        "reasoning",
        litellm_utils.Usage(completion_tokens=10, reasoning_tokens=4),
        ours(completion_tokens=10, reasoning_tokens=4),
    )
    assert_same(
        "cache hit",
        litellm_utils.Usage(prompt_tokens=5, prompt_cache_hit_tokens=3),
        ours(prompt_tokens=5, prompt_cache_hit_tokens=3),
    )
    assert_same(
        "cache read",
        litellm_utils.Usage(prompt_tokens=5, cache_read_input_tokens=3),
        ours(prompt_tokens=5, cache_read_input_tokens=3),
    )
    assert_same(
        "cache creation",
        litellm_utils.Usage(prompt_tokens=5, cache_creation_input_tokens=3),
        ours(prompt_tokens=5, cache_creation_input_tokens=3),
    )
    assert_same(
        "prompt details",
        litellm_utils.Usage(prompt_tokens_details=PROMPT_DETAILS),
        ours(prompt_tokens_details=PROMPT_DETAILS),
    )
    assert_same(
        "server and cost",
        litellm_utils.Usage(server_tool_use=SERVER_TOOL_USE, cost=0.5),
        ours(server_tool_use=SERVER_TOOL_USE, cost=0.5),
    )
    assert_same(
        "nested cache",
        litellm_utils.Usage(prompt_tokens_details=NESTED_CACHE),
        ours(prompt_tokens_details=NESTED_CACHE),
    )
    extra = betterlite_utils.Usage(foo="bar")
    assert parse_object(extra.model_dump_json())["foo"] == "bar"


def test_dump_unset_default() -> None:
    assert_same("response", litellm_utils.ModelResponse(id="i", created=5),
                betterlite_utils.ModelResponse(id="i", created=5))
    stream_reference = litellm_utils.ModelResponseStream(id="i", created=5)
    stream_ours = betterlite_utils.ModelResponseStream(id="i", created=5)
    assert "model" not in parse_object(stream_ours.model_dump_json(exclude_unset=True))
    assert_full_dump("stream full", stream_reference, stream_ours)


def test_dump_json_parity() -> None:
    ours = betterlite_utils.ModelResponse(id="i", created=5, model="m")
    reference: Dumpable = litellm_utils.ModelResponse(id="i", created=5, model="m")
    assert ours.json() == ours.model_dump()
    assert_same("response json", reference, ours)
    message = betterlite_utils.Message(content="x")
    reference_message: Dumpable = litellm_utils.Message(content="x")
    assert message.json() == message.model_dump()
    assert_same("message json", reference_message, message)


def test_tool_call_function_args() -> None:
    assert betterlite_utils.Function(arguments=ARGS).arguments == litellm_utils.Function(
        arguments=ARGS
    ).arguments
    assert betterlite_utils.Function(parameters=ARGS).arguments == litellm_utils.Function(
        parameters=ARGS
    ).arguments
    assert betterlite_utils.Function().arguments == litellm_utils.Function().arguments
    text = '{"a": 1}'
    assert betterlite_utils.Function(arguments=text).arguments == text
    assert_same(
        "function",
        litellm_utils.Function(arguments=FUNCTION_ARGS),
        betterlite_utils.Function(arguments=FUNCTION_ARGS),
    )


def test_tool_call_ids_and_types() -> None:
    ours = betterlite_utils.ChatCompletionMessageToolCall(function=TOOL_FUNCTION)
    reference = litellm_utils.ChatCompletionMessageToolCall(function=TOOL_FUNCTION)
    assert ours.type == reference.type
    assert ours.type == "function"
    assert uuid.UUID(ours.id).version == UUID_VERSION
    again = betterlite_utils.ChatCompletionMessageToolCall(function=TOOL_FUNCTION)
    assert again.id != ours.id
    assert ours.id != reference.id
    explicit = betterlite_utils.ChatCompletionMessageToolCall(
        function=TOOL_FUNCTION, id="call", type="custom"
    )
    reference_explicit = litellm_utils.ChatCompletionMessageToolCall(
        function=TOOL_FUNCTION, id="call", type="custom"
    )
    assert explicit.id == "call"
    assert explicit.type == "custom"
    assert_same("ids", reference_explicit, explicit)
    audio = betterlite_utils.ChatCompletionAudioResponse(data="d", expires_at=1, transcript="t")
    again_audio = betterlite_utils.ChatCompletionAudioResponse(
        data="d", expires_at=1, transcript="t"
    )
    assert uuid.UUID(audio.id).version == UUID_VERSION
    assert again_audio.id != audio.id
    audio_explicit = betterlite_utils.ChatCompletionAudioResponse(
        data="d", expires_at=1, transcript="t", id="a"
    )
    reference_audio = litellm_utils.ChatCompletionAudioResponse(
        data="d", expires_at=1, transcript="t", id="a"
    )
    assert audio_explicit.id == "a"
    assert_same("audio", reference_audio, audio_explicit)


def test_tool_call_custom_helpers() -> None:
    assert betterlite_utils.is_custom_tool_call_dict({"type": "custom"})
    assert betterlite_utils.is_custom_tool_call_dict({"custom": {"name": "n"}})
    assert not betterlite_utils.is_custom_tool_call_dict({"type": "function"})
    payload: dict[str, JsonValue] = {
        "type": "custom",
        "id": "call",
        "custom": {"name": "n", "input": "i"},
        "function": None,
    }
    ours = betterlite_utils.chat_completion_tool_call_from_dict(payload)
    reference: Dumpable = litellm_utils.chat_completion_tool_call_from_dict(payload)
    assert "custom" in ours
    assert "function" not in ours
    assert ours.model_dump() == reference.model_dump()
    payload = {"type": "function", "id": "call", "function": {"name": "f", "arguments": "{}"}}
    called = betterlite_utils.chat_completion_tool_call_from_dict(payload)
    called_reference: Dumpable = litellm_utils.chat_completion_tool_call_from_dict(payload)
    assert "function" in called
    assert called.model_dump() == called_reference.model_dump()


def test_tool_call_delta_indexing() -> None:
    calls: list[dict[str, JsonValue]] = [
        {"function": {"name": "f", "arguments": ""}},
        {"id": "a", "index": 3, "function": {"name": "g", "arguments": ""}},
        {"function": {"name": "h", "arguments": ""}},
    ]
    ours = betterlite_utils.Delta(tool_calls=calls)
    assert "index" not in calls[0]
    assert "type" not in calls[0]
    copies: list[JsonValue] = [dict(entry) for entry in calls]
    reference = litellm_utils.Delta(tool_calls=copies)
    assert_same("delta indexing", reference, ours)
    ours_calls = ours.tool_calls
    assert ours_calls is not None
    assert [call.index for call in ours_calls] == [0, 3, 1]


def test_tool_call_delta_custom() -> None:
    calls_in: list[JsonValue] = [CUSTOM_DELTA_CALL.copy()]
    ours = betterlite_utils.Delta(tool_calls=calls_in)
    reference: Dumpable = litellm_utils.Delta(tool_calls=calls_in)
    assert_same("custom delta", reference, ours)
    calls = ours.tool_calls
    assert calls is not None
    assert calls[0].index == 0


def test_message_none_fields() -> None:
    reference = litellm_utils.Message(content="x", audio=None, images=None, annotations=None)
    ours = betterlite_utils.Message(content="x", audio=None, images=None, annotations=None)
    assert_same("none fields", reference, ours)
    for name in ("audio", "images", "annotations", "reasoning_content", "thinking_blocks"):
        assert name not in ours
    calls_message = betterlite_utils.Message(content="x", tool_calls=TOOL_CALLS)
    calls = calls_message.tool_calls
    assert calls is not None
    assert "function" in calls[0]
    tool_reference = litellm_utils.Message(content="x", tool_calls=TOOL_CALLS)
    assert_same("tool calls", tool_reference, calls_message)
    assert betterlite_utils.Message(content="x", tool_calls=EMPTY_LIST).tool_calls is None
    provided = betterlite_utils.Message(content="x", provider_specific_fields=PROVIDER)
    reference_provided = litellm_utils.Message(content="x", provider_specific_fields=PROVIDER)
    assert_same("provider", reference_provided, provided)
    absent = betterlite_utils.Message(content="x")
    assert "provider_specific_fields" not in parse_object(
        absent.model_dump_json(exclude_unset=True)
    )


def test_choices_native_reason() -> None:
    ours = betterlite_utils.Choices(finish_reason="tool_use")
    assert ours.finish_reason == "tool_calls"
    assert ours.provider_specific_fields == {"native_finish_reason": "tool_use"}
    assert_same("native reason", litellm_utils.Choices(finish_reason="tool_use"), ours)
    mapped = betterlite_utils.Choices(finish_reason="stop")
    assert "provider_specific_fields" not in mapped


def test_choices_unknown_warns() -> None:
    handler = RecordingHandler()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(handler)
    try:
        ours = betterlite_utils.Choices(finish_reason="bogus")
    finally:
        logger.removeHandler(handler)
    reference = litellm_utils.Choices(finish_reason="bogus")
    assert ours.finish_reason == reference.finish_reason
    assert ours.finish_reason == "stop"
    assert handler.messages == ["Unmapped finish_reason 'bogus', defaulting to 'stop'"]
    assert_same("bogus", reference, ours)


def test_choices_keep_model() -> None:
    message = betterlite_utils.Message(content="y")
    reference_message = litellm_utils.Message(content="y")
    ours = betterlite_utils.Choices(provider_specific_fields={"x": message})
    reference_fields: dict[str, Dumpable] = {"x": reference_message}
    reference = litellm_utils.Choices(provider_specific_fields=reference_fields)
    stored = ours.provider_specific_fields
    assert stored is not None
    assert stored["x"] == message.model_dump(exclude_unset=True)
    reference_dumpable: Dumpable = reference
    assert ours.model_dump(exclude_unset=True) == reference_dumpable.model_dump(
        exclude_unset=True,
    )


def test_choices_defaults() -> None:
    reference = litellm_utils.Choices()
    ours = betterlite_utils.Choices(index=None)
    assert (ours.index, "logprobs" in ours) == (reference.index, False)
    assert_same("choices defaults", reference, ours)
    assert betterlite_utils.StreamingChoices(index=None).index == 0


def test_choices_logprobs() -> None:
    holder = betterlite_utils.ChatCompletionTokenLogprob(
        token=SAMPLE_TEXT, logprob=-1.0, top_logprobs=None
    )
    assert holder.top_logprobs == []
    reference = litellm_utils.ChatCompletionTokenLogprob(
        token=SAMPLE_TEXT, logprob=-1.0, top_logprobs=[]
    )
    assert_same("token logprob", reference, holder)


def test_streaming_choices_message() -> None:
    ours = betterlite_utils.StreamingChoices(message=MESSAGE_TEXT, delta=DELTA_TEXT, index=2)
    reference = litellm_utils.StreamingChoices(
        message=MESSAGE_TEXT, delta=litellm_utils.Delta(content="y"), index=2
    )
    assert "message" not in ours
    assert ours.finish_reason is None
    assert parse_object(ours.model_dump_json())["index"] == STREAM_INDEX
    assert_same("streaming message", reference, ours)
    for reason in (None, "", "bogus", "length"):
        streamed = betterlite_utils.StreamingChoices(finish_reason=reason)
        streamed_ref = litellm_utils.StreamingChoices(finish_reason=reason)
        assert streamed.finish_reason == streamed_ref.finish_reason
        assert_same("streaming reason", streamed_ref, streamed)
    assert betterlite_utils.StreamingChoices(finish_reason="bogus").finish_reason == "stop"
    assert betterlite_utils.StreamingChoices(finish_reason=None).finish_reason is None


def test_usage_text_tokens() -> None:
    for arguments in ((10, 8, 3, 1), (10, 4, 4, 0), (10, 5, 4, 0)):
        ours = betterlite_utils.text_tokens_without_nested_reasoning(*arguments)
        reference = litellm_utils.text_tokens_without_nested_reasoning(*arguments)
        assert ours == reference


def test_usage_cache_tokens() -> None:
    ours = betterlite_utils.Usage(prompt_tokens=5, prompt_cache_hit_tokens=3)
    assert_same("cache hit", litellm_utils.Usage(prompt_tokens=5, prompt_cache_hit_tokens=3), ours)
    assert ours._cache_read_input_tokens == READ_TOKENS  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1858-L1863
    creation = betterlite_utils.Usage(prompt_tokens=5, cache_creation_input_tokens=3)
    assert creation._cache_creation_input_tokens == CACHE_WRITE_TOKENS  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1858-L1863
    dumped = parse_object(creation.model_dump_json())
    assert "cache_creation_input_tokens" in dumped
    assert "_cache_creation_input_tokens" not in dumped
    private = betterlite_utils.Usage(cache_read_input_tokens=3, cache_creation_input_tokens=4)
    assert private._cache_read_input_tokens == READ_TOKENS  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1858-L1863
    assert private._cache_creation_input_tokens == CREATED_TOKENS  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1858-L1863
    assert parse_object(private.model_dump_json())["cache_read_input_tokens"] == READ_TOKENS
    assert "_cache_read_input_tokens" not in parse_object(private.model_dump_json())


def test_usage_details_wrappers() -> None:
    ours = betterlite_utils.Usage(completion_tokens=10, reasoning_tokens=4)
    reference = litellm_utils.Usage(completion_tokens=10, reasoning_tokens=4)
    details = ours.completion_tokens_details
    assert details is not None
    assert details.text_tokens == TEXT_TOKENS
    assert details.reasoning_tokens == REASONING_TOKENS
    assert_same("reasoning details", reference, ours)
    cached = betterlite_utils.Usage(prompt_tokens_details=CACHED_DETAILS)
    reference_cached = litellm_utils.Usage(prompt_tokens_details=CACHED_DETAILS)
    prompt_details = cached.prompt_tokens_details
    assert prompt_details is not None
    assert prompt_details.cached_tokens == PROMPT_CACHE_TOKENS
    assert_same("prompt details", reference_cached, cached)
    openai_details = PromptTokensDetails(cached_tokens=2)
    with_openai = betterlite_utils.Usage(prompt_tokens=1, prompt_tokens_details=openai_details)
    reference_openai = litellm_utils.Usage(prompt_tokens=1, prompt_tokens_details=openai_details)
    assert_same("openai details", reference_openai, with_openai)


def test_usage_server_tool() -> None:
    ours = betterlite_utils.Usage(server_tool_use=SERVER_TOOL_USE, cost=0.5)
    reference = litellm_utils.Usage(server_tool_use=SERVER_TOOL_USE, cost=0.5)
    tool_use = ours.server_tool_use
    assert tool_use is not None
    assert tool_use.web_search_requests == 1
    assert ours.cost == SERVER_COST
    assert_same("server tool", reference, ours)
    assert "server_tool_use" not in betterlite_utils.Usage()
    assert "cost" not in betterlite_utils.Usage()


def test_usage_ignores_extras() -> None:
    ours = betterlite_utils.Usage(server_tool_use=EXTRA_TOOL_USE)
    reference: Dumpable = litellm_utils.Usage(server_tool_use=EXTRA_TOOL_USE)
    assert_same("server tool extra", reference, ours)
    tool_use = betterlite_utils.ServerToolUse(**EXTRA_TOOL_USE)
    reference_tool: Dumpable = litellm_utils.ServerToolUse(**EXTRA_TOOL_USE)
    assert tool_use.model_dump() == reference_tool.model_dump()
    with pytest.raises(KeyError):
        _ = tool_use["web_fetch_requests"]
    details = betterlite_utils.CacheCreationTokenDetails(
        ephemeral_5m_input_tokens=1, **EXTRA_TOKEN_COUNT,
    )
    reference_details: Dumpable = litellm_utils.CacheCreationTokenDetails(
        ephemeral_5m_input_tokens=1, **EXTRA_TOKEN_COUNT,
    )
    assert details.model_dump() == reference_details.model_dump()


def test_usage_token_details() -> None:
    ours = betterlite_utils.Usage(
        completion_tokens=10,
        reasoning_tokens=4,
        completion_tokens_details=COMPLETION_DETAILS,
    )
    reference = litellm_utils.Usage(
        completion_tokens=10,
        reasoning_tokens=4,
        completion_tokens_details=COMPLETION_DETAILS,
    )
    details = ours.completion_tokens_details
    assert details is not None
    assert details.text_tokens == 1
    assert_same("token details", reference, ours)
    wrapper = betterlite_utils.CompletionTokensDetailsWrapper(reasoning_tokens=2, audio_tokens=1)
    assert_same(
        "wrapper",
        litellm_utils.CompletionTokensDetailsWrapper(reasoning_tokens=2, audio_tokens=1),
        wrapper,
    )
    prompt_wrapper = betterlite_utils.PromptTokensDetailsWrapper(cache_write_tokens=3)
    assert prompt_wrapper.cache_creation_tokens == CACHE_WRITE_TOKENS
    assert_same(
        "prompt wrapper",
        litellm_utils.PromptTokensDetailsWrapper(cache_write_tokens=3),
        prompt_wrapper,
    )
    creation_wrapper = betterlite_utils.PromptTokensDetailsWrapper(
        cache_creation_tokens=MISMATCHED_TOKENS,
    )
    assert creation_wrapper.cache_write_tokens == MISMATCHED_TOKENS
    assert_same(
        "prompt wrapper creation",
        litellm_utils.PromptTokensDetailsWrapper(cache_creation_tokens=MISMATCHED_TOKENS),
        creation_wrapper,
    )
    both_wrapper = betterlite_utils.PromptTokensDetailsWrapper(
        cache_write_tokens=CACHE_WRITE_TOKENS, cache_creation_tokens=MISMATCHED_TOKENS,
    )
    assert both_wrapper.cache_creation_tokens == CACHE_WRITE_TOKENS
    assert_same(
        "prompt wrapper both",
        litellm_utils.PromptTokensDetailsWrapper(
            cache_write_tokens=CACHE_WRITE_TOKENS, cache_creation_tokens=MISMATCHED_TOKENS,
        ),
        both_wrapper,
    )


def test_model_response_defaults() -> None:
    ours = betterlite_utils.ModelResponse(id="x", created=1)
    assert ours.object == "chat.completion"
    assert ours.model is None
    assert_same("response defaults", litellm_utils.ModelResponse(id="x", created=1), ours)
    forced = betterlite_utils.ModelResponse(id="i", created=5, object="text_completion")
    assert forced.object == "chat.completion"
    generated = betterlite_utils.ModelResponse()
    assert generated.id.startswith("chatcmpl-")
    assert generated.created > 0


def test_model_response_usage() -> None:
    ours = betterlite_utils.ModelResponse(id="i", created=5, usage=USAGE_VALUES)
    assert_same("response usage",
                litellm_utils.ModelResponse(id="i", created=5, usage=USAGE_VALUES), ours)
    empty = betterlite_utils.ModelResponse(id="i", created=5)
    assert "usage" not in parse_object(empty.model_dump_json())


def test_model_response_headers() -> None:
    ours = betterlite_utils.ModelResponse(id="i", created=5)
    reference = litellm_utils.ModelResponse(id="i", created=5)
    headers = httpx.Headers(
        {
            "x-ratelimit-limit-requests": "10",
            "llm_provider-x": "y",
            "content-type": "application/json",
        }
    )
    ours.set_provider_response_headers(headers)
    reference.set_provider_response_headers(headers)
    reference_hidden: JsonValue = reference._hidden_params  # noqa: SLF001  # pyright: ignore[reportUnknownVariableType, reportUnknownMemberType, reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088-L2094
    assert ours._hidden_params == reference_hidden  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088-L2094
    assert ours._hidden_params["additional_headers"] == {  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088-L2094
        "x-ratelimit-limit-requests": "10",
        "llm_provider-x": "y",
        "llm_provider-x-ratelimit-limit-requests": "10",
        "llm_provider-content-type": "application/json",
    }
    first = betterlite_utils.ModelResponse(id="i", created=5)
    first._hidden_params["x"] = 1  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088-L2094
    second = betterlite_utils.ModelResponse(id="i", created=5)
    assert second._hidden_params == {}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088-L2094
    assert "_hidden_params" not in parse_object(first.model_dump_json())


def test_model_response_stream() -> None:
    ours = betterlite_utils.ModelResponseStream(id="i", created=5, model="m")
    assert ours.object == "chat.completion.chunk"
    assert ours.provider_specific_fields is None
    assert "provider_specific_fields" in ours
    assert_same("stream", litellm_utils.ModelResponseStream(id="i", created=5, model="m"), ours)
    empty = betterlite_utils.ModelResponseStream(id="i", created=5, choices=EMPTY_LIST)
    assert empty.choices == []


def test_model_response_choices() -> None:
    ours = betterlite_utils.ModelResponse(id="i", created=5, choices=CHOICES)
    assert ours.choices[0].finish_reason == "length"
    assert_same("response choices",
                litellm_utils.ModelResponse(id="i", created=5, choices=CHOICES), ours)
    assert len(betterlite_utils.ModelResponse(id="i", created=5).choices) == 1


def test_model_response_chunk() -> None:
    chunk: dict[str, JsonValue] = {
        "id": "i", "created": 5, "model": "m", "object": "chat.completion.chunk",
        "choices": [
            {
                "index": 0,
                "delta": {"role": "assistant", "content": None},
                "finish_reason": "tool_use",
                "logprobs": {"content": [{"token": "a", "logprob": -1.0, "top_logprobs": []}]},
            }
        ],
        "usage": {**USAGE_VALUES, "prompt_tokens_details": CACHED_DETAILS},
    }
    ours = betterlite_utils.StreamingChatCompletionChunk(**chunk)
    reference: Dumpable = litellm_utils.StreamingChatCompletionChunk(**chunk)
    assert ours.choices[0].finish_reason == "tool_calls"
    assert ours.model_dump() == reference.model_dump()
    assert parse_object(ours.model_dump_json()) == parse_object(reference.model_dump_json())


def test_model_response_openai() -> None:
    message = ChatCompletionMessage(role="assistant", content="x")
    choice = Choice(finish_reason="stop", index=0, message=message)
    chunk_choice = ChunkChoice(delta=ChoiceDelta(content="x"), index=0)
    usage = CompletionUsage(prompt_tokens=1, completion_tokens=2, total_tokens=3)
    choice_message = betterlite_utils.Choices(message=message)
    assert_same("choices", litellm_utils.Choices(message=message), choice_message)  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1690-L1693
    choices: list[JsonValue | pydantic.BaseModel] = [choice]
    response = betterlite_utils.ModelResponse(id="i", created=5, choices=choices)
    response_reference = litellm_utils.ModelResponse(id="i", created=5, choices=choices)
    assert_same("response", response_reference, response)
    chunk_choices: list[JsonValue | pydantic.BaseModel] = [chunk_choice]
    stream = betterlite_utils.ModelResponseStream(id="i", created=5, choices=chunk_choices)
    stream_reference = litellm_utils.ModelResponseStream(id="i", created=5, choices=chunk_choices)  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2107-L2113
    assert_same("stream", stream_reference, stream)
    with_usage = betterlite_utils.ModelResponse(id="i", created=5, usage=usage)
    assert_same("usage", litellm_utils.ModelResponse(id="i", created=5, usage=usage), with_usage)


def test_model_response_hidden() -> None:
    ours = betterlite_utils.ModelResponse(hidden_params=HIDDEN_PARAMS, _response_headers=HEADERS)
    assert ours._hidden_params == HIDDEN_PARAMS  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2237-L2245
    assert ours._response_headers == HEADERS  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2237-L2245


def test_dump_include_and_exclude() -> None:
    ours = betterlite_utils.ModelResponse(id="i", created=5, usage=USAGE_VALUES)
    reference = litellm_utils.ModelResponse(id="i", created=5, usage=USAGE_VALUES)
    assert_selected("exclude", reference, ours, exclude={"usage"})
    assert_selected("include", reference, ours, include={"id"})


def test_typed_dict_keys_match() -> None:
    ref, ours = litellm_completion, betterlite_completion
    pairs = (
        (ours.ChatCompletionSystemMessageParam, ref.ChatCompletionSystemMessageParam),
        (ours.ChatCompletionContentPartTextParam, ref.ChatCompletionContentPartTextParam),
        (ours.ImageURL, ref.ImageURL),
        (ours.ChatCompletionContentPartImageParam, ref.ChatCompletionContentPartImageParam),
        (ours.ChatCompletionUserMessageParam, ref.ChatCompletionUserMessageParam),
        (ours.FunctionCall, ref.FunctionCall),
        (ours.Function, ref.Function),
        (ours.ChatCompletionToolMessageParam, ref.ChatCompletionToolMessageParam),
        (ours.ChatCompletionFunctionMessageParam, ref.ChatCompletionFunctionMessageParam),
        (ours.ChatCompletionMessageToolCallParam, ref.ChatCompletionMessageToolCallParam),
        (ours.ChatCompletionAssistantMessageParam, ref.ChatCompletionAssistantMessageParam),
    )
    for pair in pairs:
        assert sorted(pair[0].__annotations__) == sorted(pair[1].__annotations__)


def test_typed_dict_unions() -> None:
    content: tuple[Member, Member] = (
        betterlite_completion.ChatCompletionContentPartParam,
        litellm_completion.ChatCompletionContentPartParam,
    )
    message: tuple[Member, Member] = (
        betterlite_completion.ChatCompletionMessageParam,
        litellm_completion.ChatCompletionMessageParam,
    )
    for ours, reference in (content, message):
        assert sorted(union_names(ours)) == sorted(union_names(reference))


def test_typed_dict_request() -> None:
    ours = betterlite_completion.CompletionRequest(model="gpt-4o")
    reference: Dumpable = litellm_completion.CompletionRequest(model="gpt-4o")
    assert_same("request", reference, ours)
    assert ours.stream is None
    assert betterlite_completion.CompletionRequest(model="m", extra="x")["extra"] == "x"


def test_base_root_model_dump() -> None:
    root: RootModel[int] = RootModel[int](BOX_VALUE)
    assert root.root == BOX_VALUE
    dumped: JsonValue = root.model_dump()
    assert dumped == BOX_VALUE
    listed: JsonValue = RootModel[list[int]]([1, 2]).model_dump()
    assert listed == [1, 2]
    assert root.model_dump_json() == str(BOX_VALUE)


def test_base_generic_subclass() -> None:
    box = Box[int](5)
    assert box.value == BOX_VALUE
    assert box["value"] == BOX_VALUE
    assert box.model_dump() == {"value": BOX_VALUE}
    assert box == Box[int](5)
    named = Named(id="i", name="n")
    assert named.name == "n"
    assert named.model_dump() == {"id": "i", "name": "n"}


def test_base_required_field_missing() -> None:  # ast-grep-ignore: no-long-names
    missing: dict[str, JsonValue] = {}
    with pytest.raises(ValidationError) as error:
        Payload.model_validate(missing)
    assert "data" in str(error.value)
    assert "seen" in str(error.value)
    assert ValidationError.__mro__[1] is ValueError


def test_base_field_validator_runs() -> None:
    assert Doubled(count=3).count == DOUBLED_COUNT
    with pytest.raises(ValidationError) as error:
        Doubled(count=-1)
    assert "count is negative" in str(error.value)


def test_base_model_validator_error() -> None:  # ast-grep-ignore: no-long-names
    assert Checked(count=3).count == READ_TOKENS
    with pytest.raises(ValidationError) as error:
        Checked(count=100)
    assert "count is too large" in str(error.value)


def test_base_base64str_roundtrip() -> None:
    values: dict[str, JsonValue] = {"data": "aGk=", "seen": "2024-01-02T03:04:05+00:00"}
    payload = Payload.model_validate(values)
    assert payload.data == "hi"
    assert payload["data"] == "hi"
    parsed = parse_object(payload.model_dump_json())
    assert parsed["data"] == "aGk="


def test_base_aware_datetime_rejects_naive() -> None:  # ast-grep-ignore: no-long-names
    values: dict[str, JsonValue] = {"data": "aGk=", "seen": "2024-01-02T03:04:05+00:00"}
    payload = Payload.model_validate(values)
    assert payload.seen.isoformat() == "2024-01-02T03:04:05+00:00"
    naive: dict[str, JsonValue] = {"data": "aGk=", "seen": "2024-01-02T03:04:05"}
    with pytest.raises(ValidationError):
        Payload.model_validate(naive)
    with pytest.raises(ValidationError):
        Payload(data="aGk=", seen=datetime.fromisoformat("2024-01-02T03:04:05"))
    aware = Payload(data="aGk=", seen=datetime(2024, 1, 2, 3, 4, 5, tzinfo=UTC))
    assert aware.seen.utcoffset() is not None


def test_unknown_field_names() -> None:
    message = betterlite_utils.Message(content="x")
    with pytest.raises(AttributeError):
        _ = message["unknown_attribute"]
