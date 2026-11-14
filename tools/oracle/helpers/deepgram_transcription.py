import os
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, cast

import httpx
from litellm.llms.deepgram.audio_transcription.transformation import (
    DeepgramAudioTranscriptionConfig,
)
from litellm.types.llms.openai import (
    OpenAIAudioTranscriptionOptionalParams as TranscriptionParams,
)
from pydantic import JsonValue, TypeAdapter


class RequestData(Protocol):
    @property
    def data(self) -> bytes | dict[str, JsonValue]: ...

    @property
    def files(self) -> dict[str, JsonValue] | None: ...

    @property
    def content_type(self) -> str | None: ...


class Transcription(Protocol):
    def get(self, key: str) -> JsonValue: ...

    def model_dump_json(self, *, exclude_none: bool = False) -> str: ...


class RaisedError(Protocol):
    @property
    def status_code(self) -> int: ...

    @property
    def message(self) -> str: ...

    @property
    def headers(self) -> dict[str, str] | httpx.Headers | None: ...


class Config(Protocol):
    def get_supported_openai_params(self, model: str) -> list[TranscriptionParams]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def validate_environment(
        self,
        _headers: dict[str, JsonValue],
        _model: str,
        _messages: list[JsonValue],
        _optional_params: dict[str, JsonValue],
        _litellm_params: dict[str, JsonValue],
        /,
        *,
        api_key: str | None = None,
    ) -> dict[str, JsonValue]: ...

    def transform_audio_transcription_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        audio_file: bytes | tuple[str, bytes],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> RequestData: ...

    def transform_audio_transcription_response(  # ast-grep-ignore: no-long-names
        self, raw_response: httpx.Response
    ) -> Transcription: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> str: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, str]
    ) -> RaisedError: ...


# LiteLLM's messages carry Any and its constructor is untyped, so the config is cast
CONFIG: Final[Config] = cast(Config, DeepgramAudioTranscriptionConfig())  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepgram/audio_transcription/transformation.py#L26
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
BYTES: Final[TypeAdapter[bytes]] = TypeAdapter(bytes)


class Params(TypedDict):
    step: Literal["params"]
    optional: dict[str, JsonValue]
    non_default: dict[str, JsonValue]


class Request(TypedDict):
    step: Literal["request"]
    audio: Literal["bytes", "tuple"]
    filename: str
    content: str


class Url(TypedDict):
    step: Literal["url"]
    model: str
    api_base: str | None
    options: dict[str, JsonValue]


class Auth(TypedDict):
    step: Literal["auth"]
    api_key: str | None


class Reply(TypedDict):
    step: Literal["reply"]
    body: str


class ErrorClass(TypedDict):
    step: Literal["error_class"]
    message: str
    status_code: int
    headers: dict[str, str]


Step: TypeAlias = Params | Request | Url | Auth | Reply | ErrorClass


class Input(TypedDict):
    env: dict[str, str]
    steps: list[Step]


def request_answer(step: Request) -> JsonValue:
    content = step["content"].encode()
    audio: bytes | tuple[str, bytes] = (
        content if step["audio"] == "bytes" else (step["filename"], content)
    )
    result = CONFIG.transform_audio_transcription_request(
        model="", audio_file=audio, optional_params={}, litellm_params={}
    )
    return {
        "data": JSON.validate_python(list(BYTES.validate_python(result.data))),
        "files": JSON.validate_python(result.files),
        "content_type": result.content_type,
    }


def reply_answer(step: Reply) -> JsonValue:
    raw = httpx.Response(200, content=step["body"].encode())
    result = CONFIG.transform_audio_transcription_response(raw_response=raw)
    return {
        "response": JSON.validate_json(result.model_dump_json(exclude_none=True)),
        "hidden": result.get("_hidden_params"),
    }


def error_class_answer(step: ErrorClass) -> JsonValue:
    error = CONFIG.get_error_class(step["message"], step["status_code"], step["headers"])
    view: dict[str, JsonValue] = {
        "status_code": error.status_code,
        "message": error.message,
        "headers": JSON.validate_python(error.headers),
    }
    return view


def outcome(step: Step) -> JsonValue:
    match step["step"]:
        case "params":
            mapped = CONFIG.map_openai_params(
                step["non_default"], step["optional"], model="", drop_params=False
            )
            supported: list[JsonValue] = [
                str(name) for name in CONFIG.get_supported_openai_params("")
            ]
            return {"supported": supported, "mapped": JSON.validate_python(mapped)}
        case "request":
            return request_answer(step)
        case "url":
            return CONFIG.get_complete_url(
                api_base=step["api_base"],
                api_key=None,
                model=step["model"],
                optional_params=step["options"],
                litellm_params={},
            )
        case "auth":
            headers = CONFIG.validate_environment(
                {}, "", [], {}, {}, api_key=step["api_key"]
            )
            return JSON.validate_python(headers)
        case "reply":
            return reply_answer(step)
        case "error_class":
            return error_class_answer(step)


def guarded(step: Step) -> JsonValue:
    try:
        return outcome(step)
    except ValueError:
        return {"raised": "ValueError"}


def run(input_value: Input) -> list[JsonValue]:
    for name in ("DEEPGRAM_API_KEY", "DEEPGRAM_API_BASE"):
        os.environ.pop(name, None)
    os.environ.update(input_value["env"])
    return [guarded(step) for step in input_value["steps"]]
