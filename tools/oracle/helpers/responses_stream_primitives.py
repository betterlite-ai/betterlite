from collections.abc import Callable
from datetime import datetime
from types import SimpleNamespace
from typing import Final, Literal, TypeAlias, TypedDict, TypeVar

import httpx
import litellm
from litellm.exceptions import Timeout
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.responses import streaming_iterator as primitives
from litellm.responses.streaming_iterator import BaseResponsesAPIStreamingIterator as StreamBase
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
# the base constructor takes dict[str, object] maps, so the JSON-facing maps bind to that shape
OBJECTS: Final = TypeAdapter(dict[str, object] | None)
# the private loaders, guards and classifiers the stream iterator wraps
LOAD_VALUE: Final = primitives._load_json_value  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L153
IS_OBJECT: Final = primitives._is_json_object  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L80
IS_ARRAY: Final = primitives._is_json_array  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L84
OPTIONAL_STR: Final = primitives._optional_str  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L88
ARRAY_OR_EMPTY: Final = primitives._json_array_or_empty  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L93
IS_STR_MAPPING: Final = primitives._is_str_mapping  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L98
ERROR_FIELDS: Final = primitives._error_event_fields  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L221
STATUS_AND_MESSAGE: Final = primitives.stream_error_status_and_message
MAP_ERROR: Final = primitives._map_stream_error_to_exception  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L258
FALLBACK_ELIGIBLE: Final = primitives._mid_stream_fallback_eligible  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L281


class ProbeSpec(TypedDict):
    model: str
    provider: str | None
    litellm_metadata: dict[str, JsonValue] | None
    request_data: dict[str, JsonValue] | None
    call_type: str | None
    start_time: str
    created: float
    litellm_params: dict[str, JsonValue]
    headers: dict[str, str]


class BaseCall(TypedDict):
    operation: Literal["base_stream_state"]
    rows: list[ProbeSpec]


class ShapesCall(TypedDict):
    operation: Literal["json_shape_guards"]
    payloads: list[str]


class ErrorRow(TypedDict):
    kind: Literal["mapping", "attributes"]
    fields: dict[str, JsonValue]


class ErrorsCall(TypedDict):
    operation: Literal["stream_error_fields"]
    rows: list[ErrorRow]


class MappedRow(TypedDict):
    model: str
    provider: str
    fields: dict[str, JsonValue]


class MappedCall(TypedDict):
    operation: Literal["mapped_stream_failure"]
    rows: list[MappedRow]


class DurationRow(TypedDict):
    environment: str | None
    created: float
    now: float
    model: str
    provider: str | None


class DurationCall(TypedDict):
    operation: Literal["duration_boundary"]
    rows: list[DurationRow]


Input: TypeAlias = BaseCall | ShapesCall | ErrorsCall | MappedCall | DurationCall
Row = TypeVar("Row")


class ErrorAttributes:
    def __init__(self, fields: dict[str, JsonValue]) -> None:
        self.message = fields.get("message")
        self.type = fields.get("type")
        self.code = fields.get("code")


class StartTimeRecord(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    start_time: datetime


class MappedRecord(BaseModel):
    status_code: int
    error_type: str | None = Field(alias="type")
    param: str | None = None
    code: str | None = None


class TimeoutRecord(BaseModel):
    status_code: int
    message: str
    model: str
    llm_provider: str


class Clock:
    def __init__(self, moments: list[float]) -> None:
        self.moments = list(moments)

    def __call__(self) -> float:
        return self.moments.pop(0)


class StreamProbe(StreamBase):
    def duration_check(self) -> None:
        self._check_max_streaming_duration()

    def state(self) -> dict[str, JsonValue]:
        record = StartTimeRecord.model_validate(self)
        answer: dict[str, JsonValue] = {
            "model": self.model,
            "custom_llm_provider": self.custom_llm_provider,
            "litellm_metadata": JSON.validate_python(self.litellm_metadata),
            "request_data": JSON.validate_python(self.request_data),
            "call_type": self.call_type,
            "start_time": record.start_time.isoformat(),
            "created_timestamp": self._stream_created_time,
            "finished": self.finished,
            "failure_handled": self._failure_handled,
            "yielded_first_chunk": self._yielded_first_chunk,
            "output_started": self._output_started,
            "completed_response_cache_hit": self._completed_response_cache_hit,
            "completed_response_cached": self._completed_response_cached,
            "completed_response_logged": self._completed_response_logged,
            "persist_before_logging": self._persist_completed_response_before_logging,
            "generated_content": self._generated_content,
            "generated_tool_arguments": self._generated_tool_arguments,
            "pending_tasks": len(self._pending_logging_tasks),
            "hidden_params": JSON.validate_python(self._hidden_params),
            "raw_response_headers": JSON.validate_python(dict(self._raw_response_headers)),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L349
        }
        return answer


def rows_answer(rows: list[Row], build: Callable[[Row], JsonValue]) -> JsonValue:
    answers: list[JsonValue] = [build(row) for row in rows]
    return {"rows": answers}


def stream_probe(row: ProbeSpec, clock: Clock, limit: float | None) -> StreamProbe:
    logging_object = Logging(
        model=row["model"],
        messages=None,
        stream=True,
        call_type="responses",
        start_time=datetime.fromisoformat(row["start_time"]),
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    details: dict[str, JsonValue] = logging_object.model_call_details
    details["litellm_params"] = row["litellm_params"]
    object.__setattr__(primitives, "time", SimpleNamespace(time=clock))
    object.__setattr__(primitives, "LITELLM_MAX_STREAMING_DURATION_SECONDS", limit)
    return StreamProbe(
        response=httpx.Response(
            status_code=200,
            headers=row["headers"],
            request=httpx.Request("POST", "https://example.test/v1/responses"),
        ),
        model=row["model"],
        responses_api_provider_config=None,
        logging_obj=logging_object,
        litellm_metadata=OBJECTS.validate_python(row["litellm_metadata"]),
        custom_llm_provider=row["provider"],
        request_data=OBJECTS.validate_python(row["request_data"]),
        call_type=row["call_type"],
    )


def base_answer(row: ProbeSpec) -> JsonValue:
    probe = stream_probe(row, Clock([row["created"]]), None)
    return probe.state()


def shape_answer(payload: str) -> JsonValue:
    try:
        loaded: JsonValue = JSON.validate_python(LOAD_VALUE(payload))
    except ValueError:
        return {"payload": payload, "decoded": False}
    answer: dict[str, JsonValue] = {
        "payload": payload,
        "decoded": True,
        "value": loaded,
        "is_object": IS_OBJECT(loaded),
        "is_array": IS_ARRAY(loaded),
        "optional_str": OPTIONAL_STR(loaded),
        "array_or_empty": JSON.validate_python(list(ARRAY_OR_EMPTY(loaded))),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L93-L95
        "str_mapping": IS_STR_MAPPING(loaded),
    }
    return answer


def errors_answer(row: ErrorRow) -> JsonValue:
    error_object: dict[str, JsonValue] | ErrorAttributes = (
        row["fields"] if row["kind"] == "mapping" else ErrorAttributes(row["fields"])
    )
    status, message = STATUS_AND_MESSAGE(error_object)
    _, error_type, error_code = ERROR_FIELDS(error_object)
    answer: dict[str, JsonValue] = {"kind": row["kind"], "status": status}
    answer["message"] = message
    answer["type"] = error_type
    answer["code"] = error_code
    return answer


def mapped_answer(row: MappedRow) -> JsonValue:
    mapped = MAP_ERROR(row["fields"], row["model"], row["provider"])
    record = MappedRecord.model_validate(mapped, from_attributes=True)
    answer: dict[str, JsonValue] = {
        "class": type(mapped).__name__,
        "status": record.status_code,
        "type": record.error_type,
        "param": record.param,
        "code": record.code,
        "eligible": FALLBACK_ELIGIBLE(mapped),
    }
    return answer


def duration_answer(row: DurationRow) -> JsonValue:
    environment = row["environment"]
    try:
        limit = None if environment is None else float(environment)
    except ValueError:
        return {"loaded": False}
    spec: ProbeSpec = {
        "model": row["model"],
        "provider": row["provider"],
        "litellm_metadata": None,
        "request_data": None,
        "call_type": None,
        "start_time": "2026-01-01T00:00:00+00:00",
        "created": row["created"],
        "litellm_params": {},
        "headers": {},
    }
    probe = stream_probe(spec, Clock([row["created"], row["now"]]), limit)
    try:
        probe.duration_check()
    except Timeout as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L356
        record = TimeoutRecord.model_validate(error, from_attributes=True)
        raised: dict[str, JsonValue] = {
            "loaded": True,
            "limit": None if limit is None else repr(limit),
            "raised": True,
            "status": record.status_code,
            "message": record.message,
            "model": record.model,
            "llm_provider": record.llm_provider,
        }
        return raised
    provider = "" if probe.custom_llm_provider is None else probe.custom_llm_provider
    return {
        "loaded": True,
        "limit": None if limit is None else repr(limit),
        "raised": False,
        "status": None,
        "message": None,
        "model": probe.model,
        "llm_provider": provider,
    }


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "base_stream_state":
        return rows_answer(input_value["rows"], base_answer)
    if input_value["operation"] == "json_shape_guards":
        return rows_answer(input_value["payloads"], shape_answer)
    if input_value["operation"] == "stream_error_fields":
        return rows_answer(input_value["rows"], errors_answer)
    if input_value["operation"] == "mapped_stream_failure":
        return rows_answer(input_value["rows"], mapped_answer)
    return rows_answer(input_value["rows"], duration_answer)
