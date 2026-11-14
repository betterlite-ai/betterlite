from typing import Literal

from pydantic import JsonValue
from typing_extensions import TypedDict


class GeminiThinkingConfig(TypedDict, total=False):
    includeThoughts: bool
    thinkingBudget: int
    thinkingLevel: Literal["minimal", "low", "medium", "high"]


GeminiResponseModalities = Literal["TEXT", "IMAGE", "AUDIO", "VIDEO"]

GeminiImageAspectRatio = Literal["1:1", "2:3", "3:2", "3:4", "4:3", "9:16", "16:9", "21:9"]

GeminiImageSize = Literal["1K", "2K", "4K"]


class GeminiImageConfig(TypedDict, total=False):
    aspectRatio: GeminiImageAspectRatio
    imageSize: GeminiImageSize


class PrebuiltVoiceConfig(TypedDict):
    voiceName: str


class VoiceConfig(TypedDict):
    prebuiltVoiceConfig: PrebuiltVoiceConfig


class SpeechConfig(TypedDict, total=False):
    voiceConfig: VoiceConfig
    languageCode: str


class GenerationConfig(TypedDict, total=False):
    temperature: float
    top_p: float
    top_k: float
    candidate_count: int
    max_output_tokens: int
    stop_sequences: list[str]
    presence_penalty: float
    frequency_penalty: float
    response_mime_type: Literal["text/plain", "application/json"]
    response_schema: dict[str, JsonValue]
    response_json_schema: dict[str, JsonValue]
    responseFormat: dict[str, JsonValue]
    seed: int
    responseLogprobs: bool
    logprobs: int
    responseModalities: list[GeminiResponseModalities]
    imageConfig: GeminiImageConfig
    thinkingConfig: GeminiThinkingConfig
    mediaResolution: str
    speechConfig: SpeechConfig
