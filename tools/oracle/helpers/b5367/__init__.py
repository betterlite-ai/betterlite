from __future__ import annotations

import datetime as dt
from contextlib import ExitStack
from typing import Final, Protocol, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm import utils
from litellm._logging import verbose_logger
from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils import litellm_logging
from litellm.litellm_core_utils.rules import Rules
from litellm.models.credentials import CredentialItem

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
CallbackValue: TypeAlias = str | CustomLogger

# ruff: noqa: SLF001 - the case is about the private helpers LiteLLM routes through


class Utils(Protocol):
    def function_setup(
        self,
        original_function: str,
        rules_obj: object,
        start_time: object,
        **kwargs: Json,
    ) -> tuple[object, Fields]: ...

    def load_credentials_from_list(self, kwargs: Fields) -> None: ...

    def _is_async_request(self, kwargs: Fields, *, is_pass_through: bool = False) -> bool: ...

    def _is_streaming_request(self, kwargs: Fields, call_type: str) -> bool: ...


U: Final = cast(Utils, utils)

_LIST_FIELDS: Final[dict[str, str]] = {
    "input": "input_callback",
    "success": "success_callback",
    "async_success": "_async_success_callback",
    "failure": "failure_callback",
    "async_failure": "_async_failure_callback",
}

_DYNAMIC_FIELDS: Final[dict[str, str]] = {
    "input": "dynamic_input_callbacks",
    "success": "dynamic_success_callbacks",
    "async_success": "dynamic_async_success_callbacks",
    "failure": "dynamic_failure_callbacks",
    "async_failure": "dynamic_async_failure_callbacks",
}


class InputError(ValueError):
    pass


class _Marker(CustomLogger):
    pass


class _GuardrailMarker(CustomGuardrail):
    pass


_MARKERS: Final[dict[str, CustomLogger]] = {}
_LOG_CLASSES: Final[dict[str, type[_Marker]]] = {}
_GUARD_CLASSES: Final[dict[str, type[_GuardrailMarker]]] = {}
_CALLBACK_NAME: Final = "callback_name"
_CALLBACK_LIST: Final = "callback_list"


class _StubLogging:
    def __init__(self, **kwargs: object) -> None:
        self.init_kwargs: dict[str, object] = kwargs

    def update_environment_variables(self, **kwargs: object) -> None:
        self.update_kwargs: dict[str, object] = kwargs


def _object(value: Json, name: str) -> Fields:
    if not isinstance(value, dict):
        raise InputError(f"{name} must be an object")
    return dict(cast("dict[str, Json]", value))


def _items(value: Json, name: str) -> list[Json]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise InputError(f"{name} must be a list")
    return list(cast("list[Json]", value))


def _text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise InputError(f"{name} must be a string")
    return value


def _strings(value: Json, name: str) -> list[str]:
    return [_text(item, name) for item in _items(value, name)]


def _flag(value: Json, name: str, *, default: bool) -> bool:
    if value is None:
        return default
    if not isinstance(value, bool):
        raise InputError(f"{name} must be a bool")
    return value


def _guardrail(spec: Json) -> tuple[str | None, bool] | None:
    if spec is None:
        return None
    fields = _object(spec, "guardrail")
    name = fields.get("name")
    return (
        None if name is None else _text(name, "guardrail.name"),
        _flag(fields.get("default_on"), "guardrail.default_on", default=False),
    )


def _marker(label: str, guardrail: tuple[str | None, bool] | None) -> CustomLogger:
    existing = _MARKERS.get(label)
    if existing is not None:
        return existing
    if guardrail is None:
        logger_class = _LOG_CLASSES.get(label)
        if logger_class is None:
            name = f"Marker_{len(_LOG_CLASSES) + 1}"
            logger_class = cast("type[_Marker]", type(name, (_Marker,), {}))
            _LOG_CLASSES[label] = logger_class
        instance: CustomLogger = logger_class()
    else:
        guardrail_name, default_on = guardrail
        guard_class = _GUARD_CLASSES.get(label)
        if guard_class is None:
            name = f"Guardrail_{len(_GUARD_CLASSES) + 1}"
            guard_class = cast("type[_GuardrailMarker]", type(name, (_GuardrailMarker,), {}))
            _GUARD_CLASSES[label] = guard_class
        instance = guard_class(guardrail_name=guardrail_name, default_on=default_on)
    setattr(instance, _CALLBACK_NAME, label)
    _MARKERS[label] = instance
    return instance


def _entry(value: Json) -> CallbackValue:
    if isinstance(value, str):
        return value
    fields = _object(value, "entry")
    label = _text(fields.get("logger"), "entry.logger")
    return _marker(label, _guardrail(fields.get("guardrail")))


def _entries(value: Json, name: str) -> list[CallbackValue]:
    return [_entry(item) for item in _items(value, name)]


def _lists(value: Json) -> dict[str, list[CallbackValue]] | None:
    if value is None:
        return None
    spec = _object(value, "lists")
    return {
        key: _entries(spec.get(key), f"lists.{key}")
        for key in _LIST_FIELDS
    }


def _outcomes(value: Json) -> dict[str, Fields]:
    if value is None:
        return {}
    result: dict[str, Fields] = {}
    for name, item in _object(value, "resolver").items():
        result[name] = _object(item, f"resolver.{name}")
    return result


class _FixedRule:
    def __init__(self, decision: Json) -> None:
        self._decision = decision

    def __call__(self, _input: str) -> Json:
        return self._decision


def _rules(value: Json) -> list[object]:
    return [None if item is None else _FixedRule(item) for item in _items(value, "rules")]


class _Recorder:
    def __init__(self, outcomes: dict[str, Fields]) -> None:
        self.outcomes = outcomes
        self.calls: list[Fields] = []

    def resolve(
        self,
        logging_integration: str,
        internal_usage_cache: object = None,
        llm_router: object = None,
        custom_logger_init_args: dict[str, object] | None = None,
    ) -> CustomLogger | None:
        del internal_usage_cache, llm_router
        self.calls.append({"name": logging_integration})
        outcome = self.outcomes.get(logging_integration)
        if outcome is None or outcome.get("error") is True:
            return None
        label = outcome.get("label")
        del custom_logger_init_args
        if label is None:
            return None
        return _marker(_text(label, "resolver.label"), _guardrail(outcome.get("guardrail")))


def _entry_out(callback: object) -> Json:
    if isinstance(callback, str):
        return callback
    name = getattr(callback, "callback_name", None)
    if not isinstance(name, str):
        name = type(callback).__name__
    return {"logger": name}


def _callbacks_out(value: object) -> Json:
    if value is None:
        return None
    if not isinstance(value, list):
        raise InputError("a dynamic callback list is not a list")
    return [_entry_out(item) for item in cast("list[object]", value)]


def _logging_out(logging_obj: object) -> Json:
    stub = cast(_StubLogging, logging_obj)
    init = stub.init_kwargs
    dynamic = {
        key: _callbacks_out(init.get(attribute))
        for key, attribute in _DYNAMIC_FIELDS.items()
    }
    return {
        "model": cast(Json, init.get("model")),
        "call_type": cast(Json, init.get("call_type")),
        "function_id": cast(Json, init.get("function_id")),
        "stream": cast(Json, init.get("stream")),
        "applied_guardrails": cast(Json, init.get("applied_guardrails")),
        "dynamic": cast(Json, dynamic),
    }


def _stub_getter() -> type[_StubLogging]:
    return _StubLogging


def setup(input_value: Fields) -> Json:
    callbacks = _entries(input_value.get("callbacks"), "callbacks")
    lists = _lists(input_value.get("lists")) or {key: [] for key in _LIST_FIELDS}
    outcomes = _outcomes(input_value.get("resolver"))
    recorder = _Recorder(outcomes)
    provider_map = _strings(input_value.get("provider_map"), "provider_map")
    with ExitStack() as stack:
        stack.enter_context(
            patch.object(litellm_logging, "_init_custom_logger_compatible_class", recorder.resolve)
        )
        stack.enter_context(patch.object(utils, "get_litellm_logging_class", _stub_getter))
        configured = _rules(input_value.get("rules"))
        # the caller owns the callback list here, so LiteLLM never rebuilds its globals
        setattr(utils, _CALLBACK_LIST, [object()])
        stack.enter_context(patch.object(litellm, "pre_call_rules", configured))
        litellm.callbacks = cast("list[object]", callbacks)
        litellm.input_callback = cast("list[object]", lists["input"])
        litellm.success_callback = cast("list[object]", lists["success"])
        litellm._async_success_callback = cast(  # pyright: ignore[reportPrivateUsage]
            "list[object]", lists["async_success"]
        )
        litellm.failure_callback = cast("list[object]", lists["failure"])
        litellm._async_failure_callback = cast(  # pyright: ignore[reportPrivateUsage]
            "list[object]", lists["async_failure"]
        )
        litellm.custom_provider_map = cast(
            "list[object]", [{"provider": name} for name in provider_map]
        )
        litellm.provider_list = _strings(input_value.get("providers"), "providers")
        custom_providers = _strings(input_value.get("custom_providers"), "custom_providers")
        litellm._custom_providers = custom_providers  # pyright: ignore[reportPrivateUsage]
        kwargs = _object(input_value.get("request"), "request")
        logging_obj, changed = U.function_setup(
            original_function=_text(input_value.get("call_type"), "call_type"),
            rules_obj=Rules(),
            start_time=_moment(),
            is_async_call=_flag(input_value.get("is_async_call"), "is_async_call", default=True),
            **kwargs,
        )
    return {
        "request": cast(Json, changed),
        "lists": {
            key: [_entry_out(item) for item in cast("list[object]", getattr(litellm, attribute))]
            for key, attribute in _LIST_FIELDS.items()
        },
        "providers": {
            "providers": cast(Json, list(litellm.provider_list)),
            "custom_providers": cast(
                Json,
                list(litellm._custom_providers),  # pyright: ignore[reportPrivateUsage]
            ),
        },
        "logging": _logging_out(logging_obj),
    }


def _moment() -> object:
    return dt.datetime(2026, 1, 1, tzinfo=dt.UTC)


def credentials(input_value: Fields) -> Json:
    entries: list[CredentialItem] = []
    for item in _items(input_value.get("credentials"), "credentials"):
        fields = _object(item, "credential")
        values = _object(fields.get("credential_values"), "credential_values")
        entries.append(
            CredentialItem(
                credential_name=_text(fields.get("credential_name"), "credential_name"),
                credential_info={},
                credential_values=values,
            )
        )
    litellm.credential_list = cast("list[object]", entries)
    kwargs = _object(input_value.get("request"), "request")
    with patch.object(verbose_logger, "warning") as warning:
        U.load_credentials_from_list(kwargs)
    reported: Json = None
    if warning.call_args is not None:
        reported = cast(Json, warning.call_args.args[1])
    return {"request": cast(Json, kwargs), "warning": reported}


def request_flags(input_value: Fields) -> Json:
    kwargs = _object(input_value.get("request"), "request")
    return {
        "async": U._is_async_request(  # pyright: ignore[reportPrivateUsage]
            kwargs=kwargs,
            is_pass_through=_flag(
                input_value.get("is_pass_through"), "is_pass_through", default=False
            ),
        ),
        "streaming": U._is_streaming_request(  # pyright: ignore[reportPrivateUsage]
            kwargs=kwargs, call_type=_text(input_value.get("call_type"), "call_type")
        ),
    }


def run(input_value: Fields) -> Json:
    action = input_value.get("action", "setup")
    if action == "setup":
        return setup(input_value)
    if action == "credentials":
        return credentials(input_value)
    if action == "request_flags":
        return request_flags(input_value)
    raise InputError(f"unknown call setup action: {action!r}")
