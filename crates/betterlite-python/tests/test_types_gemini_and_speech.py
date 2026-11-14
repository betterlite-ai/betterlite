from enum import Enum
from typing import get_args

import litellm.types.llms.gemini as ref_gemini
import litellm.types.llms.gemini_audio_transcription as ref_audio
import litellm.types.llms.vertex_ai_gemini_transcription as ref_transcription
import litellm.types.llms.vertex_ai_speech_to_text as ref_speech
import litellm.types.llms.vertex_ai_text_to_speech as ref_tts
import litellm.types.passthrough_endpoints.vertex_ai as ref_passthrough
import pydantic

import betterlite.types.llms.gemini.gemini as ours_gemini
import betterlite.types.llms.gemini.gemini_audio_transcription as ours_audio
import betterlite.types.llms.vertex_ai_audio.vertex_ai_gemini_transcription as ours_transcription
import betterlite.types.llms.vertex_ai_audio.vertex_ai_speech_to_text as ours_speech
import betterlite.types.llms.vertex_ai_audio.vertex_ai_text_to_speech as ours_tts
import betterlite.types.passthrough_endpoints.vertex_ai as ours_passthrough
from betterlite.types.utils import JsonValue


def same_typed_dict(
    reference: dict[str, JsonValue],
    ours: dict[str, JsonValue],
    reference_required: frozenset[str],
    ours_required: frozenset[str],
) -> None:
    assert set(reference) == set(ours)
    assert reference_required == ours_required


def enum_members(enum: type[Enum]) -> list[tuple[str, JsonValue]]:
    members: list[tuple[str, JsonValue]] = []
    for member in enum:
        value: JsonValue = member.value
        members.append((member.name, value))
    return members


def annotation_words(annotation: JsonValue) -> list[str]:
    text = str(annotation).replace("typing_extensions.", "").replace("typing.", "")
    words = text.replace("[", " ").replace("]", " ").replace(",", " ").replace("'", " ")
    return sorted(word for word in words.split() if word)


def same_literals(
    reference: dict[str, JsonValue],
    ours: dict[str, JsonValue],
) -> None:
    reference_words = {
        name: annotation_words(annotation)
        for name, annotation in reference.items()
        if "Literal" in str(annotation)
    }
    ours_words = {
        name: annotation_words(annotation)
        for name, annotation in ours.items()
        if "Literal" in str(annotation)
    }
    assert reference_words == ours_words


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


def same_model(reference: type[pydantic.BaseModel], ours: type[pydantic.BaseModel]) -> None:
    assert set(reference.model_fields) == set(ours.model_fields)
    reference_required = sorted(
        name for name, field in reference.model_fields.items() if field.is_required()
    )
    ours_required = sorted(
        name for name, field in ours.model_fields.items() if field.is_required()
    )
    assert reference_required == ours_required
    assert constructed_dump(reference) == constructed_dump(ours)
    assert schema_literals(reference) == schema_literals(ours)
    assert reference.model_config.get("frozen") == ours.model_config.get("frozen")


def test_gemini_names() -> None:
    ref = ref_gemini
    ours = ours_gemini
    assert enum_members(ref.GeminiFilesState) == enum_members(ours.GeminiFilesState)
    assert enum_members(ref.GeminiFilesSource) == enum_members(ours.GeminiFilesSource)
    reference_start: tuple[JsonValue, ...] = get_args(ref.StartOfSpeechSensitivityEnum)
    ours_start: tuple[JsonValue, ...] = get_args(ours.StartOfSpeechSensitivityEnum)
    assert reference_start == ours_start
    reference_end: tuple[JsonValue, ...] = get_args(ref.EndOfSpeechSensitivityEnum)
    ours_end: tuple[JsonValue, ...] = get_args(ours.EndOfSpeechSensitivityEnum)
    assert reference_end == ours_end
    same_typed_dict(
        ref.GeminiCreateFilesResponseObject.__annotations__,
        ours.GeminiCreateFilesResponseObject.__annotations__,
        ref.GeminiCreateFilesResponseObject.__required_keys__,
        ours.GeminiCreateFilesResponseObject.__required_keys__,
    )
    same_typed_dict(
        ref.BidiGenerateContentTranscription.__annotations__,
        ours.BidiGenerateContentTranscription.__annotations__,
        ref.BidiGenerateContentTranscription.__required_keys__,
        ours.BidiGenerateContentTranscription.__required_keys__,
    )
    same_typed_dict(
        ref.BidiGenerateContentServerContent.__annotations__,
        ours.BidiGenerateContentServerContent.__annotations__,
        ref.BidiGenerateContentServerContent.__required_keys__,
        ours.BidiGenerateContentServerContent.__required_keys__,
    )
    same_typed_dict(
        ref.BidiGenerateContentServerMessage.__annotations__,
        ours.BidiGenerateContentServerMessage.__annotations__,
        ref.BidiGenerateContentServerMessage.__required_keys__,
        ours.BidiGenerateContentServerMessage.__required_keys__,
    )
    same_typed_dict(
        ref.BidiGenerateContentRealtimeInput.__annotations__,
        ours.BidiGenerateContentRealtimeInput.__annotations__,
        ref.BidiGenerateContentRealtimeInput.__required_keys__,
        ours.BidiGenerateContentRealtimeInput.__required_keys__,
    )
    same_typed_dict(
        ref.AutomaticActivityDetection.__annotations__,
        ours.AutomaticActivityDetection.__annotations__,
        ref.AutomaticActivityDetection.__required_keys__,
        ours.AutomaticActivityDetection.__required_keys__,
    )
    same_literals(
        ref.AutomaticActivityDetection.__annotations__,
        ours.AutomaticActivityDetection.__annotations__,
    )
    same_typed_dict(
        ref.BidiGenerateContentRealtimeInputConfig.__annotations__,
        ours.BidiGenerateContentRealtimeInputConfig.__annotations__,
        ref.BidiGenerateContentRealtimeInputConfig.__required_keys__,
        ours.BidiGenerateContentRealtimeInputConfig.__required_keys__,
    )
    same_typed_dict(
        ref.BidiGenerateContentSetup.__annotations__,
        ours.BidiGenerateContentSetup.__annotations__,
        ref.BidiGenerateContentSetup.__required_keys__,
        ours.BidiGenerateContentSetup.__required_keys__,
    )
    same_typed_dict(
        ref.GeminiImageGenerationInstance.__annotations__,
        ours.GeminiImageGenerationInstance.__annotations__,
        ref.GeminiImageGenerationInstance.__required_keys__,
        ours.GeminiImageGenerationInstance.__required_keys__,
    )
    same_typed_dict(
        ref.GeminiGeneratedImage.__annotations__,
        ours.GeminiGeneratedImage.__annotations__,
        ref.GeminiGeneratedImage.__required_keys__,
        ours.GeminiGeneratedImage.__required_keys__,
    )
    same_typed_dict(
        ref.GeminiImageGenerationPrediction.__annotations__,
        ours.GeminiImageGenerationPrediction.__annotations__,
        ref.GeminiImageGenerationPrediction.__required_keys__,
        ours.GeminiImageGenerationPrediction.__required_keys__,
    )
    same_typed_dict(
        ref.GeminiImageGenerationResponse.__annotations__,
        ours.GeminiImageGenerationResponse.__annotations__,
        ref.GeminiImageGenerationResponse.__required_keys__,
        ours.GeminiImageGenerationResponse.__required_keys__,
    )
    same_typed_dict(
        ref.GeminiVideoGenerationInstance.__annotations__,
        ours.GeminiVideoGenerationInstance.__annotations__,
        ref.GeminiVideoGenerationInstance.__required_keys__,
        ours.GeminiVideoGenerationInstance.__required_keys__,
    )
    same_typed_dict(
        ref.GenerationConfig.__annotations__,
        ours.GenerationConfig.__annotations__,
        ref.GenerationConfig.__required_keys__,
        ours.GenerationConfig.__required_keys__,
    )
    same_typed_dict(
        ref.HttpxBlobType.__annotations__,
        ours.HttpxBlobType.__annotations__,
        ref.HttpxBlobType.__required_keys__,
        ours.HttpxBlobType.__required_keys__,
    )
    same_typed_dict(
        ref.HttpxContentType.__annotations__,
        ours.HttpxContentType.__annotations__,
        ref.HttpxContentType.__required_keys__,
        ours.HttpxContentType.__required_keys__,
    )
    same_typed_dict(
        ref.Tools.__annotations__,
        ours.Tools.__annotations__,
        ref.Tools.__required_keys__,
        ours.Tools.__required_keys__,
    )
    same_typed_dict(
        ref.UsageMetadata.__annotations__,
        ours.UsageMetadata.__annotations__,
        ref.UsageMetadata.__required_keys__,
        ours.UsageMetadata.__required_keys__,
    )
    same_model(ref.GeminiImageGenerationParameters, ours.GeminiImageGenerationParameters)
    same_model(ref.GeminiImageGenerationRequest, ours.GeminiImageGenerationRequest)
    same_model(ref.GeminiVideoGenerationParameters, ours.GeminiVideoGenerationParameters)
    same_model(ref.GeminiVideoGenerationRequest, ours.GeminiVideoGenerationRequest)
    same_model(ref.GeminiVideoUri, ours.GeminiVideoUri)
    same_model(ref.GeminiGeneratedVideoSample, ours.GeminiGeneratedVideoSample)
    same_model(ref.GeminiGenerateVideoResponse, ours.GeminiGenerateVideoResponse)
    same_model(ref.GeminiOperationResponse, ours.GeminiOperationResponse)
    same_model(ref.GeminiOperationMetadata, ours.GeminiOperationMetadata)
    same_model(ref.GeminiLongRunningOperationResponse, ours.GeminiLongRunningOperationResponse)


def test_transcription_names() -> None:
    same_typed_dict(
        ref_audio.GeminiTranscriptionAudioInput.__annotations__,
        ours_audio.GeminiTranscriptionAudioInput.__annotations__,
        ref_audio.GeminiTranscriptionAudioInput.__required_keys__,
        ours_audio.GeminiTranscriptionAudioInput.__required_keys__,
    )
    same_typed_dict(
        ref_audio.GeminiTranscriptionVerbatimMode.__annotations__,
        ours_audio.GeminiTranscriptionVerbatimMode.__annotations__,
        ref_audio.GeminiTranscriptionVerbatimMode.__required_keys__,
        ours_audio.GeminiTranscriptionVerbatimMode.__required_keys__,
    )
    same_typed_dict(
        ref_audio.GeminiTranscriptionConfig.__annotations__,
        ours_audio.GeminiTranscriptionConfig.__annotations__,
        ref_audio.GeminiTranscriptionConfig.__required_keys__,
        ours_audio.GeminiTranscriptionConfig.__required_keys__,
    )
    same_typed_dict(
        ref_audio.GeminiTranscriptionGenerationConfig.__annotations__,
        ours_audio.GeminiTranscriptionGenerationConfig.__annotations__,
        ref_audio.GeminiTranscriptionGenerationConfig.__required_keys__,
        ours_audio.GeminiTranscriptionGenerationConfig.__required_keys__,
    )
    same_typed_dict(
        ref_audio.GeminiTranscriptionInteractionRequest.__annotations__,
        ours_audio.GeminiTranscriptionInteractionRequest.__annotations__,
        ref_audio.GeminiTranscriptionInteractionRequest.__required_keys__,
        ours_audio.GeminiTranscriptionInteractionRequest.__required_keys__,
    )
    same_literals(
        ref_audio.GeminiTranscriptionAudioInput.__annotations__,
        ours_audio.GeminiTranscriptionAudioInput.__annotations__,
    )
    same_literals(
        ref_audio.GeminiTranscriptionVerbatimMode.__annotations__,
        ours_audio.GeminiTranscriptionVerbatimMode.__annotations__,
    )
    same_literals(
        ref_audio.GeminiTranscriptionInteractionRequest.__annotations__,
        ours_audio.GeminiTranscriptionInteractionRequest.__annotations__,
    )
    same_model(ref_audio.GeminiTranscriptionContent, ours_audio.GeminiTranscriptionContent)
    same_model(ref_audio.GeminiTranscriptionStep, ours_audio.GeminiTranscriptionStep)
    same_model(ref_audio.GeminiTranscriptionUsage, ours_audio.GeminiTranscriptionUsage)
    same_model(
        ref_audio.GeminiTranscriptionWordAnnotation,
        ours_audio.GeminiTranscriptionWordAnnotation,
    )
    same_model(
        ref_audio.GeminiTranscriptionModalityTokens,
        ours_audio.GeminiTranscriptionModalityTokens,
    )
    same_model(
        ref_audio.GeminiTranscriptionInteractionResponse,
        ours_audio.GeminiTranscriptionInteractionResponse,
    )
    same_typed_dict(
        ref_transcription.VertexGeminiTranscriptionInlineData.__annotations__,
        ours_transcription.VertexGeminiTranscriptionInlineData.__annotations__,
        ref_transcription.VertexGeminiTranscriptionInlineData.__required_keys__,
        ours_transcription.VertexGeminiTranscriptionInlineData.__required_keys__,
    )
    same_typed_dict(
        ref_transcription.VertexGeminiTranscriptionPart.__annotations__,
        ours_transcription.VertexGeminiTranscriptionPart.__annotations__,
        ref_transcription.VertexGeminiTranscriptionPart.__required_keys__,
        ours_transcription.VertexGeminiTranscriptionPart.__required_keys__,
    )
    same_typed_dict(
        ref_transcription.VertexGeminiTranscriptionContent.__annotations__,
        ours_transcription.VertexGeminiTranscriptionContent.__annotations__,
        ref_transcription.VertexGeminiTranscriptionContent.__required_keys__,
        ours_transcription.VertexGeminiTranscriptionContent.__required_keys__,
    )
    same_literals(
        ref_transcription.VertexGeminiTranscriptionContent.__annotations__,
        ours_transcription.VertexGeminiTranscriptionContent.__annotations__,
    )
    same_typed_dict(
        ref_transcription.VertexGeminiTranscriptionAudioConfig.__annotations__,
        ours_transcription.VertexGeminiTranscriptionAudioConfig.__annotations__,
        ref_transcription.VertexGeminiTranscriptionAudioConfig.__required_keys__,
        ours_transcription.VertexGeminiTranscriptionAudioConfig.__required_keys__,
    )
    same_typed_dict(
        ref_transcription.VertexGeminiTranscriptionGenerationConfig.__annotations__,
        ours_transcription.VertexGeminiTranscriptionGenerationConfig.__annotations__,
        ref_transcription.VertexGeminiTranscriptionGenerationConfig.__required_keys__,
        ours_transcription.VertexGeminiTranscriptionGenerationConfig.__required_keys__,
    )
    same_typed_dict(
        ref_transcription.VertexGeminiTranscriptionRequest.__annotations__,
        ours_transcription.VertexGeminiTranscriptionRequest.__annotations__,
        ref_transcription.VertexGeminiTranscriptionRequest.__required_keys__,
        ours_transcription.VertexGeminiTranscriptionRequest.__required_keys__,
    )
    same_model(
        ref_transcription.VertexGeminiTranscriptionResponsePart,
        ours_transcription.VertexGeminiTranscriptionResponsePart,
    )
    same_model(
        ref_transcription.VertexGeminiTranscriptionResponseContent,
        ours_transcription.VertexGeminiTranscriptionResponseContent,
    )
    same_model(
        ref_transcription.VertexGeminiTranscriptionCandidate,
        ours_transcription.VertexGeminiTranscriptionCandidate,
    )
    same_model(
        ref_transcription.VertexGeminiTranscriptionModalityTokens,
        ours_transcription.VertexGeminiTranscriptionModalityTokens,
    )
    same_model(
        ref_transcription.VertexGeminiTranscriptionUsageMetadata,
        ours_transcription.VertexGeminiTranscriptionUsageMetadata,
    )
    same_model(
        ref_transcription.VertexGeminiTranscriptionResponse,
        ours_transcription.VertexGeminiTranscriptionResponse,
    )


def test_speech_names() -> None:
    ref = ref_speech
    ours = ours_speech
    same_typed_dict(
        ref.VertexSpeechToTextAutoDecodingConfig.__annotations__,
        ours.VertexSpeechToTextAutoDecodingConfig.__annotations__,
        ref.VertexSpeechToTextAutoDecodingConfig.__required_keys__,
        ours.VertexSpeechToTextAutoDecodingConfig.__required_keys__,
    )
    same_typed_dict(
        ref.VertexSpeechToTextRecognitionFeatures.__annotations__,
        ours.VertexSpeechToTextRecognitionFeatures.__annotations__,
        ref.VertexSpeechToTextRecognitionFeatures.__required_keys__,
        ours.VertexSpeechToTextRecognitionFeatures.__required_keys__,
    )
    same_typed_dict(
        ref.VertexSpeechToTextRecognitionConfig.__annotations__,
        ours.VertexSpeechToTextRecognitionConfig.__annotations__,
        ref.VertexSpeechToTextRecognitionConfig.__required_keys__,
        ours.VertexSpeechToTextRecognitionConfig.__required_keys__,
    )
    same_typed_dict(
        ref.VertexSpeechToTextRecognizeRequest.__annotations__,
        ours.VertexSpeechToTextRecognizeRequest.__annotations__,
        ref.VertexSpeechToTextRecognizeRequest.__required_keys__,
        ours.VertexSpeechToTextRecognizeRequest.__required_keys__,
    )
    reference_command_union: tuple[JsonValue, ...] = get_args(
        ref.VertexSpeechStreamingCommandUnion
    )
    ours_command_union: tuple[JsonValue, ...] = get_args(ours.VertexSpeechStreamingCommandUnion)
    assert len(reference_command_union) == len(ours_command_union)
    reference_command: tuple[JsonValue, ...] = get_args(ref.VertexSpeechStreamingCommand)
    ours_command: tuple[JsonValue, ...] = get_args(ours.VertexSpeechStreamingCommand)
    assert len(reference_command) == len(ours_command)
    reference_event_union: tuple[JsonValue, ...] = get_args(ref.VertexSpeechStreamingEventUnion)
    ours_event_union: tuple[JsonValue, ...] = get_args(ours.VertexSpeechStreamingEventUnion)
    assert len(reference_event_union) == len(ours_event_union)
    reference_event: tuple[JsonValue, ...] = get_args(ref.VertexSpeechStreamingEvent)
    ours_event: tuple[JsonValue, ...] = get_args(ours.VertexSpeechStreamingEvent)
    assert len(reference_event) == len(ours_event)
    same_model(ref.VertexSpeechToTextAlternative, ours.VertexSpeechToTextAlternative)
    same_model(ref.VertexSpeechToTextResult, ours.VertexSpeechToTextResult)
    same_model(ref.VertexSpeechToTextResponseMetadata, ours.VertexSpeechToTextResponseMetadata)
    same_model(ref.VertexSpeechToTextRecognizeResponse, ours.VertexSpeechToTextRecognizeResponse)
    same_model(ref.VertexSpeechStreamingConfigure, ours.VertexSpeechStreamingConfigure)
    same_model(ref.VertexSpeechStreamingFinishTurn, ours.VertexSpeechStreamingFinishTurn)
    same_model(ref.VertexSpeechStreamingDiscardTurn, ours.VertexSpeechStreamingDiscardTurn)
    same_model(ref.VertexSpeechStreamingResult, ours.VertexSpeechStreamingResult)
    same_model(ref.VertexSpeechStreamingResponse, ours.VertexSpeechStreamingResponse)
    same_model(ref.VertexSpeechStreamingConfigured, ours.VertexSpeechStreamingConfigured)
    same_model(ref.VertexSpeechStreamingTurnFinished, ours.VertexSpeechStreamingTurnFinished)
    same_model(ref.VertexSpeechStreamingTurnDiscarded, ours.VertexSpeechStreamingTurnDiscarded)


def test_tts_names() -> None:
    ref = ref_tts
    ours = ours_tts
    same_typed_dict(
        ref.VertexTextToSpeechInput.__annotations__,
        ours.VertexTextToSpeechInput.__annotations__,
        ref.VertexTextToSpeechInput.__required_keys__,
        ours.VertexTextToSpeechInput.__required_keys__,
    )
    same_typed_dict(
        ref.VertexTextToSpeechVoice.__annotations__,
        ours.VertexTextToSpeechVoice.__annotations__,
        ref.VertexTextToSpeechVoice.__required_keys__,
        ours.VertexTextToSpeechVoice.__required_keys__,
    )
    same_typed_dict(
        ref.VertexTextToSpeechAudioConfig.__annotations__,
        ours.VertexTextToSpeechAudioConfig.__annotations__,
        ref.VertexTextToSpeechAudioConfig.__required_keys__,
        ours.VertexTextToSpeechAudioConfig.__required_keys__,
    )
    same_typed_dict(
        ref.VertexTextToSpeechRequest.__annotations__,
        ours.VertexTextToSpeechRequest.__annotations__,
        ref.VertexTextToSpeechRequest.__required_keys__,
        ours.VertexTextToSpeechRequest.__required_keys__,
    )


def test_passthrough_names() -> None:
    ref = ref_passthrough
    ours = ours_passthrough
    assert ref.VERTEX_CREDENTIALS_TYPES == ours.VERTEX_CREDENTIALS_TYPES
    same_model(ref.VertexPassThroughCredentials, ours.VertexPassThroughCredentials)
