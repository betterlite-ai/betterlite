from collections.abc import Callable, Sequence
from typing import Protocol, TypeAlias, runtime_checkable

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.text_to_speech.transformation import BaseTextToSpeechConfig
from litellm.types.llms.openai import HttpxBinaryResponseContent
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import ConfigDict, JsonValue, TypeAdapter

from .inputs import Headers, Object, SpeechRequestData, Voice

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

MODEL: str = "eleven_multilingual_v2"
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

Supported: TypeAlias = Callable[[str], Sequence[str]]
Mapper: TypeAlias = Callable[[str, Object, Voice, bool, Object], tuple[str | None, Object]]
Requester: TypeAlias = Callable[[str, str, Voice, Object, Object, Headers], SpeechRequestData]
Responder: TypeAlias = Callable[[str, httpx.Response, Logging], HttpxBinaryResponseContent]


class Handler(Protocol):
    def text_to_speech_handler(self, **kwargs: JsonValue) -> JsonValue: ...


Dispatcher: TypeAlias = Callable[
    [
        str,
        str,
        Voice,
        Object,
        Object,
        Logging,
        float,
        Object | None,
        Handler,
        bool,
        str | None,
        str | None,
    ],
    HttpxBinaryResponseContent,
]


@runtime_checkable
class SpeechConfig(Protocol):
    @property
    def get_supported_openai_params(self) -> Supported: ...

    @property
    def map_openai_params(self) -> Mapper: ...

    @property
    def transform_text_to_speech_request(self) -> Requester: ...  # ast-grep-ignore: no-long-names

    @property
    def transform_text_to_speech_response(self) -> Responder: ...  # ast-grep-ignore: no-long-names

    @property
    def dispatch_text_to_speech(self) -> Dispatcher: ...


BASE: BaseTextToSpeechConfig | None = ProviderConfigManager.get_provider_text_to_speech_config(
    model=MODEL, provider=LlmProviders.RUNWAYML
)
if BASE is None:
    raise RuntimeError("LiteLLM has no RunwayML text-to-speech config")
SPEECH: TypeAdapter[SpeechConfig] = TypeAdapter(
    SpeechConfig, config=ConfigDict(arbitrary_types_allowed=True)
)
CONFIG: SpeechConfig = SPEECH.validate_python(BASE)
