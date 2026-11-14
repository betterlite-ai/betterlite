import importlib
from enum import Enum
from typing import ClassVar, NamedTuple, Protocol, get_args, get_origin

import litellm.types.llms.gemini as ref_gemini
import litellm.types.llms.gemini_audio_transcription as ref_audio
import litellm.types.llms.vertex_ai_gemini_transcription as ref_transcription
import litellm.types.llms.vertex_ai_speech_to_text as ref_speech
import litellm.types.passthrough_endpoints.vertex_ai as ref_passthrough
import pydantic
import pytest
from pydantic import JsonValue, TypeAdapter
from pydantic.fields import FieldInfo
from typing_extensions import get_type_hints, is_typeddict

import betterlite.types.llms.gemini.gemini as ours_gemini
import betterlite.types.llms.gemini.gemini_audio_transcription as ours_audio
import betterlite.types.llms.vertex_ai_audio.vertex_ai_gemini_transcription as ours_transcription
import betterlite.types.llms.vertex_ai_audio.vertex_ai_speech_to_text as ours_speech
import betterlite.types.passthrough_endpoints.vertex_ai as ours_passthrough


class TypedDictClass(Protocol):
    __required_keys__: ClassVar[frozenset[str]]
    __optional_keys__: ClassVar[frozenset[str]]
    __readonly_keys__: ClassVar[frozenset[str]]
    __total__: ClassVar[bool]


class Blocks(Protocol):
    def __getattr__(self, name: str) -> TypedDictClass: ...


class ModelPair(NamedTuple):
    reference: type[pydantic.BaseModel]
    ours: type[pydantic.BaseModel]
    payload: dict[str, JsonValue]


ref_gemini_blocks: Blocks = importlib.import_module("litellm.types.llms.gemini")
ours_gemini_blocks: Blocks = importlib.import_module("betterlite.types.llms.gemini.gemini")
ref_audio_blocks: Blocks = importlib.import_module("litellm.types.llms.gemini_audio_transcription")
ours_audio_blocks: Blocks = importlib.import_module(
    "betterlite.types.llms.gemini.gemini_audio_transcription",
)
ref_transcription_blocks: Blocks = importlib.import_module(
    "litellm.types.llms.vertex_ai_gemini_transcription",
)
ours_transcription_blocks: Blocks = importlib.import_module(
    "betterlite.types.llms.vertex_ai_audio.vertex_ai_gemini_transcription",
)
ref_speech_blocks: Blocks = importlib.import_module("litellm.types.llms.vertex_ai_speech_to_text")
ours_speech_blocks: Blocks = importlib.import_module(
    "betterlite.types.llms.vertex_ai_audio.vertex_ai_speech_to_text",
)
ref_tts_blocks: Blocks = importlib.import_module("litellm.types.llms.vertex_ai_text_to_speech")
ours_tts_blocks: Blocks = importlib.import_module(
    "betterlite.types.llms.vertex_ai_audio.vertex_ai_text_to_speech",
)

TYPED_DICTS: dict[str, tuple[TypedDictClass, TypedDictClass]] = {
    "GeminiCreateFilesResponseObject": (
        ref_gemini_blocks.GeminiCreateFilesResponseObject,
        ours_gemini_blocks.GeminiCreateFilesResponseObject,
    ),
    "BidiGenerateContentTranscription": (
        ref_gemini_blocks.BidiGenerateContentTranscription,
        ours_gemini_blocks.BidiGenerateContentTranscription,
    ),
    "BidiGenerateContentServerContent": (
        ref_gemini_blocks.BidiGenerateContentServerContent,
        ours_gemini_blocks.BidiGenerateContentServerContent,
    ),
    "BidiGenerateContentServerMessage": (
        ref_gemini_blocks.BidiGenerateContentServerMessage,
        ours_gemini_blocks.BidiGenerateContentServerMessage,
    ),
    "BidiGenerateContentRealtimeInput": (
        ref_gemini_blocks.BidiGenerateContentRealtimeInput,
        ours_gemini_blocks.BidiGenerateContentRealtimeInput,
    ),
    "AutomaticActivityDetection": (
        ref_gemini_blocks.AutomaticActivityDetection,
        ours_gemini_blocks.AutomaticActivityDetection,
    ),
    "BidiGenerateContentRealtimeInputConfig": (
        ref_gemini_blocks.BidiGenerateContentRealtimeInputConfig,
        ours_gemini_blocks.BidiGenerateContentRealtimeInputConfig,
    ),
    "BidiGenerateContentSetup": (
        ref_gemini_blocks.BidiGenerateContentSetup,
        ours_gemini_blocks.BidiGenerateContentSetup,
    ),
    "GeminiImageGenerationInstance": (
        ref_gemini_blocks.GeminiImageGenerationInstance,
        ours_gemini_blocks.GeminiImageGenerationInstance,
    ),
    "GeminiGeneratedImage": (
        ref_gemini_blocks.GeminiGeneratedImage,
        ours_gemini_blocks.GeminiGeneratedImage,
    ),
    "GeminiImageGenerationPrediction": (
        ref_gemini_blocks.GeminiImageGenerationPrediction,
        ours_gemini_blocks.GeminiImageGenerationPrediction,
    ),
    "GeminiImageGenerationResponse": (
        ref_gemini_blocks.GeminiImageGenerationResponse,
        ours_gemini_blocks.GeminiImageGenerationResponse,
    ),
    "GeminiVideoGenerationInstance": (
        ref_gemini_blocks.GeminiVideoGenerationInstance,
        ours_gemini_blocks.GeminiVideoGenerationInstance,
    ),
    "GenerationConfig": (
        ref_gemini_blocks.GenerationConfig,
        ours_gemini_blocks.GenerationConfig,
    ),
    "HttpxBlobType": (
        ref_gemini_blocks.HttpxBlobType,
        ours_gemini_blocks.HttpxBlobType,
    ),
    "HttpxContentType": (
        ref_gemini_blocks.HttpxContentType,
        ours_gemini_blocks.HttpxContentType,
    ),
    "Tools": (ref_gemini_blocks.Tools, ours_gemini_blocks.Tools),
    "UsageMetadata": (
        ref_gemini_blocks.UsageMetadata,
        ours_gemini_blocks.UsageMetadata,
    ),
    "GeminiTranscriptionAudioInput": (
        ref_audio_blocks.GeminiTranscriptionAudioInput,
        ours_audio_blocks.GeminiTranscriptionAudioInput,
    ),
    "GeminiTranscriptionVerbatimMode": (
        ref_audio_blocks.GeminiTranscriptionVerbatimMode,
        ours_audio_blocks.GeminiTranscriptionVerbatimMode,
    ),
    "GeminiTranscriptionConfig": (
        ref_audio_blocks.GeminiTranscriptionConfig,
        ours_audio_blocks.GeminiTranscriptionConfig,
    ),
    "GeminiTranscriptionGenerationConfig": (
        ref_audio_blocks.GeminiTranscriptionGenerationConfig,
        ours_audio_blocks.GeminiTranscriptionGenerationConfig,
    ),
    "GeminiTranscriptionInteractionRequest": (
        ref_audio_blocks.GeminiTranscriptionInteractionRequest,
        ours_audio_blocks.GeminiTranscriptionInteractionRequest,
    ),
    "VertexGeminiTranscriptionInlineData": (
        ref_transcription_blocks.VertexGeminiTranscriptionInlineData,
        ours_transcription_blocks.VertexGeminiTranscriptionInlineData,
    ),
    "VertexGeminiTranscriptionPart": (
        ref_transcription_blocks.VertexGeminiTranscriptionPart,
        ours_transcription_blocks.VertexGeminiTranscriptionPart,
    ),
    "VertexGeminiTranscriptionContent": (
        ref_transcription_blocks.VertexGeminiTranscriptionContent,
        ours_transcription_blocks.VertexGeminiTranscriptionContent,
    ),
    "VertexGeminiTranscriptionAudioConfig": (
        ref_transcription_blocks.VertexGeminiTranscriptionAudioConfig,
        ours_transcription_blocks.VertexGeminiTranscriptionAudioConfig,
    ),
    "VertexGeminiTranscriptionGenerationConfig": (
        ref_transcription_blocks.VertexGeminiTranscriptionGenerationConfig,
        ours_transcription_blocks.VertexGeminiTranscriptionGenerationConfig,
    ),
    "VertexGeminiTranscriptionRequest": (
        ref_transcription_blocks.VertexGeminiTranscriptionRequest,
        ours_transcription_blocks.VertexGeminiTranscriptionRequest,
    ),
    "VertexSpeechToTextAutoDecodingConfig": (
        ref_speech_blocks.VertexSpeechToTextAutoDecodingConfig,
        ours_speech_blocks.VertexSpeechToTextAutoDecodingConfig,
    ),
    "VertexSpeechToTextRecognitionFeatures": (
        ref_speech_blocks.VertexSpeechToTextRecognitionFeatures,
        ours_speech_blocks.VertexSpeechToTextRecognitionFeatures,
    ),
    "VertexSpeechToTextRecognitionConfig": (
        ref_speech_blocks.VertexSpeechToTextRecognitionConfig,
        ours_speech_blocks.VertexSpeechToTextRecognitionConfig,
    ),
    "VertexSpeechToTextRecognizeRequest": (
        ref_speech_blocks.VertexSpeechToTextRecognizeRequest,
        ours_speech_blocks.VertexSpeechToTextRecognizeRequest,
    ),
    "VertexTextToSpeechInput": (
        ref_tts_blocks.VertexTextToSpeechInput,
        ours_tts_blocks.VertexTextToSpeechInput,
    ),
    "VertexTextToSpeechVoice": (
        ref_tts_blocks.VertexTextToSpeechVoice,
        ours_tts_blocks.VertexTextToSpeechVoice,
    ),
    "VertexTextToSpeechAudioConfig": (
        ref_tts_blocks.VertexTextToSpeechAudioConfig,
        ours_tts_blocks.VertexTextToSpeechAudioConfig,
    ),
    "VertexTextToSpeechRequest": (
        ref_tts_blocks.VertexTextToSpeechRequest,
        ours_tts_blocks.VertexTextToSpeechRequest,
    ),
}

VIDEO_URI: dict[str, JsonValue] = {"uri": "gs://bucket/video.mp4"}
VIDEO_SAMPLE: dict[str, JsonValue] = {"video": VIDEO_URI}
VIDEO_RESPONSE: dict[str, JsonValue] = {"generatedSamples": [VIDEO_SAMPLE]}
OPERATION_RESPONSE: dict[str, JsonValue] = {"generateVideoResponse": VIDEO_RESPONSE}
OPERATION_METADATA: dict[str, JsonValue] = {"createTime": "2026-01-01T00:00:00Z", "model": "veo-3"}
IMAGE_PARAMETERS: dict[str, JsonValue] = {
    "sampleCount": 2,
    "aspectRatio": "16:9",
    "imageSize": "2K",
    "personGeneration": "allow_adult",
    "quality": "high",
}
VIDEO_PARAMETERS: dict[str, JsonValue] = {
    "aspectRatio": "16:9",
    "durationSeconds": 8,
    "resolution": "720p",
    "negativePrompt": "blur",
    "lastFrame": {"bytesBase64Encoded": "AA=="},
    "referenceImages": [{"image": "x"}],
    "video": VIDEO_URI,
    "personGeneration": "allow_all",
}
WORD_ANNOTATION: dict[str, JsonValue] = {
    "type": "word",
    "text": "hi",
    "speaker": "s1",
    "start_offset": "0s",
    "end_offset": "1s",
}
AUDIO_CONTENT: dict[str, JsonValue] = {
    "type": "text",
    "text": "hi",
    "annotations": [WORD_ANNOTATION],
}
AUDIO_STEP: dict[str, JsonValue] = {"type": "model_output", "content": [AUDIO_CONTENT]}
AUDIO_TOKENS: dict[str, JsonValue] = {"modality": "AUDIO", "tokens": 10}
AUDIO_USAGE: dict[str, JsonValue] = {
    "total_tokens": 12,
    "total_input_tokens": 10,
    "total_output_tokens": 2,
    "input_tokens_by_modality": [AUDIO_TOKENS],
}
RESPONSE_PART: dict[str, JsonValue] = {"text": "hi"}
RESPONSE_CONTENT: dict[str, JsonValue] = {"parts": [RESPONSE_PART]}
CANDIDATE: dict[str, JsonValue] = {"content": RESPONSE_CONTENT}
VERTEX_TOKENS: dict[str, JsonValue] = {"modality": "AUDIO", "tokenCount": 5}
USAGE_METADATA: dict[str, JsonValue] = {
    "promptTokenCount": 5,
    "candidatesTokenCount": 2,
    "totalTokenCount": 7,
    "promptTokensDetails": [VERTEX_TOKENS],
}
ALTERNATIVE: dict[str, JsonValue] = {"transcript": "hi"}
RESULT: dict[str, JsonValue] = {"alternatives": [ALTERNATIVE], "languageCode": "en-US"}
RESPONSE_METADATA: dict[str, JsonValue] = {"totalBilledDuration": "1s"}
STREAMING_RESULT: dict[str, JsonValue] = {"transcript": "hi", "is_final": True}

MODELS: dict[str, ModelPair] = {
    "GeminiImageGenerationParameters": ModelPair(
        ref_gemini.GeminiImageGenerationParameters,
        ours_gemini.GeminiImageGenerationParameters,
        IMAGE_PARAMETERS,
    ),
    "GeminiImageGenerationRequest": ModelPair(
        ref_gemini.GeminiImageGenerationRequest,
        ours_gemini.GeminiImageGenerationRequest,
        {"instances": [{"prompt": "a cat"}], "parameters": IMAGE_PARAMETERS},
    ),
    "GeminiVideoGenerationParameters": ModelPair(
        ref_gemini.GeminiVideoGenerationParameters,
        ours_gemini.GeminiVideoGenerationParameters,
        VIDEO_PARAMETERS,
    ),
    "GeminiVideoGenerationRequest": ModelPair(
        ref_gemini.GeminiVideoGenerationRequest,
        ours_gemini.GeminiVideoGenerationRequest,
        {"instances": [{"prompt": "a dog"}], "parameters": VIDEO_PARAMETERS},
    ),
    "GeminiVideoUri": ModelPair(
        ref_gemini.GeminiVideoUri,
        ours_gemini.GeminiVideoUri,
        VIDEO_URI,
    ),
    "GeminiGeneratedVideoSample": ModelPair(
        ref_gemini.GeminiGeneratedVideoSample,
        ours_gemini.GeminiGeneratedVideoSample,
        VIDEO_SAMPLE,
    ),
    "GeminiGenerateVideoResponse": ModelPair(
        ref_gemini.GeminiGenerateVideoResponse,
        ours_gemini.GeminiGenerateVideoResponse,
        VIDEO_RESPONSE,
    ),
    "GeminiOperationResponse": ModelPair(
        ref_gemini.GeminiOperationResponse,
        ours_gemini.GeminiOperationResponse,
        OPERATION_RESPONSE,
    ),
    "GeminiOperationMetadata": ModelPair(
        ref_gemini.GeminiOperationMetadata,
        ours_gemini.GeminiOperationMetadata,
        OPERATION_METADATA,
    ),
    "GeminiLongRunningOperationResponse": ModelPair(
        ref_gemini.GeminiLongRunningOperationResponse,
        ours_gemini.GeminiLongRunningOperationResponse,
        {
            "name": "operations/1",
            "done": True,
            "metadata": OPERATION_METADATA,
            "response": OPERATION_RESPONSE,
            "error": {"code": 3},
        },
    ),
    "GeminiTranscriptionWordAnnotation": ModelPair(
        ref_audio.GeminiTranscriptionWordAnnotation,
        ours_audio.GeminiTranscriptionWordAnnotation,
        WORD_ANNOTATION,
    ),
    "GeminiTranscriptionContent": ModelPair(
        ref_audio.GeminiTranscriptionContent,
        ours_audio.GeminiTranscriptionContent,
        AUDIO_CONTENT,
    ),
    "GeminiTranscriptionStep": ModelPair(
        ref_audio.GeminiTranscriptionStep,
        ours_audio.GeminiTranscriptionStep,
        AUDIO_STEP,
    ),
    "GeminiTranscriptionModalityTokens": ModelPair(
        ref_audio.GeminiTranscriptionModalityTokens,
        ours_audio.GeminiTranscriptionModalityTokens,
        AUDIO_TOKENS,
    ),
    "GeminiTranscriptionUsage": ModelPair(
        ref_audio.GeminiTranscriptionUsage,
        ours_audio.GeminiTranscriptionUsage,
        AUDIO_USAGE,
    ),
    "GeminiTranscriptionInteractionResponse": ModelPair(
        ref_audio.GeminiTranscriptionInteractionResponse,
        ours_audio.GeminiTranscriptionInteractionResponse,
        {"id": "i1", "status": "completed", "usage": AUDIO_USAGE, "steps": [AUDIO_STEP]},
    ),
    "VertexGeminiTranscriptionResponsePart": ModelPair(
        ref_transcription.VertexGeminiTranscriptionResponsePart,
        ours_transcription.VertexGeminiTranscriptionResponsePart,
        RESPONSE_PART,
    ),
    "VertexGeminiTranscriptionResponseContent": ModelPair(
        ref_transcription.VertexGeminiTranscriptionResponseContent,
        ours_transcription.VertexGeminiTranscriptionResponseContent,
        RESPONSE_CONTENT,
    ),
    "VertexGeminiTranscriptionCandidate": ModelPair(
        ref_transcription.VertexGeminiTranscriptionCandidate,
        ours_transcription.VertexGeminiTranscriptionCandidate,
        CANDIDATE,
    ),
    "VertexGeminiTranscriptionModalityTokens": ModelPair(
        ref_transcription.VertexGeminiTranscriptionModalityTokens,
        ours_transcription.VertexGeminiTranscriptionModalityTokens,
        VERTEX_TOKENS,
    ),
    "VertexGeminiTranscriptionUsageMetadata": ModelPair(
        ref_transcription.VertexGeminiTranscriptionUsageMetadata,
        ours_transcription.VertexGeminiTranscriptionUsageMetadata,
        USAGE_METADATA,
    ),
    "VertexGeminiTranscriptionResponse": ModelPair(
        ref_transcription.VertexGeminiTranscriptionResponse,
        ours_transcription.VertexGeminiTranscriptionResponse,
        {"candidates": [CANDIDATE], "usageMetadata": USAGE_METADATA},
    ),
    "VertexSpeechToTextAlternative": ModelPair(
        ref_speech.VertexSpeechToTextAlternative,
        ours_speech.VertexSpeechToTextAlternative,
        ALTERNATIVE,
    ),
    "VertexSpeechToTextResult": ModelPair(
        ref_speech.VertexSpeechToTextResult,
        ours_speech.VertexSpeechToTextResult,
        RESULT,
    ),
    "VertexSpeechToTextResponseMetadata": ModelPair(
        ref_speech.VertexSpeechToTextResponseMetadata,
        ours_speech.VertexSpeechToTextResponseMetadata,
        RESPONSE_METADATA,
    ),
    "VertexSpeechToTextRecognizeResponse": ModelPair(
        ref_speech.VertexSpeechToTextRecognizeResponse,
        ours_speech.VertexSpeechToTextRecognizeResponse,
        {"results": [RESULT], "metadata": RESPONSE_METADATA},
    ),
    "VertexSpeechStreamingConfigure": ModelPair(
        ref_speech.VertexSpeechStreamingConfigure,
        ours_speech.VertexSpeechStreamingConfigure,
        {
            "kind": "configure",
            "model": "chirp_2",
            "language_codes": ["en-US"],
            "sample_rate_hertz": 16000,
        },
    ),
    "VertexSpeechStreamingFinishTurn": ModelPair(
        ref_speech.VertexSpeechStreamingFinishTurn,
        ours_speech.VertexSpeechStreamingFinishTurn,
        {"kind": "finish_turn"},
    ),
    "VertexSpeechStreamingDiscardTurn": ModelPair(
        ref_speech.VertexSpeechStreamingDiscardTurn,
        ours_speech.VertexSpeechStreamingDiscardTurn,
        {"kind": "discard_turn"},
    ),
    "VertexSpeechStreamingResult": ModelPair(
        ref_speech.VertexSpeechStreamingResult,
        ours_speech.VertexSpeechStreamingResult,
        STREAMING_RESULT,
    ),
    "VertexSpeechStreamingResponse": ModelPair(
        ref_speech.VertexSpeechStreamingResponse,
        ours_speech.VertexSpeechStreamingResponse,
        {
            "kind": "response",
            "speech_event": "begin",
            "results": [STREAMING_RESULT],
            "billed_seconds": 1.5,
        },
    ),
    "VertexSpeechStreamingConfigured": ModelPair(
        ref_speech.VertexSpeechStreamingConfigured,
        ours_speech.VertexSpeechStreamingConfigured,
        {"kind": "configured"},
    ),
    "VertexSpeechStreamingTurnFinished": ModelPair(
        ref_speech.VertexSpeechStreamingTurnFinished,
        ours_speech.VertexSpeechStreamingTurnFinished,
        {"kind": "turn_finished"},
    ),
    "VertexSpeechStreamingTurnDiscarded": ModelPair(
        ref_speech.VertexSpeechStreamingTurnDiscarded,
        ours_speech.VertexSpeechStreamingTurnDiscarded,
        {"kind": "turn_discarded", "billed_seconds": 0.5},
    ),
    "VertexPassThroughCredentials": ModelPair(
        ref_passthrough.VertexPassThroughCredentials,
        ours_passthrough.VertexPassThroughCredentials,
        {
            "vertex_project": "project",
            "vertex_location": "us-central1",
            "vertex_credentials": {"type": "service_account"},
        },
    ),
    "VertexPassThroughCredentialsPath": ModelPair(
        ref_passthrough.VertexPassThroughCredentials,
        ours_passthrough.VertexPassThroughCredentials,
        {"vertex_credentials": "os.environ/GOOGLE_CREDS"},
    ),
}


def shape(annotation: type) -> str:
    origin: type | None = get_origin(annotation)
    arguments: tuple[type, ...] = get_args(annotation)
    if is_typeddict(annotation):
        return f"TypedDict:{annotation.__name__}"
    if origin is None:
        name = annotation.__name__
        if name in ("Any", "JsonValue"):
            return "json"
        return name
    name = origin.__name__
    if name == "Literal":
        return f"Literal{sorted(str(argument) for argument in arguments)}"
    if name in ("Union", "UnionType"):
        members = sorted(shape(argument) for argument in arguments)
        if "json" in members:
            return "json"
        return "|".join(members)
    if name == "dict" and shape(arguments[1]) == "json":
        return "dict"
    if name == "list" and shape(arguments[0]) == "json":
        return "list"
    if name == "tuple":
        return f"tuple[{shape(arguments[0])},...]"
    return name + "[" + ",".join(shape(argument) for argument in arguments) + "]"


def enum_members(enum: type[Enum]) -> list[tuple[str, JsonValue]]:
    members: list[tuple[str, JsonValue]] = []
    for member in enum:
        value: JsonValue = member.value
        members.append((member.name, value))
    return members


def model_hints(model: type[pydantic.BaseModel]) -> dict[str, type]:
    hints: dict[str, type] = {}
    for name, field in model.model_fields.items():
        annotation = field.annotation
        assert annotation is not None
        hints[name] = annotation
    return hints


def property_literal(schema: JsonValue) -> JsonValue:
    match schema:
        case dict():
            if "const" in schema:
                return schema["const"]
            return schema.get("enum")
        case list() | bool() | int() | float() | str() | None:
            return None


def schema_literals(model: type[pydantic.BaseModel]) -> dict[str, JsonValue]:
    schema: dict[str, JsonValue] = model.model_json_schema()
    properties = schema["properties"]
    match properties:
        case dict():
            return {name: property_literal(value) for name, value in properties.items()}
        case list() | bool() | int() | float() | str() | None:
            raise AssertionError("the schema has no properties")


def constructed_dump(model: type[pydantic.BaseModel]) -> dict[str, JsonValue]:
    dumped: dict[str, JsonValue] = model.model_construct().model_dump()
    return dumped


@pytest.mark.parametrize("name", TYPED_DICTS)
def test_names_typed_dict_matches(name: str) -> None:
    reference, ours = TYPED_DICTS[name]
    reference_hints: dict[str, type] = get_type_hints(reference)
    ours_hints: dict[str, type] = get_type_hints(ours)

    assert ours.__required_keys__ == reference.__required_keys__
    assert ours.__optional_keys__ == reference.__optional_keys__
    assert ours.__readonly_keys__ == reference.__readonly_keys__
    assert ours.__total__ == reference.__total__
    assert set(ours_hints) == set(reference_hints)
    for key, hint in reference_hints.items():
        assert shape(ours_hints[key]) == shape(hint), key


@pytest.mark.parametrize("name", MODELS)
def test_names_model_matches(name: str) -> None:
    reference, ours, payload = MODELS[name]
    reference_hints = model_hints(reference)
    ours_hints = model_hints(ours)

    assert set(ours_hints) == set(reference_hints)
    for field, hint in reference_hints.items():
        assert shape(ours_hints[field]) == shape(hint), field
    reference_required = {n for n, f in reference.model_fields.items() if f.is_required()}
    ours_required = {n for n, f in ours.model_fields.items() if f.is_required()}
    assert ours_required == reference_required
    assert constructed_dump(ours) == constructed_dump(reference)
    assert schema_literals(ours) == schema_literals(reference)
    assert ours.model_config.get("frozen") == reference.model_config.get("frozen")

    sent: dict[str, JsonValue] = {**payload, "unexpected": 1}
    reference_dump: dict[str, JsonValue] = reference.model_validate(sent).model_dump()
    ours_dump: dict[str, JsonValue] = ours.model_validate(sent).model_dump()
    assert ours_dump == reference_dump
    assert set(payload) <= set(ours_dump)
    assert "unexpected" not in ours_dump


def test_names_gemini_literals() -> None:
    assert enum_members(ours_gemini.GeminiFilesState) == enum_members(ref_gemini.GeminiFilesState)
    assert enum_members(ours_gemini.GeminiFilesSource) == enum_members(
        ref_gemini.GeminiFilesSource,
    )
    reference_start: tuple[str, ...] = get_args(ref_gemini.StartOfSpeechSensitivityEnum)
    ours_start: tuple[str, ...] = get_args(ours_gemini.StartOfSpeechSensitivityEnum)
    assert ours_start == reference_start
    reference_end: tuple[str, ...] = get_args(ref_gemini.EndOfSpeechSensitivityEnum)
    ours_end: tuple[str, ...] = get_args(ours_gemini.EndOfSpeechSensitivityEnum)
    assert ours_end == reference_end


def test_names_command_union() -> None:
    reference_members: tuple[type, ...] = get_args(ref_speech.VertexSpeechStreamingCommandUnion)
    ours_members: tuple[type, ...] = get_args(ours_speech.VertexSpeechStreamingCommandUnion)
    names = [
        "VertexSpeechStreamingConfigure",
        "VertexSpeechStreamingFinishTurn",
        "VertexSpeechStreamingDiscardTurn",
    ]
    assert [member.__name__ for member in reference_members] == names
    assert [member.__name__ for member in ours_members] == names
    reference_metadata: tuple[FieldInfo, ...] = get_args(
        ref_speech.VertexSpeechStreamingCommand,
    )[1:]
    ours_metadata: tuple[FieldInfo, ...] = get_args(
        ours_speech.VertexSpeechStreamingCommand,
    )[1:]
    assert [info.discriminator for info in reference_metadata] == ["kind"]
    assert [info.discriminator for info in ours_metadata] == ["kind"]

    reference_adapter = TypeAdapter(ref_speech.VertexSpeechStreamingCommand)
    ours_adapter = TypeAdapter(ours_speech.VertexSpeechStreamingCommand)
    for name in names:
        payload = MODELS[name].payload
        reference_value = reference_adapter.validate_python(payload)
        ours_value = ours_adapter.validate_python(payload)
        assert reference_value.__class__.__name__ == name
        assert ours_value.__class__.__name__ == name
        assert ours_value.model_dump() == reference_value.model_dump()


def test_names_event_union() -> None:
    reference_members: tuple[type, ...] = get_args(ref_speech.VertexSpeechStreamingEventUnion)
    ours_members: tuple[type, ...] = get_args(ours_speech.VertexSpeechStreamingEventUnion)
    names = [
        "VertexSpeechStreamingResponse",
        "VertexSpeechStreamingConfigured",
        "VertexSpeechStreamingTurnFinished",
        "VertexSpeechStreamingTurnDiscarded",
    ]
    assert [member.__name__ for member in reference_members] == names
    assert [member.__name__ for member in ours_members] == names
    reference_metadata: tuple[FieldInfo, ...] = get_args(
        ref_speech.VertexSpeechStreamingEvent,
    )[1:]
    ours_metadata: tuple[FieldInfo, ...] = get_args(
        ours_speech.VertexSpeechStreamingEvent,
    )[1:]
    assert [info.discriminator for info in reference_metadata] == ["kind"]
    assert [info.discriminator for info in ours_metadata] == ["kind"]

    reference_adapter = TypeAdapter(ref_speech.VertexSpeechStreamingEvent)
    ours_adapter = TypeAdapter(ours_speech.VertexSpeechStreamingEvent)
    for name in names:
        payload = MODELS[name].payload
        reference_value = reference_adapter.validate_python(payload)
        ours_value = ours_adapter.validate_python(payload)
        assert reference_value.__class__.__name__ == name
        assert ours_value.__class__.__name__ == name
        assert ours_value.model_dump() == reference_value.model_dump()


def test_names_credentials_type() -> None:
    assert ours_passthrough.VERTEX_CREDENTIALS_TYPES == ref_passthrough.VERTEX_CREDENTIALS_TYPES
