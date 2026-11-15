import base64
import datetime as dt
from typing import Literal, TypedDict

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.realtime.transformation import RealtimeBackend
from litellm.llms.vertex_ai.audio_transcription import realtime_transformation
from litellm.types.realtime import RealtimeResponseTransformInput
from pydantic import JsonValue as Json
from pydantic import TypeAdapter, ValidationError

litellm.suppress_debug_info = True

_JSON: TypeAdapter[Json] = TypeAdapter(Json)


class CommandsInput(TypedDict):
    function: Literal["commands"]
    project: str
    location: str | None
    model: str
    session_id: str | None
    steps: list[str]


class UrlCase(TypedDict):
    model: str
    api_base: str | None


class UrlsInput(TypedDict):
    function: Literal["urls"]
    project: str
    location: str | None
    cases: list[UrlCase]


class TextFrame(TypedDict):
    kind: Literal["text"]
    value: str


class BytesFrame(TypedDict):
    kind: Literal["bytes"]
    base64: str


Frame = TextFrame | BytesFrame


class ResponsesInput(TypedDict):
    function: Literal["responses"]
    project: str
    location: str | None
    model: str
    session_id: str
    configure: str | None
    state: RealtimeResponseTransformInput
    frames: list[Frame]


async def _unresolved_token() -> str:
    message = "the access token is never resolved"
    raise RuntimeError(message)


def _unopened_backend(_target: realtime_transformation.SpeechStreamingTarget) -> RealtimeBackend:
    message = "the backend is never opened"
    raise RuntimeError(message)


def _adapter(
    project: str, location: str | None
) -> realtime_transformation.VertexChirpRealtimeConfig:
    return realtime_transformation.VertexChirpRealtimeConfig(
        resolve_access_token=_unresolved_token,
        project=project,
        location=location,
        backend_factory=_unopened_backend,
    )


def _logging() -> Logging:
    return Logging(
        model="chirp_2",
        messages=None,
        stream=False,
        call_type="realtime",
        start_time=dt.datetime(2026, 1, 1, tzinfo=dt.UTC),
        litellm_call_id="chirp-adapter",
        function_id="chirp-adapter",
    )


def _usage(config: realtime_transformation.VertexChirpRealtimeConfig, model: str) -> Json:
    usage = config.unbilled_usage_on_session_close(model)
    if usage is None:
        return None
    return _JSON.validate_python(usage)


def _command(command: str | bytes) -> Json:
    match command:
        case bytes() as audio:
            encoded = base64.b64encode(audio).decode("ascii")
            audio_command = {"kind": "audio", "base64": encoded}
            return _JSON.validate_python(audio_command)
        case str() as text:
            text_command = {"kind": "text", "value": text}
            return _JSON.validate_python(text_command)


def _command_step(
    config: realtime_transformation.VertexChirpRealtimeConfig,
    model: str,
    message: str,
) -> Json:
    try:
        commands = config.transform_realtime_request(message, model)
    except realtime_transformation.ChirpProtocolError:
        no_commands: list[Json] = []
        rejected: dict[str, Json] = {"commands": no_commands, "error": True}
        return _JSON.validate_python(rejected)
    encoded: list[Json] = [_command(command) for command in commands]
    accepted: dict[str, Json] = {"commands": encoded, "error": False}
    return _JSON.validate_python(accepted)


def _session_created(
    config: realtime_transformation.VertexChirpRealtimeConfig,
    model: str,
    session_id: str | None,
) -> Json:
    if session_id is None:
        return None
    return _JSON.validate_python(config.transform_session_created_event(model, session_id))


def _session_close(
    config: realtime_transformation.VertexChirpRealtimeConfig,
    model: str,
    body: dict[str, Json],
) -> Json:
    document: dict[str, Json] = {
        **body,
        "usage": _usage(config, model),
        "repeated_usage": _usage(config, model),
    }
    return _JSON.validate_python(document)


def _commands(input_value: CommandsInput) -> Json:
    config = _adapter(input_value["project"], input_value["location"])
    created = _session_created(config, input_value["model"], input_value["session_id"])
    steps: list[Json] = [
        _command_step(config, input_value["model"], step) for step in input_value["steps"]
    ]
    return _session_close(config, input_value["model"], {"created": created, "steps": steps})


def _url_document(url: str | None) -> Json:
    document: dict[str, Json] = {"url": url, "error": url is None}
    return _JSON.validate_python(document)


def _url_outcome(
    config: realtime_transformation.VertexChirpRealtimeConfig, case: UrlCase
) -> Json:
    try:
        url = config.get_complete_url(case["api_base"], case["model"])
    except ValueError:
        return _url_document(None)
    return _url_document(url)


def _urls(input_value: UrlsInput) -> Json:
    config = _adapter(input_value["project"], input_value["location"])
    urls: list[Json] = [_url_outcome(config, case) for case in input_value["cases"]]
    document: dict[str, Json] = {"urls": urls}
    return _JSON.validate_python(document)


def _frame_message(frame: Frame) -> str | bytes:
    if frame["kind"] == "text":
        return frame["value"]
    return base64.b64decode(frame["base64"], validate=True)


def _response(
    config: realtime_transformation.VertexChirpRealtimeConfig,
    model: str,
    frame: Frame,
    state: RealtimeResponseTransformInput,
    logging_obj: Logging,
) -> Json:
    message = _frame_message(frame)
    try:
        result: Json = _JSON.validate_python(config.transform_realtime_response(message, model, logging_obj, state))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/realtime.py#L29-L37
    except realtime_transformation.ChirpProtocolError:
        protocol_failure: dict[str, Json] = {"result": None, "error": {"kind": "protocol"}}
        return _JSON.validate_python(protocol_failure)
    except ValidationError:
        invalid_failure: dict[str, Json] = {"result": None, "error": {"kind": "invalid"}}
        return _JSON.validate_python(invalid_failure)
    answered: dict[str, Json] = {"result": result, "error": None}
    return _JSON.validate_python(answered)


def _responses(input_value: ResponsesInput) -> Json:
    config = _adapter(input_value["project"], input_value["location"])
    created = _session_created(config, input_value["model"], input_value["session_id"])
    configure = input_value["configure"]
    if configure is not None:
        config.transform_realtime_request(configure, input_value["model"])
    logging_obj = _logging()
    responses: list[Json] = [
        _response(config, input_value["model"], frame, input_value["state"], logging_obj)
        for frame in input_value["frames"]
    ]
    body: dict[str, Json] = {"created": created, "responses": responses}
    return _session_close(config, input_value["model"], body)


def run(input_value: CommandsInput | UrlsInput | ResponsesInput) -> Json:
    if input_value["function"] == "commands":
        return _commands(input_value)
    if input_value["function"] == "urls":
        return _urls(input_value)
    return _responses(input_value)
