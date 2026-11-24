import datetime as dt
from collections.abc import Mapping
from typing import Final, Literal, NotRequired, TypedDict

from litellm.files.streaming import FileContentStreamingResponse
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.openai_files_endpoints import common_utils
from litellm.proxy.openai_files_endpoints.file_content_streaming_handler import (
    FileContentStreamingHandler,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

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


class HiddenView(TypedDict):
    api_base: str | None
    litellm_model_name: str | None
    response_cost: JsonValue


class PayloadView(TypedDict):
    id: str
    model: str
    api_base: str
    custom_llm_provider: str | None
    status: str
    hidden_params: HiddenView
    response: dict[str, JsonValue] | None


class ProjectionAnswer(TypedDict):
    function: Literal["streaming_projection"]
    provider: JsonValue
    file_id: JsonValue
    data: dict[str, JsonValue]
    trusted: dict[str, str] | None
    predicates: list[bool]


class HiddenAnswer(TypedDict):
    api_base: str | None
    litellm_model_name: str | None
    response_cost: JsonValue


class PayloadAnswer(TypedDict):
    id: str
    model: str
    api_base: str
    custom_llm_provider: str | None
    status: str
    hidden: HiddenAnswer
    response: dict[str, JsonValue] | None


class HiddenMetadataAnswer(TypedDict):
    function: Literal["hidden_stream_metadata"]
    payload: PayloadAnswer | None


PAYLOAD: Final = TypeAdapter(PayloadView)


class Capture(CustomLogger):
    calls: list[dict[str, JsonValue]]

    def __init__(self) -> None:
        super().__init__()
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


def text_field(value: JsonValue) -> str | None:
    match value:
        case str():
            return value
        case None | bool() | int() | float() | list() | dict():
            return None


def text_fields(snapshot: Mapping[str, JsonValue]) -> dict[str, str]:
    texts: dict[str, str] = {}
    for name, value in snapshot.items():
        text = text_field(value)
        if text is not None:
            texts[name] = text
    return texts


def predicate(provider: str | None) -> bool:
    if provider is None:
        return False
    return FileContentStreamingHandler.should_stream_file_content(custom_llm_provider=provider)


def projection_answer(call: ProjectionCall) -> JsonValue:
    predicates = [predicate(provider) for provider in call.get("predicates", [])]
    credentials = call.get("credentials")
    if call["should_route"] and credentials is not None:
        data = dict(call["data"])
        common_utils.prepare_data_with_credentials(
            data,
            credentials,
            file_id=call.get("original_file_id"),
            include_internal_credentials=True,
        )
        snapshot = text_fields(SNAPSHOT.validate_python(data.pop(SNAPSHOT_KEY)))
        data.pop("model", None)
        answer = ProjectionAnswer(
            function="streaming_projection",
            provider=credentials["custom_llm_provider"],
            file_id=data.get("file_id"),
            data=data,
            trusted=snapshot,
            predicates=predicates,
        )
        return JSON.validate_python(answer)
    answer = ProjectionAnswer(
        function="streaming_projection",
        provider=call["custom_llm_provider"],
        file_id=call["file_id"],
        data=call["data"],
        trusted=None,
        predicates=predicates,
    )
    return JSON.validate_python(answer)


def payload_view(value: JsonValue) -> PayloadView | None:
    if value is None:
        return None
    return PAYLOAD.validate_python(value)


def payload_answer(payload: PayloadView) -> PayloadAnswer:
    hidden = payload["hidden_params"]
    return PayloadAnswer(
        id=payload["id"],
        model=payload["model"],
        api_base=payload["api_base"],
        custom_llm_provider=payload["custom_llm_provider"],
        status=payload["status"],
        hidden=HiddenAnswer(
            api_base=hidden["api_base"],
            litellm_model_name=hidden["litellm_model_name"],
            response_cost=hidden["response_cost"],
        ),
        response=payload["response"],
    )


def hidden_answer(call: HiddenCall) -> JsonValue:
    capture = Capture()
    details = call["details"]
    api_base = details.get("api_base")
    kwargs: dict[str, str] | None = None
    if api_base is not None:
        kwargs = {"api_base": api_base}
    logging_obj = Logging(
        model=details["model"],
        messages=None,
        stream=False,
        call_type="afile_content",
        start_time=FIXED_TIME,
        litellm_call_id="call-1",
        function_id="fn-1",
        dynamic_success_callbacks=[capture],
        kwargs=kwargs,
    )
    response = FileContentStreamingResponse(
        iter([b"ab"]), call["file_id"], call.get("model"), call.get("custom_llm_provider"), logging_obj
    )
    _drained = list(response)
    payload: PayloadAnswer | None = None
    for recorded in capture.calls:
        view = payload_view(recorded.get("standard_logging_object"))
        if view is not None:
            payload = payload_answer(view)
    answer = HiddenMetadataAnswer(function="hidden_stream_metadata", payload=payload)
    return JSON.validate_python(answer)


def answer(call: ProjectionCall | HiddenCall) -> JsonValue:
    if call["function"] == "streaming_projection":
        return projection_answer(call)
    return hidden_answer(call)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
