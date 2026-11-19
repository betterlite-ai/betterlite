import asyncio
from collections.abc import Coroutine
from datetime import UTC, datetime
from typing import Final, Literal, Protocol

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
    OpenAIWhisperAudioTranscriptionConfig,
)
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import FileTypes, TranscriptionResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

Mode = Literal["sync", "async"]
ProviderConfig = Literal["present", "missing"]

AUDIO: Final[FileTypes] = ("a.wav", b"abc", "audio/wav")
DEFAULT_TIMEOUT: Final = 600.0
HEADERS: Final = TypeAdapter(dict[str, object] | None)
RESPONSE: Final = TypeAdapter(TranscriptionResponse)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException


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
    headers: dict[str, str] | None = None
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


class FixtureConfig(OpenAIWhisperAudioTranscriptionConfig):
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
        messages: list[AllMessageValues],
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
    # ast-grep-ignore: no-long-names
    def transform_audio_transcription_request(
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
    # ast-grep-ignore: no-long-names
    def transform_audio_transcription_response(
        self,
        raw_response: httpx.Response,
    ) -> TranscriptionResponse:
        _ = raw_response
        if self.call.response is None:
            return TranscriptionResponse()
        return TranscriptionResponse.model_validate(self.call.response)

    @property
    @override
    def supports_subtitle_synthesis(self) -> bool:
        return self.call.supports_subtitle_synthesis

    @override
    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, str] | httpx.Headers
    ) -> BaseLLMException:
        return BaseLLMException(status_code=status_code, message=error_message, headers=headers)


class Handlers(Protocol):
    def audio_transcriptions(
        self,
        _model: str,
        _audio_file: FileTypes,
        _optional_params: dict[str, JsonValue],
        _litellm_params: dict[str, JsonValue],
        _model_response: TranscriptionResponse,
        _timeout: float,
        _max_retries: int,
        _logging_obj: Logging,
        _api_key: str | None,
        _api_base: str | None,
        _custom_llm_provider: str,
        /,
        *,
        atranscription: bool,
        headers: dict[str, object] | None,
        provider_config: FixtureConfig | None,
    ) -> Coroutine[object, object, TranscriptionResponse] | TranscriptionResponse: ...

    def async_audio_transcriptions(
        self,
        _model: str,
        _audio_file: FileTypes,
        _optional_params: dict[str, JsonValue],
        _litellm_params: dict[str, JsonValue],
        _model_response: TranscriptionResponse,
        _timeout: float,
        _max_retries: int,
        _logging_obj: Logging,
        _api_key: str | None,
        _api_base: str | None,
        _custom_llm_provider: str,
        /,
        *,
        headers: dict[str, object] | None,
        provider_config: FixtureConfig | None,
    ) -> Coroutine[object, object, TranscriptionResponse]: ...


HANDLER: Handlers = BaseLLMHTTPHandler()


def logging_fixture() -> Logging:
    return Logging(
        model="",
        messages="",
        stream=False,
        call_type="transcription",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="transcription-http",
        function_id="transcription-http",
    )


def observed(response: TranscriptionResponse, names: list[str]) -> dict[str, str]:
    headers = get_provider_response_headers_from_hidden_params(response)
    if headers is None:
        return {}
    return {name: headers[name] for name in names if name in headers}


def view(response: TranscriptionResponse, names: list[str]) -> View:
    fields = RESPONSE_TEXT.validate_json(response.model_dump_json())
    return View(text=fields.text, words=fields.words, headers=observed(response, names))


def exchange(
    call: HttpCall, provider_config: FixtureConfig | None, api_base: str
) -> TranscriptionResponse:
    logging_obj = logging_fixture()
    timeout = DEFAULT_TIMEOUT if call.timeout_seconds is None else float(call.timeout_seconds)
    headers = HEADERS.validate_python(call.headers)
    if call.mode == "async" or call.atranscription:
        return asyncio.run(
            HANDLER.async_audio_transcriptions(
                call.model,
                AUDIO,
                call.optional_params,
                call.litellm_params,
                TranscriptionResponse(),
                timeout,
                call.max_retries,
                logging_obj,
                call.api_key,
                api_base,
                "openai",
                headers=headers,
                provider_config=provider_config,
            )
        )
    return RESPONSE.validate_python(
        HANDLER.audio_transcriptions(
            call.model,
            AUDIO,
            call.optional_params,
            call.litellm_params,
            TranscriptionResponse(),
            timeout,
            call.max_retries,
            logging_obj,
            call.api_key,
            api_base,
            "openai",
            atranscription=False,
            headers=headers,
            provider_config=provider_config,
        )
    )


def run_http(call: HttpCall, api_base: str) -> Outcome:
    config = FixtureConfig(call)
    provider_config = config if call.provider_config == "present" else None
    try:
        response = exchange(call, provider_config, api_base)
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
