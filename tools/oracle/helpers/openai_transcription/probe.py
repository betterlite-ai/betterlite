from dataclasses import dataclass, field
from typing import Final, Literal

import litellm
from litellm.llms.base_llm.audio_transcription.transformation import (
    BaseAudioTranscriptionConfig,
)
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

litellm.suppress_debug_info = True

JSON_HEADERS: Final = {"content-type": "application/json"}


def audio_config(model: str) -> BaseAudioTranscriptionConfig:
    config = ProviderConfigManager.get_provider_audio_transcription_config(
        model=model, provider=LlmProviders.OPENAI
    )
    if config is None:
        raise LookupError(model)
    return config


PROVIDERS: Final = {
    "whisper": audio_config("whisper-1"),
    "gpt": audio_config("gpt-4o-transcribe"),
}


@dataclass
class Answer:
    status: int = 200
    headers: dict[str, str] = field(default_factory=JSON_HEADERS.copy)
    body: str = '{"text":"hi"}'


@dataclass
class Body:
    body: str
    content_type: str


@dataclass
class Probe:
    model: str
    provider: Literal["whisper", "gpt"] | None = None
    optional_params: dict[str, Json] = field(default_factory=dict)
    non_default_params: dict[str, Json] = field(default_factory=dict)
    existing_params: dict[str, Json] = field(default_factory=dict)
    headers: dict[str, str] = field(default_factory=dict)
    api_key: str | None = None
    api_base: str | None = None
    atranscription: bool = False
    reply: Answer = field(default_factory=Answer)
    data_model: str | None = None
    data_model_null: bool = False
    texts: list[str] = field(default_factory=list)
    bodies: list[Body] = field(default_factory=list)


PROBE: Final = TypeAdapter(Probe)
