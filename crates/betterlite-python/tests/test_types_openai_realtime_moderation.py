import subprocess
import sys
from typing import Final, TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter

REALTIME_NAMES: Final = frozenset({
    "CreateVideoRequest",
    "ImageGenerationRequestQuality",
    "OpenAIBatchResponse",
    "OpenAIBatchResult",
    "OpenAIChatCompletionChoices",
    "OpenAIChatCompletionFinishReason",
    "OpenAIChatCompletionLogprobs",
    "OpenAIChatCompletionResponse",
    "OpenAIMcpServerTool",
    "OpenAIModerationResponse",
    "OpenAIModerationResult",
    "OpenAIRealtimeCachedTokensDetails",
    "OpenAIRealtimeContentPartDone",
    "OpenAIRealtimeConversationCreated",
    "OpenAIRealtimeConversationItemAdded",
    "OpenAIRealtimeConversationItemCreated",
    "OpenAIRealtimeConversationItemDone",
    "OpenAIRealtimeConversationObject",
    "OpenAIRealtimeDoneEvent",
    "OpenAIRealtimeErrorDetail",
    "OpenAIRealtimeErrorEvent",
    "OpenAIRealtimeEventTypes",
    "OpenAIRealtimeEvents",
    "OpenAIRealtimeFunctionCallArgumentsDone",
    "OpenAIRealtimeInputAudioBufferSpeechEvent",
    "OpenAIRealtimeInputAudioTranscriptionCompleted",
    "OpenAIRealtimeInputAudioTranscriptionDelta",
    "OpenAIRealtimeOutputItemDone",
    "OpenAIRealtimeResponseAudioDone",
    "OpenAIRealtimeResponseContentPart",
    "OpenAIRealtimeResponseContentPartAdded",
    "OpenAIRealtimeResponseDelta",
    "OpenAIRealtimeResponseDoneObject",
    "OpenAIRealtimeResponseTextDone",
    "OpenAIRealtimeResponseUsage",
    "OpenAIRealtimeServerVadTurnDetection",
    "OpenAIRealtimeStreamList",
    "OpenAIRealtimeStreamResponseBaseObject",
    "OpenAIRealtimeStreamResponseOutputItem",
    "OpenAIRealtimeStreamResponseOutputItemAdded",
    "OpenAIRealtimeStreamResponseOutputItemContent",
    "OpenAIRealtimeStreamSession",
    "OpenAIRealtimeStreamSessionEvents",
    "OpenAIRealtimeTranscriptionAudio",
    "OpenAIRealtimeTranscriptionAudioFormat",
    "OpenAIRealtimeTranscriptionAudioInput",
    "OpenAIRealtimeTranscriptionSession",
    "OpenAIRealtimeTranscriptionSessionCreated",
    "OpenAIRealtimeTranscriptionSettings",
    "OpenAIRealtimeTurnDetection",
    "OpenAIRealtimeUsageTokenDetails",
    "OpenAIVideoObject",
    "OpenAIWebSearchOptions",
    "OpenAIWebSearchUserLocation",
    "OpenAIWebSearchUserLocationApproximate",
    "REASONING_EFFORT",
})
EVENT_MEMBERS: Final = (
    "OpenAIRealtimeStreamResponseBaseObject",
    "OpenAIRealtimeStreamSessionEvents",
    "OpenAIRealtimeStreamResponseOutputItemAdded",
    "OpenAIRealtimeResponseContentPartAdded",
    "OpenAIRealtimeConversationItemCreated",
    "OpenAIRealtimeConversationItemAdded",
    "OpenAIRealtimeConversationItemDone",
    "OpenAIRealtimeConversationCreated",
    "OpenAIRealtimeResponseDelta",
    "OpenAIRealtimeResponseTextDone",
    "OpenAIRealtimeResponseAudioDone",
    "OpenAIRealtimeContentPartDone",
    "OpenAIRealtimeOutputItemDone",
    "OpenAIRealtimeFunctionCallArgumentsDone",
    "OpenAIRealtimeDoneEvent",
    "OpenAIRealtimeInputAudioBufferSpeechEvent",
    "OpenAIRealtimeInputAudioTranscriptionDelta",
    "OpenAIRealtimeInputAudioTranscriptionCompleted",
    "OpenAIRealtimeTranscriptionSessionCreated",
    "OpenAIRealtimeErrorEvent",
)
EVENT_TYPE_MEMBERS: Final = {
    "SESSION_CREATED": "session.created",
    "RESPONSE_TEXT_DELTA": "response.text.delta",
    "RESPONSE_AUDIO_DELTA": "response.audio.delta",
    "RESPONSE_TEXT_DONE": "response.text.done",
    "RESPONSE_AUDIO_DONE": "response.audio.done",
    "RESPONSE_OUTPUT_TEXT_DELTA": "response.output_text.delta",
    "RESPONSE_OUTPUT_AUDIO_DELTA": "response.output_audio.delta",
    "RESPONSE_OUTPUT_AUDIO_TRANSCRIPT_DELTA": "response.output_audio_transcript.delta",
    "RESPONSE_OUTPUT_TEXT_DONE": "response.output_text.done",
    "RESPONSE_OUTPUT_AUDIO_DONE": "response.output_audio.done",
    "RESPONSE_OUTPUT_AUDIO_TRANSCRIPT_DONE": "response.output_audio_transcript.done",
    "RESPONSE_DONE": "response.done",
    "RESPONSE_OUTPUT_ITEM_ADDED": "response.output_item.added",
    "RESPONSE_CONTENT_PART_ADDED": "response.content_part.added",
}
FINISH_REASONS: Final = (
    "stop",
    "content_filter",
    "function_call",
    "tool_calls",
    "length",
    "guardrail_intervened",
    "eos",
    "finish_reason_unspecified",
    "malformed_function_call",
)
REASONING_EFFORTS: Final = ("none", "minimal", "low", "medium", "high", "xhigh", "max")

DESCRIBE_SCRIPT = """
import json
import sys
from enum import Enum
from types import UnionType
from typing import Literal, Union, get_args, get_origin

from pydantic import BaseModel, TypeAdapter
from typing_extensions import is_typeddict

import betterlite.types.llms.openai.realtime as ours
import litellm.types.llms.openai as theirs

# these classes belong to the core and chat parts, which this part types as JSON mappings
MAPPED_REFS = {
    "ChatCompletionResponseMessage",
    "ChatCompletionUsageBlock",
    "OpenAIChatCompletionLogprobsContent",
    "OpenAIErrorBody",
}

EVENT_SAMPLES = [
    {
        "event_id": "e1",
        "type": "error",
        "error": {"type": "invalid_request_error", "message": "bad"},
    },
    {
        "event_id": "e2",
        "type": "session.created",
        "session": {"id": "sess_1", "model": "gpt-4o-realtime"},
    },
    {
        "type": "conversation.item.created",
        "previous_item_id": None,
        "item": {
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "hi"}],
        },
    },
]


def literal_values(annotation):
    if get_origin(annotation) is Literal:
        return [json.dumps(item, sort_keys=True) for item in get_args(annotation)]
    found = []
    if get_origin(annotation) is not None:
        for item in get_args(annotation):
            found.extend(literal_values(item))
    return found


def type_shape(node):
    if not isinstance(node, dict) or not node:
        return "json"
    if "$ref" in node:
        name = node["$ref"].rsplit("/", 1)[-1]
        return "json" if name in MAPPED_REFS or name == "JsonValue" else {"ref": name}
    if "anyOf" in node:
        return {"anyOf": [type_shape(item) for item in node["anyOf"]]}
    kind = node.get("type")
    if kind == "object":
        extra = node.get("additionalProperties")
        if extra is None or extra is True:
            return "json"
        inner = type_shape(extra)
        return "json" if inner == "json" else {"object": inner}
    if kind == "array":
        return {"array": type_shape(node.get("items", {}))}
    if kind is None:
        return "json"
    return kind


def property_shapes(schema, annotations):
    properties = schema.get("properties", {})
    return {
        "keys": sorted(properties),
        "required": sorted(schema.get("required", [])),
        "defaults": {
            name: json.dumps(field["default"], sort_keys=True)
            for name, field in properties.items()
            if "default" in field
        },
        "literals": {
            name: literal_values(annotations[name])
            for name in properties
            if name in annotations and literal_values(annotations[name])
        },
        "types": {name: type_shape(field) for name, field in properties.items()},
    }


def typeddict_shape(value):
    described = property_shapes(TypeAdapter(value).json_schema(), value.__annotations__)
    described["kind"] = "typeddict"
    return described


def model_shape(value):
    described = property_shapes(value.model_json_schema(), value.__annotations__)
    described["kind"] = "model"
    return described


def member_names(value):
    return [member.__name__ for member in get_args(value)]


def alias_shape(value):
    origin = get_origin(value)
    if origin is list:
        return {"kind": "list", "members": member_names(get_args(value)[0])}
    if origin is Union or origin is UnionType:
        return {"kind": "union", "members": member_names(value)}
    return {"kind": "literal", "values": literal_values(value)}


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


SHAPE_DEFAULTS = {
    "kind": "",
    "keys": [],
    "required": [],
    "defaults": {},
    "literals": {},
    "types": {},
    "members": [],
    "enum_members": [],
    "values": [],
}


def complete(shape):
    return {**SHAPE_DEFAULTS, **shape}


def describe(module, names):
    return {
        name: None if not hasattr(module, name) else complete(shape_of(getattr(module, name)))
        for name in names
    }


def moderation_dump(module):
    response = module.OpenAIModerationResponse(
        id="modr-1",
        model="omni-moderation-latest",
        request_id="req-1",
        results=[
            module.OpenAIModerationResult(
                categories={"hate": False, "violence": True},
                category_applied_input_types={"hate": ["text"], "violence": ["image"]},
                category_scores={"hate": 0.01, "violence": 0.9},
                flagged=True,
            )
        ],
    )
    return response.model_dump()


def video_dump(module):
    video = module.OpenAIVideoObject(
        id="video_1",
        object="video",
        status="completed",
        created_at=1713833628,
        completed_at=1713833629,
        expires_at=1713833628,
        error=None,
        progress=100,
        remixed_from_video_id="video_0",
        seconds="4",
        size="720x1280",
        model="sora-2",
    )
    return video.model_dump()


names = sys.argv[1:]
print(json.dumps({
    "litellm": describe(theirs, names),
    "betterlite": describe(ours, names),
    "dumps": {
        "litellm": {"moderation": moderation_dump(theirs), "video": video_dump(theirs)},
        "betterlite": {"moderation": moderation_dump(ours), "video": video_dump(ours)},
    },
    "events": {
        "litellm": TypeAdapter(theirs.OpenAIRealtimeStreamList).validate_python(EVENT_SAMPLES),
        "betterlite": TypeAdapter(ours.OpenAIRealtimeStreamList).validate_python(EVENT_SAMPLES),
    },
}))
"""


class Shape(TypedDict):
    kind: str
    keys: list[str]
    required: list[str]
    defaults: dict[str, str]
    literals: dict[str, list[str]]
    types: dict[str, JsonValue]
    members: list[str]
    enum_members: list[tuple[str, str]]
    values: list[str]


class Dumps(TypedDict):
    moderation: dict[str, JsonValue]
    video: dict[str, JsonValue]


class Events(TypedDict):
    litellm: list[dict[str, JsonValue]]
    betterlite: list[dict[str, JsonValue]]


class Report(TypedDict):
    litellm: dict[str, Shape | None]
    betterlite: dict[str, Shape | None]
    dumps: dict[str, Dumps]
    events: Events


@pytest.fixture(scope="module")
def report() -> Report:
    completed = subprocess.run(
        [sys.executable, "-c", DESCRIBE_SCRIPT, *sorted(REALTIME_NAMES)],
        check=True,
        capture_output=True,
        text=True,
    )
    parsed: Report = TypeAdapter(Report).validate_json(completed.stdout)
    return parsed


def test_realtime_names(report: Report) -> None:
    assert set(report["litellm"]) == REALTIME_NAMES
    assert set(report["betterlite"]) == REALTIME_NAMES
    assert None not in report["litellm"].values()
    assert None not in report["betterlite"].values()
    assert report["betterlite"] == report["litellm"]
    reasoning = report["betterlite"]["REASONING_EFFORT"]
    assert reasoning is not None
    assert [value.strip('"') for value in reasoning["values"]] == list(REASONING_EFFORTS)
    finish = report["betterlite"]["OpenAIChatCompletionFinishReason"]
    assert finish is not None
    assert [value.strip('"') for value in finish["values"]] == list(FINISH_REASONS)


def test_realtime_events(report: Report) -> None:
    litellm_enum = report["litellm"]["OpenAIRealtimeEventTypes"]
    betterlite_enum = report["betterlite"]["OpenAIRealtimeEventTypes"]
    assert betterlite_enum == litellm_enum
    assert betterlite_enum is not None
    assert dict(betterlite_enum["enum_members"]) == EVENT_TYPE_MEMBERS
    litellm_events = report["litellm"]["OpenAIRealtimeEvents"]
    betterlite_events = report["betterlite"]["OpenAIRealtimeEvents"]
    assert betterlite_events == litellm_events
    assert betterlite_events is not None
    assert betterlite_events["members"] == list(EVENT_MEMBERS)
    litellm_stream = report["litellm"]["OpenAIRealtimeStreamList"]
    betterlite_stream = report["betterlite"]["OpenAIRealtimeStreamList"]
    assert betterlite_stream == litellm_stream
    assert betterlite_stream is not None
    assert betterlite_stream["kind"] == "list"
    assert betterlite_stream["members"] == list(EVENT_MEMBERS)
    assert report["events"]["betterlite"] == report["events"]["litellm"]
    assert len(report["events"]["betterlite"]) == len(report["events"]["litellm"])


def test_realtime_models(report: Report) -> None:
    assert report["dumps"]["betterlite"] == report["dumps"]["litellm"]
    assert report["dumps"]["betterlite"]["moderation"]["id"] == "modr-1"
    assert report["dumps"]["betterlite"]["moderation"]["request_id"] == "req-1"
    assert report["dumps"]["betterlite"]["video"]["object"] == "video"
    assert "progress" in report["dumps"]["betterlite"]["video"]
