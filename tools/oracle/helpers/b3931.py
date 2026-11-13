import json
from collections.abc import Callable
from typing import TYPE_CHECKING, Final, TypeAlias, cast, get_type_hints

import litellm
from litellm.integrations.custom_logger import CustomLogger
from litellm.types.utils import (
    StandardLoggingHiddenParams,
    StandardLoggingMetadata,
    StandardLoggingPayload,
)

if TYPE_CHECKING:
    from litellm.types.integrations.base_health_check import IntegrationHealthCheckStatus

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]


class InputError(ValueError):
    pass


def _object(value: Json, name: str) -> Fields:
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise InputError(f"{name} must be an object")
    return cast("Fields", value)


def _text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise InputError(f"{name} must be a string")
    return value


def _boolean(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise InputError(f"{name} must be a bool")
    return value


def _integer(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise InputError(f"{name} must be an integer")
    return value


def _items(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise InputError(f"{name} must be a list")
    return cast("list[Json]", value)


def _strings(value: Json, name: str) -> list[str]:
    return [_text(item, name) for item in _items(value, name)]


def _to_json(value: object) -> Json:
    return cast("Json", json.loads(json.dumps(value)))


def _null_record(template: type[object]) -> Fields:
    return cast("Fields", dict.fromkeys(get_type_hints(template)))


def _payload_record(value: Json) -> Fields:
    raw = _object(value, "payload")
    metadata = _null_record(StandardLoggingMetadata)
    metadata.update(_object(raw.get("metadata"), "payload.metadata"))
    hidden = _null_record(StandardLoggingHiddenParams)
    hidden.update(_object(raw.get("hidden_params"), "payload.hidden_params"))
    payload: Fields = {}
    for key in StandardLoggingPayload.__required_keys__:
        payload[key] = raw.get(key)
    for key in StandardLoggingPayload.__optional_keys__:
        if key in raw:
            payload[key] = raw.get(key)
    payload["metadata"] = metadata
    payload["hidden_params"] = hidden
    return payload


def _options(input_value: Fields) -> Json:
    options = _object(input_value.get("options"), "options")
    logger = cast("Callable[..., CustomLogger]", CustomLogger)(**options)
    turn_off = cast("bool | None", logger.turn_off_message_logging)
    message_logging = cast("bool | None", logger.message_logging)
    return {
        "turn_off_message_logging": False if turn_off is None else turn_off,
        "message_logging": True if message_logging is None else message_logging,
    }


def _truncate(input_value: Fields) -> Json:
    payload = cast("StandardLoggingPayload", _payload_record(input_value.get("payload")))
    logger = CustomLogger()
    truncated = logger.truncate_standard_logging_payload_content(payload)
    return _to_json(truncated)


def _metadata_field(input_value: Fields) -> Json:
    kwargs = input_value.get("request_kwargs")
    logger = CustomLogger()
    selected = logger._select_metadata_field(  # noqa: SLF001  # pyright: ignore[reportUnknownMemberType, reportPrivateUsage]
        None if kwargs is None else _object(kwargs, "request_kwargs")
    )
    return cast("Json", selected)


def _model_call_details(raw: Fields) -> Fields:
    details: Fields = {}
    standard_logging_object = raw.get("standard_logging_object")
    request = raw.get("proxy_server_request")
    if standard_logging_object is not None:
        details["standard_logging_object"] = standard_logging_object
    if request is not None:
        details["litellm_params"] = {"proxy_server_request": request}
    return details


class _SelfRedactingLogger(CustomLogger):
    def redacts_messages_itself(self) -> bool:
        return True


def _redact_view(details: Fields) -> Json:
    view: Fields = {}
    standard_logging_object = details.get("standard_logging_object")
    params = details.get("litellm_params")
    request = params.get("proxy_server_request") if isinstance(params, dict) else None
    if standard_logging_object is not None:
        view["standard_logging_object"] = standard_logging_object
    if request is not None:
        view["proxy_server_request"] = cast("Json", request)
    return view


def _redact(input_value: Fields) -> Json:
    raw = _object(input_value.get("details"), "details")
    turn_off = _boolean(
        input_value.get("turn_off_message_logging"), "turn_off_message_logging"
    )
    excluded = _strings(input_value.get("excluded_fields", []), "excluded_fields")
    redacts_itself = _boolean(
        input_value.get("redacts_messages_itself", False), "redacts_messages_itself"
    )
    logger: CustomLogger = _SelfRedactingLogger() if redacts_itself else CustomLogger()
    logger.turn_off_message_logging = turn_off
    litellm.standard_logging_payload_excluded_fields = excluded
    try:
        result = logger.redact_standard_logging_payload_from_model_call_details(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            _model_call_details(raw)
        )
    finally:
        litellm.standard_logging_payload_excluded_fields = None
    return _redact_view(cast("Fields", result))


def _base64(input_value: Fields) -> Json:
    payload = cast("StandardLoggingPayload", _payload_record(input_value.get("payload")))
    max_depth = _integer(input_value.get("max_depth"), "max_depth")
    logger = CustomLogger()
    strip = cast(
        "Callable[[StandardLoggingPayload, int], StandardLoggingPayload]",
        logger._strip_base64_from_messages_sync,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    return _to_json(strip(payload, max_depth))


class _RecordingCounter(CustomLogger):
    def __init__(self, calls: list[str]) -> None:
        super().__init__()  # pyright: ignore[reportUnknownMemberType]
        self.calls = calls

    def increment_callback_logging_failure(self, callback_name: str) -> None:
        self.calls.append(callback_name)


def _register(counters: list[CustomLogger]) -> None:
    litellm.callbacks = []
    litellm.success_callback = []
    litellm.failure_callback = []
    litellm.callbacks.extend(counters[:1])  # pyright: ignore[reportUnknownMemberType]
    litellm.success_callback.extend(counters[1:2])  # pyright: ignore[reportUnknownMemberType]
    litellm.failure_callback.extend(counters[2:])  # pyright: ignore[reportUnknownMemberType]


def _failure(input_value: Fields) -> Json:
    callback_name = _text(input_value.get("callback_name"), "callback_name")
    names = _strings(input_value.get("counters"), "counters")
    records: list[list[str]] = [[] for _ in names]
    counters: list[CustomLogger] = [
        _RecordingCounter(record) for record in records
    ]
    _register(counters)
    CustomLogger().handle_callback_failure(callback_name)
    return {"calls": cast("Json", records)}


def _health(input_value: Fields) -> Json:
    statuses: list[Json] = []
    for item in _items(input_value.get("statuses"), "statuses"):
        status = _object(item, "statuses[]")
        state = _text(status.get("state"), "state")
        entry: IntegrationHealthCheckStatus
        if state == "healthy":
            entry = {"status": "healthy", "error_message": None}
        elif state == "unhealthy":
            entry = {
                "status": "unhealthy",
                "error_message": cast("str | None", status.get("message")),
            }
        else:
            raise InputError(f"unknown health state {state}")
        statuses.append(_to_json(entry))
    return {"statuses": statuses}


_HANDLERS: Final[dict[str, Callable[[Fields], Json]]] = {
    "options": _options,
    "truncate": _truncate,
    "metadata_field": _metadata_field,
    "redact": _redact,
    "base64": _base64,
    "callback_failure": _failure,
    "health": _health,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = _text(input_value.get("operation"), "operation")
    handler = _HANDLERS.get(operation)
    if handler is None:
        raise InputError(f"unknown exporter_options operation {operation}")
    return handler(input_value)
