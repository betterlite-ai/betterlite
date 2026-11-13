import ast
import json
import sys
from collections.abc import Callable
from typing import Protocol, TypeAlias, cast, override

import litellm
from litellm.integrations.opentelemetry import OpenTelemetry, OpenTelemetryConfig
from litellm.integrations.opentelemetry_utils.gen_ai_semconv import OTELSemconvCategory

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
CAPTURE_MODES = ("NO_CONTENT", "SPAN_ONLY", "EVENT_ONLY", "SPAN_AND_EVENT")
DEFAULT_CAPTURE_MODE = "SPAN_AND_EVENT"
VENDOR_NAMES = ("langtrace", "langfuse_otel", "weave_otel")

SetAttributes = Callable[[object, object, dict[str, Json], Json], None]
SetRawRequestAttributes = Callable[[object, object, dict[str, Json], object], None]
set_attributes = cast(SetAttributes, cast(object, OpenTelemetry.set_attributes))
set_raw_request_attributes = cast(
    SetRawRequestAttributes, cast(object, OpenTelemetry.set_raw_request_attributes)
)


class AttributeSpan(Protocol):
    def set_attribute(self, key: str, value: Json) -> None: ...

    def add_event(self, *_args: object, **_kwargs: object) -> None: ...

    def record_exception(self, _exception: BaseException) -> None: ...


class RecordingSpan:
    def __init__(self) -> None:
        self.attributes: dict[str, Json] = {}

    def set_attribute(self, key: str, value: Json) -> None:
        self.attributes[key] = attribute_value(value)

    def add_event(self, *_args: object, **_kwargs: object) -> None:
        pass

    def record_exception(self, _exception: BaseException) -> None:
        pass


class RecordingLogger(OpenTelemetry):
    failures: list[str]

    @override
    def handle_callback_failure(self, callback_name: str) -> None:
        self.failures.append(str(sys.exc_info()[1]))


def attribute_value(value: Json) -> Json:
    if isinstance(value, str):
        return text_value(value)
    if value is None or isinstance(value, (bool, int, float)):
        return value
    return cast(Json, json.loads(json.dumps(value, default=str)))


def text_value(text: str) -> Json:
    parsed_ok, parsed = parse_json(text)
    if parsed_ok:
        return parsed
    return literal_value(text)


def parse_json(text: str) -> tuple[bool, Json]:
    try:
        return True, cast(Json, json.loads(text))
    except json.JSONDecodeError:
        return False, text


def literal_value(text: str) -> Json:
    if not text.startswith(("{", "[")):
        return text
    try:
        parsed = cast(object, ast.literal_eval(text))
    except (ValueError, SyntaxError):
        return text
    if not isinstance(parsed, (dict, list)):
        return text
    return cast(Json, json.loads(json.dumps(parsed, default=str)))


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_list(value: Json) -> list[str]:
    if not isinstance(value, list):
        return []
    return [item for item in value if isinstance(item, str)]


def capture_mode(value: Json) -> str:
    if isinstance(value, str) and value in CAPTURE_MODES:
        return value
    return DEFAULT_CAPTURE_MODE


def config_from(input_value: dict[str, Json]) -> OpenTelemetryConfig:
    categories: set[OTELSemconvCategory] = set()
    if input_value.get("semconv") is True:
        categories.add(OTELSemconvCategory.GEN_AI_LATEST_EXPERIMENTAL)
    return OpenTelemetryConfig(
        semconv_stability_opt_in=categories,
        baggage_metadata_keys=string_list(input_value.get("baggage_metadata_keys")),
        baggage_team_metadata_keys=string_list(input_value.get("baggage_team_metadata_keys")),
    )


def new_logger(input_value: dict[str, Json], name: str | None) -> RecordingLogger:
    logger = RecordingLogger.__new__(RecordingLogger)
    logger.failures = []
    logger.config = config_from(input_value)
    logger.callback_name = name
    logger.message_logging = True
    logger._capture_mode_cached = capture_mode(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        input_value.get("capture_mode")
    )
    litellm.turn_off_message_logging = False
    return logger


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "set_attributes":
        return {"result": attributes_record(input_value)}
    if function == "set_raw_request_attributes":
        return {"result": raw_request_record(input_value)}
    raise ValueError(f"unknown b3856 function {function} ({litellm.__name__})")


def attributes_record(input_value: dict[str, Json]) -> Json:
    names = input_value.get("callback_name")
    if isinstance(names, list):
        records: list[Json] = []
        for name in names:
            record = object_value(named_record(input_value, name_value(name)), "record")
            record["callback_name"] = name
            records.append(record)
        return {"records": records}
    return named_record(input_value, name_value(names))


def name_value(value: Json) -> str | None:
    return value if isinstance(value, str) else None


def named_record(input_value: dict[str, Json], name: str | None) -> Json:
    kwargs = object_value(input_value.get("kwargs"), "kwargs")
    span = RecordingSpan()
    logger = new_logger(input_value, name)
    set_attributes(logger, span, kwargs, input_value.get("response"))
    if logger.failures:
        return {"error": logger.failures[0], "attributes": span.attributes}
    return {"attributes": span.attributes}


def raw_request_record(input_value: dict[str, Json]) -> Json:
    kwargs = object_value(input_value.get("kwargs"), "kwargs")
    span = RecordingSpan()
    set_raw_request_attributes(new_logger(input_value, None), span, kwargs, None)
    return {"attributes": span.attributes}
