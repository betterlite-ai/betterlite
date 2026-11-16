import asyncio
import base64
import inspect
import io
import re
from collections.abc import Awaitable
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.soniox.audio_transcription.handler import (
    SonioxAudioTranscriptionHandler as Handler,
)
from litellm.llms.soniox.common_utils import SonioxException
from litellm.types.utils import FileTypes, TranscriptionResponse
from pydantic import BaseModel, Field, JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

JSON: Final = TypeAdapter(JsonValue)
OBJECT: Final = TypeAdapter(dict[str, JsonValue])
HANDLER: Final = Handler()
CLIENT_TIMEOUT_MILLIS: Final = 900000
HIDDEN_NAMES: Final = ("model", "custom_llm_provider", "audio_transcription_duration")
AUTHORIZATION: Final = "authorization"
# LiteLLM's class carries Any in its constructor, so the catch names it through a typed alias
PROVIDER_ERROR: Final[type[Exception]] = SonioxException
Mode: TypeAlias = Literal["sync", "async"]
OutcomeKind: TypeAlias = Literal["response", "provider", "failure"]
FailureKind: TypeAlias = Literal["provider", "failure"]


class Answer(TypedDict):
    status: int
    body: str
    headers: NotRequired[dict[str, str]]


class Audio(TypedDict):
    filename: str
    content: str


class Case(TypedDict):
    model: str
    timeout_millis: int
    api_key: str
    api_base: str
    optional_params: dict[str, JsonValue]
    answers: list[Answer]
    audio: NotRequired[Audio]
    headers: NotRequired[dict[str, str]]


class Input(TypedDict):
    pipeline: Case


class HiddenRecord(BaseModel):
    hidden_params: dict[str, JsonValue] = Field(alias="_hidden_params")


class ErrorRecord(BaseModel):
    status_code: int


class ScriptedResponse(httpx.Response):
    @override
    def raise_for_status(self) -> httpx.Response:
        return self


class NamedAudio(io.BytesIO):
    name: str

    def __init__(self, filename: str, content: bytes) -> None:
        super().__init__(content)
        self.name = filename


class Transport:
    def __init__(self, case: Case, effects: list[JsonValue], timeout_millis: int) -> None:
        self.answers = list(case["answers"])
        self.effects = effects
        self.names = echoed_names(case)
        self.timeout_millis = timeout_millis

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.effects.append(request_view(request, self.names, self.timeout_millis))
        if not self.answers:
            raise httpx.ConnectError("the scripted transport has no answer left")
        answer = self.answers.pop(0)
        return ScriptedResponse(
            answer["status"], headers=answer.get("headers"), content=answer["body"].encode()
        )


class RecordingLogging(Logging):
    @override
    def __init__(self, effects: list[JsonValue]) -> None:
        self.effects = effects

    @override
    def pre_call(self, *_args: JsonValue, **kwargs: JsonValue) -> None:
        arguments = OBJECT.validate_python(kwargs["additional_args"])
        effect: dict[str, JsonValue] = {
            "effect": "pre_call",
            "url": arguments["api_base"],
            "body": arguments["complete_input_dict"],
        }
        self.effects.append(effect)

    @override
    def post_call(self, *_args: JsonValue, **kwargs: JsonValue) -> None:
        arguments = OBJECT.validate_python(kwargs["additional_args"])
        effect: dict[str, JsonValue] = {
            "effect": "post_call",
            "input": kwargs["input"],
            "original_response": kwargs["original_response"],
            "body": arguments["complete_input_dict"],
        }
        self.effects.append(effect)


def capture(match: re.Match[str] | None) -> str | None:
    if match is None:
        return None
    return match.group(1)


def header_text(request: httpx.Request, name: str) -> str | None:
    if name in request.headers:
        return request.headers[name]
    return None


def echoed_names(case: Case) -> list[str]:
    supplied = case.get("headers")
    if supplied is None:
        return [AUTHORIZATION]
    return sorted([AUTHORIZATION, *[name.lower() for name in supplied]])


def part_view(raw: bytes) -> dict[str, JsonValue]:
    head, _, body = raw.partition(b"\r\n\r\n")
    text = head.decode()
    name = re.search(r'name="([^"]*)"', text)
    filename = re.search(r'filename="([^"]*)"', text)
    content_type = re.search(r"(?im)^content-type: (.*)$", text)
    return {
        "name": capture(name),
        "filename": capture(filename),
        "content_type": capture(content_type),
        "body": base64.b64encode(body.removesuffix(b"\r\n")).decode(),
    }


def multipart_view(request: httpx.Request, content_type: str) -> JsonValue:
    boundary = content_type.split("boundary=", 1)[1].encode()
    pieces = [piece.removeprefix(b"\r\n") for piece in request.content.split(b"--" + boundary)]
    parts = [part_view(piece) for piece in pieces if piece.startswith(b"Content-Disposition")]
    return JSON.validate_python(parts)


def request_view(request: httpx.Request, names: list[str], timeout_millis: int) -> JsonValue:
    view: dict[str, JsonValue] = {
        "effect": "request",
        "method": request.method,
        "url": str(request.url),
        "headers": JSON.validate_python(
            {name: header_text(request, name) for name in names}
        ),
        "timeout_millis": timeout_millis,
    }
    content_type = header_text(request, "content-type")
    if content_type is None:
        return JSON.validate_python(view)
    if content_type.startswith("multipart/form-data"):
        view["files"] = multipart_view(request, content_type)
    elif content_type.startswith("application/json"):
        view["json"] = JSON.validate_json(request.content)
    return JSON.validate_python(view)


def audio_input(case: Case) -> FileTypes | None:
    audio = case.get("audio")
    if audio is None:
        return None
    return NamedAudio(audio["filename"], audio["content"].encode())


def case_headers(case: Case) -> dict[str, str]:
    headers = case.get("headers")
    if headers is None:
        return {}
    return headers


def hidden_view(response: TranscriptionResponse) -> JsonValue:
    hidden = HiddenRecord.model_validate(response, from_attributes=True).hidden_params
    present = {name: hidden[name] for name in HIDDEN_NAMES if name in hidden}
    return JSON.validate_python(present)


def outcome(kind: OutcomeKind, response: TranscriptionResponse) -> JsonValue:
    value = JSON.validate_json(response.model_dump_json())
    hidden = hidden_view(response)
    return {"kind": kind, "value": value, "hidden": hidden}


def failure(
    kind: FailureKind, response: TranscriptionResponse, error: ErrorRecord | None
) -> JsonValue:
    view = OBJECT.validate_python(outcome(kind, response))
    if error is not None:
        view["status"] = error.status_code
    return JSON.validate_python(view)


def client_for(mode: Mode, transport: Transport) -> HTTPHandler | AsyncHTTPHandler:
    if mode == "sync":
        return HTTPHandler(
            client=httpx.Client(
                transport=httpx.MockTransport(transport),
                timeout=httpx.Timeout(CLIENT_TIMEOUT_MILLIS / 1000),
            )
        )
    return AsyncHTTPHandler(
        transport=httpx.MockTransport(transport), timeout=CLIENT_TIMEOUT_MILLIS / 1000
    )


async def resolved(
    pending: TranscriptionResponse | Awaitable[TranscriptionResponse],
) -> TranscriptionResponse:
    if inspect.isawaitable(pending):
        return await pending
    return pending


def invoke(
    case: Case,
    mode: Mode,
    transport: Transport,
    logging: RecordingLogging,
    response: TranscriptionResponse,
) -> None:
    params: dict[str, JsonValue] = {}
    pending = HANDLER.audio_transcriptions(
        model=case["model"],
        audio_file=audio_input(case),
        optional_params=case["optional_params"],
        litellm_params=params,
        model_response=response,
        timeout=case["timeout_millis"] / 1000,
        max_retries=0,
        logging_obj=logging,
        api_key=case["api_key"],
        api_base=case["api_base"],
        client=client_for(mode, transport),
        atranscription=mode == "async",
        headers=case_headers(case),
    )
    asyncio.run(resolved(pending))


def perform(case: Case, mode: Mode) -> JsonValue:
    effects: list[JsonValue] = []
    transport = Transport(case, effects, case["timeout_millis"])
    logging = RecordingLogging(effects)
    response = TranscriptionResponse(text="")
    result: JsonValue
    try:
        invoke(case, mode, transport, logging, response)
    except PROVIDER_ERROR as error:
        captured = ErrorRecord.model_validate(error, from_attributes=True)
        result = failure("provider", response, captured)
    except (KeyError, TypeError, ValueError):
        result = failure("failure", response, None)
    else:
        result = outcome("response", response)
    return {"effects": JSON.validate_python(effects), "outcome": result}


def run(input_value: Input) -> JsonValue:
    case = input_value["pipeline"]
    return {"sync": perform(case, "sync"), "async": perform(case, "async")}
