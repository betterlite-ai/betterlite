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
            return _JSON.validate_python({"kind": "audio", "base64": encoded})
        case str() as text:
            return _JSON.validate_python({"kind": "text", "value": text})


def _command_step(
    config: realtime_transformation.VertexChirpRealtimeConfig,
    model: str,
    message: str,
) -> Json:
    try:
        commands = config.transform_realtime_request(message, model)
    except realtime_transformation.ChirpProtocolError as error:
        return _JSON.validate_python({"commands": [], "error": str(error)})
    encoded: list[Json] = [_command(command) for command in commands]
    return _JSON.validate_python({"commands": encoded, "error": None})


def _commands(input_value: CommandsInput) -> Json:
    config = _adapter(input_value["project"], input_value["location"])
    session_id = input_value["session_id"]
    created: Json = None
    if session_id is not None:
        created = _JSON.validate_python(
            config.transform_session_created_event(input_value["model"], session_id)
        )
    steps: list[Json] = [
        _command_step(config, input_value["model"], step) for step in input_value["steps"]
    ]
    document: dict[str, Json] = {
        "created": created,
        "steps": steps,
        "usage": _usage(config, input_value["model"]),
        "repeated_usage": _usage(config, input_value["model"]),
    }
    return _JSON.validate_python(document)


def _url_outcome(
    config: realtime_transformation.VertexChirpRealtimeConfig, case: UrlCase
) -> Json:
    try:
        url = config.get_complete_url(case["api_base"], case["model"])
    except ValueError as error:
        return _JSON.validate_python({"url": None, "error": str(error)})
    return _JSON.validate_python({"url": url, "error": None})


def _urls(input_value: UrlsInput) -> Json:
    config = _adapter(input_value["project"], input_value["location"])
    urls: list[Json] = [_url_outcome(config, case) for case in input_value["cases"]]
    return _JSON.validate_python({"urls": urls})


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
        result = config.transform_realtime_response(message, model, logging_obj, state)
    except realtime_transformation.ChirpProtocolError as error:
        return _JSON.validate_python(
            {"result": None, "error": {"kind": "protocol", "message": str(error)}}
        )
    except ValidationError:
        return _JSON.validate_python({"result": None, "error": {"kind": "invalid"}})
    return _JSON.validate_python({"result": _JSON.validate_python(result), "error": None})


def _responses(input_value: ResponsesInput) -> Json:
    config = _adapter(input_value["project"], input_value["location"])
    created = _JSON.validate_python(
        config.transform_session_created_event(input_value["model"], input_value["session_id"])
    )
    configure = input_value["configure"]
    if configure is not None:
        config.transform_realtime_request(configure, input_value["model"])
    logging_obj = _logging()
    responses: list[Json] = [
        _response(config, input_value["model"], frame, input_value["state"], logging_obj)
        for frame in input_value["frames"]
    ]
    document: dict[str, Json] = {
        "created": created,
        "responses": responses,
        "usage": _usage(config, input_value["model"]),
        "repeated_usage": _usage(config, input_value["model"]),
    }
    return _JSON.validate_python(document)


def run(input_value: CommandsInput | UrlsInput | ResponsesInput) -> Json:
    if input_value["function"] == "commands":
        return _commands(input_value)
    if input_value["function"] == "urls":
        return _urls(input_value)
    return _responses(input_value)
