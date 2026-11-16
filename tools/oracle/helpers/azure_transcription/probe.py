from dataclasses import dataclass, field
from typing import Final

import litellm
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

from openai_transcription.probe import Answer

litellm.suppress_debug_info = True

FILE: Final = ("a.wav", b"abc", "audio/wav")
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException


@dataclass
class Body:
    status: int = 200
    body: str = ""


@dataclass
class Probe:
    model: str
    optional_params: dict[str, Json] = field(default_factory=dict)
    non_default_params: dict[str, Json] = field(default_factory=dict)
    existing_params: dict[str, Json] = field(default_factory=dict)
    headers: dict[str, str] = field(default_factory=dict)
    api_key: str | None = None
    environment_api_key: str | None = None
    api_base: str | None = None
    environment_api_base: str | None = None
    bases: list[str | None] = field(default_factory=list)
    atranscription: bool = False
    api_version: str | None = None
    azure_ad_token: str | None = None
    max_retries: int | None = None
    client_kind: str | None = None
    reply: Answer = field(default_factory=Answer)
    bodies: list[Body] = field(default_factory=list)
    audio: str = ""


PROBE: Final = TypeAdapter(Probe)
