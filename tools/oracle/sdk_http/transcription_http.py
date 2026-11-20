import asyncio
from collections.abc import Coroutine, Sequence
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypeAlias

import httpx
from litellm.litellm_core_utils.core_helpers import (
    get_provider_response_headers_from_hidden_params,
)
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.audio_transcription.transformation import (
    AudioTranscriptionRequestData,
)
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.openai.transcriptions.whisper_transformation import (
    OpenAIWhisperAudioTranscriptionConfig as WhisperConfig,
)
from litellm.types.utils import FileTypes, TranscriptionResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

Mode = Literal["sync", "async"]
ProviderConfig = Literal["present", "missing"]

AUDIO: Final[FileTypes] = ("a.wav", b"abc", "audio/wav")
RESPONSE: Final = TypeAdapter(TranscriptionResponse)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException
AsyncWork: TypeAlias = Coroutine[object, object, TranscriptionResponse]
Work: TypeAlias = TranscriptionResponse | AsyncWork


class Transform(BaseModel):
    model_config = ConfigDict(extra="forbid")

    data: dict[str, JsonValue] | None = None
    raw: str | None = None
    files: dict[str, str] | None = None
    content_type: str | None = None


class HttpCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    mode: Mode
    model: str
    api_key: str | None = None
    optional_params: dict[str, JsonValue] = Field(default_factory=dict)
    litellm_params: dict[str, JsonValue] = Field(default_factory=dict)
    transform: Transform = Field(default_factory=Transform)
    response: dict[str, JsonValue] | None = None
    supports_subtitle_synthesis: bool = False
    provider_config: ProviderConfig = "present"
    atranscription: bool = False
    max_retries: int = 0
    timeout_seconds: int | None = None
    observe_headers: list[str] = Field(default_factory=list)


class ResponseText(BaseModel):
    text: str | None = None
    words: JsonValue = None


RESPONSE_TEXT: Final = TypeAdapter(ResponseText)


class View(BaseModel):
    model_config = ConfigDict(extra="forbid")

    text: str | None
    words: JsonValue
    headers: dict[str, str]


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int = Field(serialization_alias="status")
    status_code_is_synthesized: bool = Field(serialization_alias="synthesized")
    message: str
    headers: dict[str, JsonValue]


class Raised(BaseModel):
    model_config = ConfigDict(extra="forbid")

    cls: str = Field(serialization_alias="class")
    message: str


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    result: View | None
    failure: Failure | None
    raised: Raised | None


class TranscriptionHttpValue(BaseModel):
    value: Outcome


class Envelope(BaseModel):
    transcription_http: TranscriptionHttpValue


class FixtureConfig(WhisperConfig):
    def __init__(self, call: HttpCall) -> None:
        self.call = call

    @override
    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        stream: bool | None = None,
    ) -> str:
        _ = api_key, model, optional_params, litellm_params, stream
        base = "" if api_base is None else api_base
        return f"{base}/audio/transcriptions"

    @override
    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        # ast-grep-ignore: no-loose-types
        messages: Sequence[object],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> dict[str, str]:
        _ = model, messages, optional_params, litellm_params, api_base
        validated = dict(headers)
        if api_key is not None:
            validated["Authorization"] = f"Bearer {api_key}"
        return validated

    @override
    def transform_audio_transcription_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        audio_file: FileTypes,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> AudioTranscriptionRequestData:
        _ = model, audio_file, optional_params, litellm_params
        transform = self.call.transform
        if transform.raw is not None:
            return AudioTranscriptionRequestData(
                data=transform.raw.encode(), content_type=transform.content_type
            )
        files: dict[str, bytes] | None = None
        if transform.files is not None:
            files = {name: text.encode() for name, text in transform.files.items()}
        data: dict[str, JsonValue] = {} if transform.data is None else dict(transform.data)
        return AudioTranscriptionRequestData(
            data=data, files=files, content_type=transform.content_type
        )

    @override
    def transform_audio_transcription_response(  # ast-grep-ignore: no-long-names
        self,
        raw_response: httpx.Response,
    ) -> TranscriptionResponse:
        _ = raw_response
        if self.call.response is None:
            return TranscriptionResponse()
        return RESPONSE.validate_python(self.call.response)

    @property
    @override
    def supports_subtitle_synthesis(self) -> bool:
        return self.call.supports_subtitle_synthesis


class Handlers(Protocol):
    def audio_transcriptions(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L1642-L1665
        self,
        *,
        model: str,
        audio_file: FileTypes,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        model_response: TranscriptionResponse,
        timeout: float,
        max_retries: int,
        logging_obj: Logging,
        api_key: str | None,
        api_base: str | None,
        custom_llm_provider: str,
        atranscription: bool,
        provider_config: FixtureConfig | None,
    ) -> Work: ...

    def async_audio_transcriptions(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L1728-L1751
        self,
        *,
        model: str,
        audio_file: FileTypes,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        model_response: TranscriptionResponse,
        timeout: float,
        max_retries: int,
        logging_obj: Logging,
        api_key: str | None,
        api_base: str | None,
        custom_llm_provider: str,
        provider_config: FixtureConfig | None,
    ) -> AsyncWork: ...


HANDLER: Handlers = BaseLLMHTTPHandler()


def view(response: TranscriptionResponse, names: list[str]) -> View:
    fields = RESPONSE_TEXT.validate_json(response.model_dump_json())
    hidden = get_provider_response_headers_from_hidden_params(response)
    headers: dict[str, str] = {}
    if hidden is not None:
        headers = {name: hidden[name] for name in names if name in hidden}
    return View(text=fields.text, words=fields.words, headers=headers)


def run_http(call: HttpCall, api_base: str) -> Outcome:
    logging_obj = Logging(
        model="",
        messages="",
        stream=False,
        call_type="transcription",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="transcription-http",
        function_id="transcription-http",
    )
    provider_config = FixtureConfig(call) if call.provider_config == "present" else None
    timeout = 600.0 if call.timeout_seconds is None else float(call.timeout_seconds)
    try:
        if call.mode == "async" or call.atranscription:
            response = asyncio.run(
                HANDLER.async_audio_transcriptions(
                    model=call.model,
                    audio_file=AUDIO,
                    optional_params=call.optional_params,
                    litellm_params=call.litellm_params,
                    model_response=TranscriptionResponse(),
                    timeout=timeout,
                    max_retries=call.max_retries,
                    logging_obj=logging_obj,
                    api_key=call.api_key,
                    api_base=api_base,
                    custom_llm_provider="openai",
                    provider_config=provider_config,
                )
            )
        else:
            response = RESPONSE.validate_python(
                HANDLER.audio_transcriptions(
                    model=call.model,
                    audio_file=AUDIO,
                    optional_params=call.optional_params,
                    litellm_params=call.litellm_params,
                    model_response=TranscriptionResponse(),
                    timeout=timeout,
                    max_retries=call.max_retries,
                    logging_obj=logging_obj,
                    api_key=call.api_key,
                    api_base=api_base,
                    custom_llm_provider="openai",
                    atranscription=False,
                    provider_config=provider_config,
                )
            )
    except PROVIDER_ERROR as error:
        return Outcome(result=None, failure=Failure.model_validate(error), raised=None)
    except ValueError as error:
        raised = Raised(cls=type(error).__name__, message=str(error))
        return Outcome(result=None, failure=None, raised=raised)
    return Outcome(result=view(response, call.observe_headers), failure=None, raised=None)


class TranscriptionHttpRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    transcription_http: HttpCall

    def serialize(self, api_base: str) -> str:
        outcome = run_http(self.transcription_http, api_base)
        envelope = Envelope(transcription_http=TranscriptionHttpValue(value=outcome))
        return envelope.model_dump_json(by_alias=True)
