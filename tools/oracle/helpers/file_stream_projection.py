import datetime as dt
from collections.abc import Mapping
from typing import Final, Literal, NotRequired, Protocol, TypedDict

import litellm
from litellm.files.streaming import FileContentStreamingResponse
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.openai_files_endpoints import common_utils
from litellm.proxy.openai_files_endpoints.file_content_streaming_handler import (
    FileContentStreamingHandler,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

JSON: Final = TypeAdapter(JsonValue)
SNAPSHOT: Final = TypeAdapter(Mapping[str, JsonValue])
SNAPSHOT_KEY: Final = "_litellm_internal_model_credentials"
FIXED_TIME: Final = dt.datetime(2026, 1, 1, tzinfo=dt.UTC)


class Details(TypedDict):
    model: str
    api_base: NotRequired[str | None]


class ProjectionCall(TypedDict):
    function: Literal["streaming_projection"]
    custom_llm_provider: str
    file_id: str
    data: dict[str, JsonValue]
    should_route: bool
    original_file_id: NotRequired[str | None]
    credentials: NotRequired[dict[str, JsonValue] | None]
    predicates: NotRequired[list[str | None]]


class HiddenCall(TypedDict):
    function: Literal["hidden_stream_metadata"]
    file_id: str
    model: NotRequired[str | None]
    custom_llm_provider: NotRequired[str | None]
    details: Details


class Input(TypedDict):
    calls: list[ProjectionCall | HiddenCall]


class PayloadView(TypedDict):
    id: str
    model: str
    api_base: str
    custom_llm_provider: str | None
    status: str
    hidden_params: dict[str, JsonValue]
    response: dict[str, JsonValue] | None


PAYLOAD: Final = TypeAdapter(PayloadView)


class Capture(CustomLogger):
    calls: list[dict[str, JsonValue]]

    def __init__(self) -> None:
        self.calls = []

    @override
    def log_success_event(
        self,
        kwargs: dict[str, JsonValue],
        response_obj: JsonValue,
        start_time: dt.datetime | None,
        end_time: dt.datetime | None,
    ) -> None:
        del response_obj, start_time, end_time
        self.calls.append(kwargs)


class CommonUtils(Protocol):
    def prepare_data_with_credentials(
        self,
        data: dict[str, JsonValue],
        credentials: dict[str, JsonValue],
        *,
        file_id: str | None = ...,
        include_internal_credentials: bool = ...,
    ) -> None: ...


def text_fields(snapshot: Mapping[str, JsonValue]) -> dict[str, JsonValue]:
    texts: dict[str, JsonValue] = {}
    for name, value in snapshot.items():
        match value:
            case str():
                texts[name] = value
            case None | bool() | int() | float() | list() | dict():
                continue
    return texts


def predicates(call: ProjectionCall) -> list[JsonValue]:
    providers = call.get("predicates")
    checks: list[JsonValue] = []
    if providers is None:
        return checks
    for name in providers:
        enabled = name is not None and FileContentStreamingHandler.should_stream_file_content(
            custom_llm_provider=name
        )
        checks.append(enabled)
    return checks


def projection_answer(
    call: ProjectionCall,
    utils: CommonUtils = common_utils,
) -> JsonValue:
    credentials = call.get("credentials")
    data = call["data"]
    file_id: JsonValue = call["file_id"]
    provider: JsonValue = call["custom_llm_provider"]
    trusted: dict[str, JsonValue] | None = None
    if call["should_route"] and credentials is not None:
        data = dict(data)
        utils.prepare_data_with_credentials(
            data,
            credentials,
            file_id=call.get("original_file_id"),
            include_internal_credentials=True,
        )
        trusted = text_fields(SNAPSHOT.validate_python(data.pop(SNAPSHOT_KEY)))
        data.pop("model", None)
        file_id = data.get("file_id")
        provider = credentials["custom_llm_provider"]
    answer: JsonValue = {
        "function": "streaming_projection",
        "provider": provider,
        "file_id": file_id,
        "data": data,
        "trusted": trusted,
        "predicates": predicates(call),
    }
    return JSON.validate_python(answer)


def payload_view(value: JsonValue) -> JsonValue:
    if value is None:
        return None
    payload = PAYLOAD.validate_python(value)
    hidden = payload["hidden_params"]
    answer: JsonValue = {
        "id": payload["id"],
        "model": payload["model"],
        "api_base": payload["api_base"],
        "custom_llm_provider": payload["custom_llm_provider"],
        "status": payload["status"],
        "hidden": {
            "api_base": hidden["api_base"],
            "litellm_model_name": hidden["litellm_model_name"],
            "response_cost": hidden["response_cost"],
        },
        "response": payload["response"],
    }
    return JSON.validate_python(answer)


def hidden_answer(call: HiddenCall) -> JsonValue:
    capture = Capture()
    details = call["details"]
    api_base = details.get("api_base")
    kwargs: dict[str, str] | None = None if api_base is None else {"api_base": api_base}
    logging_obj = Logging(
        model=details["model"],
        messages=None,
        stream=False,
        call_type="afile_content",
        start_time=FIXED_TIME,
        litellm_call_id="call-1",
        function_id="fn-1",
        kwargs=kwargs,
    )
    logging_obj.add_dynamic_callback(capture)
    response = FileContentStreamingResponse(
        iter([b"ab"]),
        call["file_id"],
        call.get("model"),
        call.get("custom_llm_provider"),
        logging_obj,
    )
    _drained = list(response)
    payload: JsonValue = None
    for recorded in capture.calls:
        view = payload_view(recorded.get("standard_logging_object"))
        if view is not None:
            payload = view
    answer: JsonValue = {"function": "hidden_stream_metadata", "payload": payload}
    return JSON.validate_python(answer)


def answer(call: ProjectionCall | HiddenCall) -> JsonValue:
    if call["function"] == "streaming_projection":
        return projection_answer(call)
    return hidden_answer(call)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
