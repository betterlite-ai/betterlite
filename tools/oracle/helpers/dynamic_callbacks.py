from collections.abc import Callable
from typing import Final, TypeAlias, cast
from unittest.mock import patch

from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils import litellm_logging
from litellm.litellm_core_utils.initialize_dynamic_callback_params import (
    inherit_message_logging_privacy,
    initialize_standard_callback_dynamic_params,
)
from litellm.litellm_core_utils.litellm_logging import Logging

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
CallbackValue: TypeAlias = str | Callable[[], None] | CustomLogger
Outcome: TypeAlias = dict[str, Json]

ATTRIBUTES: Final[dict[str, str]] = {
    "input": "dynamic_input_callbacks",
    "success": "dynamic_success_callbacks",
    "async_success": "dynamic_async_success_callbacks",
    "failure": "dynamic_failure_callbacks",
    "async_failure": "dynamic_async_failure_callbacks",
}

_NAME_MEMBER: Final = "_get_callback_name"
_INTERNAL_MEMBER: Final = "_is_internal_litellm_proxy_callback"
_REMOVE_INTERNAL_MEMBER: Final = "_remove_internal_litellm_callbacks"
_REMOVE_LOGGER_MEMBER: Final = "_remove_internal_custom_logger_callbacks"
_RESOLVE_MEMBER: Final = "_resolve_dynamic_callback_string"
_COMBINE_MEMBER: Final = "get_combined_callback_list"
_BUILDER_MEMBER: Final = "_init_custom_logger_compatible_class"
_TRUSTED_MEMBER: Final = "_trusted_callback_vars"

NAME_OF: Final = cast("Callable[[Logging, object], str]", getattr(Logging, _NAME_MEMBER))
IS_INTERNAL: Final = cast(
    "Callable[[Logging, object], bool]", getattr(Logging, _INTERNAL_MEMBER)
)
REMOVE_INTERNAL: Final = cast(
    "Callable[[Logging, list[CallbackValue]], list[CallbackValue]]",
    getattr(Logging, _REMOVE_INTERNAL_MEMBER),
)
REMOVE_LOGGER: Final = cast(
    "Callable[[Logging, list[CallbackValue]], list[CallbackValue]]",
    getattr(Logging, _REMOVE_LOGGER_MEMBER),
)
RESOLVE_STRING: Final = cast(
    "Callable[[Logging, str], tuple[CustomLogger, ...]]",
    getattr(Logging, _RESOLVE_MEMBER),
)
COMBINE: Final = cast(
    "Callable[[Logging, list[CallbackValue] | None, list[CallbackValue]], list[CallbackValue]]",
    getattr(Logging, _COMBINE_MEMBER),
)

_MARKERS: Final[dict[str, CustomLogger]] = {}


class InputError(ValueError):
    pass


def marker(label: str) -> CustomLogger:
    existing = _MARKERS.get(label)
    if existing is not None:
        return existing
    logger_class = cast("type[CustomLogger]", type(label, (CustomLogger,), {}))
    instance = logger_class()
    _MARKERS[label] = instance
    return instance


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise InputError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise InputError(f"{name} must be a string")
    return value


def string_map(value: Json) -> dict[str, str]:
    if value is None:
        return {}
    result: dict[str, str] = {}
    for key, item in object_value(value, "trusted").items():
        result[key] = string_value(item, f"trusted.{key}")
    return result


def entry(value: Json) -> CallbackValue:
    if isinstance(value, str):
        return value
    return marker(string_value(object_value(value, "entry").get("logger"), "entry.logger"))


def entries(value: Json) -> list[CallbackValue]:
    if not isinstance(value, list):
        raise InputError("entries must be a list")
    return [entry(item) for item in value]


def strings(value: Json) -> list[str]:
    if not isinstance(value, list):
        raise InputError("names must be a list")
    return [string_value(item, "name") for item in value]


def entry_out(obj: Logging, callback: CallbackValue) -> Json:
    if isinstance(callback, str):
        return callback
    return {"logger": NAME_OF(obj, callback)}


def list_out(obj: Logging, callbacks: list[CallbackValue] | None) -> Json:
    if callbacks is None:
        return None
    return [entry_out(obj, callback) for callback in callbacks]


def outcomes(value: Json) -> dict[str, Outcome]:
    if value is None:
        return {}
    result: dict[str, Outcome] = {}
    for name, item in object_value(value, "resolver").items():
        result[name] = object_value(item, f"resolver.{name}")
    return result


def lists_value(value: Json) -> dict[str, list[CallbackValue] | None] | None:
    if value is None:
        return None
    spec = object_value(value, "lists")
    result: dict[str, list[CallbackValue] | None] = {}
    for key in ATTRIBUTES:
        item = spec.get(key)
        result[key] = None if item is None else entries(item)
    return result


class Recorder:
    def __init__(self, outcomes: dict[str, Outcome]) -> None:
        self.outcomes = outcomes
        self.calls: list[Json] = []

    def resolve(
        self,
        logging_integration: str,
        internal_usage_cache: object = None,
        llm_router: object = None,
        custom_logger_init_args: dict[str, object] | None = None,
    ) -> CustomLogger | None:
        del internal_usage_cache, llm_router
        params = {
            key: value
            for key, value in (custom_logger_init_args or {}).items()
            if isinstance(value, str)
        }
        self.calls.append(
            {"name": logging_integration, "params": cast(Json, params)}
        )
        outcome = self.outcomes.get(logging_integration)
        if outcome is None:
            raise LookupError(f"no callback adapter is installed for '{logging_integration}'")
        if outcome.get("error") is True:
            raise LookupError(f"initializing '{logging_integration}' failed")
        if not params and "empty_label" in outcome:
            label = outcome.get("empty_label")
        else:
            label = outcome.get("label")
        if label is None:
            return None
        return marker(cast(str, label))


def blank(
    lists: dict[str, list[CallbackValue] | None] | None, trusted: dict[str, str]
) -> Logging:
    obj: Logging = object.__new__(Logging)
    for key, attribute in ATTRIBUTES.items():
        callbacks = None if lists is None else lists[key]
        setattr(obj, attribute, callbacks)
    setattr(obj, _TRUSTED_MEMBER, tuple(trusted.items()))
    return obj


def names_object() -> Logging:
    return blank(None, {})


def add_process(input_value: dict[str, Json]) -> Json:
    recorder = Recorder(outcomes(input_value.get("resolver")))
    obj = blank(
        lists_value(input_value.get("lists")), string_map(input_value.get("trusted"))
    )
    for callback in entries(input_value.get("add", [])):
        Logging.add_dynamic_callback(obj, cast(CustomLogger, callback))
    if input_value.get("process") is True:
        with patch.object(litellm_logging, _BUILDER_MEMBER, recorder.resolve):
            Logging.process_dynamic_callbacks(obj)
    return {
        "lists": {
            key: list_out(obj, cast("list[CallbackValue] | None", getattr(obj, attribute)))
            for key, attribute in ATTRIBUTES.items()
        },
        "calls": recorder.calls,
    }


def params(input_value: dict[str, Json]) -> Json:
    kwargs: dict[str, Json] = dict(
        object_value(input_value.get("request"), "request")
    )
    kwargs["litellm_trusted_callback_vars"] = cast(
        Json, string_map(input_value.get("trusted"))
    )
    with inherit_message_logging_privacy(input_value.get("inherited") is True):
        dynamic = initialize_standard_callback_dynamic_params(
            cast("dict[str, object]", kwargs)
        )
    return cast(Json, dict(dynamic))


def combined(input_value: dict[str, Json]) -> Json:
    dynamic_value = input_value.get("dynamic")
    dynamic = None if dynamic_value is None else entries(dynamic_value)
    global_values = entries(input_value.get("global", []))
    obj = names_object()
    return {
        "with_dynamic": list_out(obj, COMBINE(obj, dynamic, global_values)),
        "without_dynamic": list_out(obj, COMBINE(obj, None, global_values)),
    }


def filtered(input_value: dict[str, Json]) -> Json:
    obj = names_object()
    callbacks = entries(input_value.get("callbacks", []))
    return {
        "names": [NAME_OF(obj, callback) for callback in callbacks],
        "internal": [IS_INTERNAL(obj, callback) for callback in callbacks],
        "litellm_kept": [
            entry_out(obj, callback) for callback in REMOVE_INTERNAL(obj, callbacks)
        ],
        "logger_kept": [
            entry_out(obj, callback) for callback in REMOVE_LOGGER(obj, callbacks)
        ],
    }


def resolved(input_value: dict[str, Json]) -> Json:
    recorder = Recorder(outcomes(input_value.get("resolver")))
    obj = blank(None, string_map(input_value.get("trusted")))
    results: list[Json] = []
    with patch.object(litellm_logging, _BUILDER_MEMBER, recorder.resolve):
        for name in strings(input_value.get("names", [])):
            try:
                loggers = RESOLVE_STRING(obj, name)
            except LookupError:
                results.append({"name": name, "error": True})
                continue
            first = loggers[0] if loggers else None
            results.append(
                {
                    "name": name,
                    "loggers": [
                        {"name": NAME_OF(obj, logger), "same_as_first": logger is first}
                        for logger in loggers
                    ],
                }
            )
    return {"calls": recorder.calls, "results": results}


def run(input_value: dict[str, Json]) -> Json:
    action = input_value.get("action", "add_process")
    if action == "add_process":
        return add_process(input_value)
    if action == "params":
        return params(input_value)
    if action == "combined":
        return combined(input_value)
    if action == "filter":
        return filtered(input_value)
    if action == "resolve":
        return resolved(input_value)
    raise InputError(f"unknown dynamic callbacks action: {action}")
