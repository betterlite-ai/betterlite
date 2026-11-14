import os
from collections.abc import Callable
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
import litellm
from litellm.llms.base_llm.audio_transcription.transformation import (
    BaseAudioTranscriptionConfig,
)
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter

MODEL = "eleven_multilingual_v2"
ENVIRONMENT_NAMES = ("ELEVENLABS_API_KEY", "ELEVENLABS_API_BASE")
EMPTY: Final[dict[str, JsonValue]] = {}
SENTINEL = "global-openai-key"

# LiteLLM leaves these two dicts bare, so the typed view carries the shapes the calls read and write
Authenticate: TypeAlias = Callable[
    [
        dict[str, str],
        str,
        list[AllMessageValues],
        dict[str, JsonValue],
        dict[str, JsonValue],
        str | None,
    ],
    dict[str, str],
]


class Config(Protocol):
    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    @property
    def validate_environment(self) -> Authenticate: ...


class FormView(Protocol):
    data: dict[str, JsonValue] | bytes
    files: dict[str, JsonValue] | None


def load_base() -> BaseAudioTranscriptionConfig:
    base = ProviderConfigManager.get_provider_audio_transcription_config(
        model=MODEL, provider=LlmProviders.ELEVENLABS
    )
    if base is None:
        raise RuntimeError("LiteLLM has no ElevenLabs audio transcription config")
    return base


BASE: Final[BaseAudioTranscriptionConfig] = load_base()
CONFIG: Config = BASE
OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
STRINGS: Final[TypeAdapter[dict[str, str]]] = TypeAdapter(dict[str, str])
PARTS: Final[TypeAdapter[dict[str, tuple[str, bytes, str]]]] = TypeAdapter(
    dict[str, tuple[str, bytes, str]]
)


class MapCall(TypedDict):
    function: Literal["map_params"]
    non_default: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]


class RequestCall(TypedDict):
    function: Literal["request"]
    model: str
    filename: str | None
    content: str
    optional_params: dict[str, JsonValue]


class UrlCall(TypedDict):
    function: Literal["url"]
    api_base: str | None
    environment: dict[str, str]


class HeadersCall(TypedDict):
    function: Literal["headers"]
    headers: dict[str, str]
    api_key: str | None
    environment: dict[str, str]


class TranscriptCall(TypedDict):
    function: Literal["transcript"]
    body: str


Call: TypeAlias = MapCall | RequestCall | UrlCall | HeadersCall | TranscriptCall


class Input(TypedDict):
    calls: list[Call]


class File(TypedDict):
    name: str
    filename: str
    content_type: str
    bytes: list[int]


class Mapped(TypedDict):
    ok: bool
    supported: list[str]
    mapped: dict[str, JsonValue]


class Requested(TypedDict):
    ok: bool
    form: list[tuple[str, str]]
    file: File


class Located(TypedDict):
    ok: bool
    url: str


class Authenticated(TypedDict):
    ok: bool
    headers: dict[str, str]


class Transcribed(TypedDict):
    ok: bool
    response: dict[str, JsonValue]
    hidden: dict[str, JsonValue]


class Failure(TypedDict):
    ok: bool


Answer: TypeAlias = Mapped | Requested | Located | Authenticated | Transcribed | Failure


class Output(TypedDict):
    calls: list[Answer]


def apply_environment(environment: dict[str, str]) -> None:
    for name in ENVIRONMENT_NAMES:
        os.environ.pop(name, None)
    os.environ.update(environment)


def map_answer(call: MapCall) -> Mapped:
    mapped = CONFIG.map_openai_params(
        non_default_params=call["non_default"],
        optional_params=dict(call["optional_params"]),
        model=MODEL,
        drop_params=False,
    )
    supported = [str(name) for name in BASE.get_supported_openai_params(MODEL)]
    return Mapped(ok=True, supported=supported, mapped=mapped)


def form_answer(request: FormView) -> Requested:
    filename, body, content_type = PARTS.validate_python(request.files)["file"]
    pairs = [(name, value) for name, value in STRINGS.validate_python(request.data).items()]
    return {
        "ok": True,
        "form": pairs,
        "file": {
            "name": "file",
            "filename": filename,
            "content_type": content_type,
            "bytes": list(body),
        },
    }


def request_answer(call: RequestCall) -> Requested:
    content = call["content"].encode()
    audio = content if call["filename"] is None else (call["filename"], content)
    request = BASE.transform_audio_transcription_request(
        model=call["model"],
        audio_file=audio,
        optional_params=call["optional_params"],
        litellm_params=EMPTY,
    )
    return form_answer(request)


def url_answer(call: UrlCall) -> Located:
    apply_environment(call["environment"])
    url = BASE.get_complete_url(
        api_base=call["api_base"],
        api_key=None,
        model=MODEL,
        optional_params=EMPTY,
        litellm_params=EMPTY,
    )
    return {"ok": True, "url": url}


def headers_answer(call: HeadersCall) -> Authenticated:
    apply_environment(call["environment"])
    litellm.api_key = SENTINEL
    litellm.openai_key = SENTINEL
    headers = CONFIG.validate_environment(call["headers"], MODEL, [], EMPTY, EMPTY, call["api_key"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/elevenlabs/audio_transcription/transformation.py#L173
    return {"ok": True, "headers": headers}


def transcript_answer(call: TranscriptCall) -> Transcribed:
    body = call["body"].encode()
    response = httpx.Response(200, content=body)
    result = BASE.transform_audio_transcription_response(raw_response=response)
    # TranscriptionResponse declares usage, which the transformation never sets
    dumped = OBJECT.validate_json(result.model_dump_json())
    dumped.pop("usage", None)
    return {
        "ok": True,
        "response": dumped,
        "hidden": OBJECT.validate_json(body),
    }


def answer(call: Call) -> Answer:
    try:
        if call["function"] == "map_params":
            return map_answer(call)
        if call["function"] == "request":
            return request_answer(call)
        if call["function"] == "url":
            return url_answer(call)
        if call["function"] == "headers":
            return headers_answer(call)
        return transcript_answer(call)
    except ValueError:
        return {"ok": False}


def run(input_value: Input) -> Output:
    answers: list[Answer] = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}
