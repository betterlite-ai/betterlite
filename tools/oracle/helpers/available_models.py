import logging
import os
from collections.abc import Callable
from contextlib import ExitStack
from typing import TypeAlias, cast
from unittest import mock

import litellm
from litellm import utils
from litellm.llms.base_llm.base_utils import BaseLLMModelInfo
from litellm.types.router import LiteLLM_Params
from litellm.types.utils import LlmProviders
from litellm.utils import AvailableModelsCache, ProviderConfigManager, get_valid_models

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

WARNING_PREFIX = "Error getting valid models"


class _ListingError(Exception):
    pass


class _WarningCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.kinds: list[Json] = []

    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        kind = "error_getting_valid_models" if message.startswith(WARNING_PREFIX) else "other"
        self.kinds.append(cast(Json, {"kind": kind}))


def object_value(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_flag(fields: Object, name: str) -> bool | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, bool):
        raise TypeError(f"{name} must be a boolean")
    return value


def flag(fields: Object, name: str) -> bool:
    return optional_flag(fields, name) is True


def number(fields: Object, name: str) -> float:
    value = fields.get(name, 0.0)
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError(f"{name} must be a number")
    return float(value)


def object_list(value: Json, name: str) -> list[Object]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [object_value(item, name) for item in value]


def string_list(value: Json, name: str) -> list[str]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [string_value(item, name) for item in value]


def string_map(value: Json, name: str) -> dict[str, str]:
    entries = object_value(value, name)
    return {key: string_value(entry, f"{name}.{key}") for key, entry in entries.items()}


def models_map(value: Json, name: str) -> dict[str, list[str]]:
    entries = object_value(value, name)
    return {key: string_list(entry, f"{name}.{key}") for key, entry in entries.items()}


def parameters(fields: Object, name: str) -> LiteLLM_Params | None:
    value = fields.get(name)
    if value is None:
        return None
    params = object_value(value, name)
    return LiteLLM_Params(
        model=string_value(params.get("model"), "model"),
        api_key=optional_text(params, "api_key"),
        api_base=optional_text(params, "api_base"),
        api_version=optional_text(params, "api_version"),
        custom_llm_provider=optional_text(params, "custom_llm_provider"),
    )


def _fixed_clock(now: float) -> Callable[[], float]:
    def clock() -> float:
        return now

    return clock


def _raising_clock() -> float:
    raise RuntimeError("the case scripts a failing model cache clock")


def _clocked(cache: AvailableModelsCache, step: Object, field: str) -> None:
    if flag(step, field):
        cache._clock = _raising_clock  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    else:
        cache._clock = _fixed_clock(number(step, "now"))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def _class_for(provider: str) -> type[BaseLLMModelInfo] | None:
    config = ProviderConfigManager.get_provider_model_info(
        model=None,
        provider=LlmProviders(provider),
    )
    return None if config is None else type(config)


def _reachable_classes(names: list[str]) -> dict[str, type[BaseLLMModelInfo]]:
    classes: dict[str, type[BaseLLMModelInfo]] = {}
    for name in names:
        try:
            class_type = _class_for(name)
        except ValueError:
            continue
        if class_type is not None:
            classes[class_type.__name__] = class_type
    return classes


def _listing_fake(
    class_name: str,
    outcome: Json,
    records: list[Json],
) -> Callable[..., list[str]]:
    def get_models(
        _self: object,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> list[str]:
        records.append(
            cast(
                Json,
                {"class": class_name, "api_key": api_key, "api_base": api_base},
            )
        )
        if outcome is None:
            raise _ListingError(f"the case scripts no model listing for {class_name}")
        body = object_value(outcome, class_name)
        error = body.get("error")
        if isinstance(error, str):
            raise _ListingError(error)
        return string_list(body.get("models"), "models")

    return get_models


def _patch_listings(
    stack: ExitStack,
    classes: dict[str, type[BaseLLMModelInfo]],
    listing: Object,
    records: list[Json],
) -> None:
    for class_name, class_type in classes.items():
        outcome = listing.get(class_name)
        fake = _listing_fake(class_name, outcome, records)
        stack.enter_context(mock.patch.object(class_type, "get_models", fake))


def _custom_providers(calls: list[Object]) -> list[str]:
    names: list[str] = []
    for call in calls:
        name = optional_text(call, "custom_llm_provider")
        if name is not None and name not in names:
            names.append(name)
    return names


def _call(
    call: Object,
    classes: dict[str, type[BaseLLMModelInfo]],
    cache: AvailableModelsCache,
    *,
    default_global_check: bool,
) -> Json:
    records: list[Json] = []
    handler = _WarningCapture()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(handler)
    global_check = optional_flag(call, "global_check_provider_endpoint")
    try:
        with ExitStack() as stack:
            listing = object_value(call.get("listing", {}), "listing")
            _patch_listings(stack, classes, listing, records)
            _clocked(cache, call, "cache_clock_fails")
            stack.enter_context(
                mock.patch.object(
                    litellm,
                    "check_provider_endpoint",
                    default_global_check if global_check is None else global_check,
                )
            )
            models = get_valid_models(
                check_provider_endpoint=optional_flag(call, "check_provider_endpoint"),
                custom_llm_provider=optional_text(call, "custom_llm_provider"),
                litellm_params=parameters(call, "litellm_params"),
                api_key=optional_text(call, "api_key"),
                api_base=optional_text(call, "api_base"),
            )
    finally:
        logger.removeHandler(handler)
    return {"models": cast(Json, models), "warnings": handler.kinds, "listings": records}


def _enumerate(fields: Object) -> Json:
    environment = string_map(fields.get("environment", {}), "environment")
    provider_list = string_list(fields.get("provider_list", []), "provider_list")
    models_by_provider = models_map(fields.get("models_by_provider", {}), "models_by_provider")
    global_check = flag(fields, "global_check_provider_endpoint")
    calls = object_list(fields.get("calls"), "calls")
    classes = _reachable_classes([*provider_list, *_custom_providers(calls)])
    cache = AvailableModelsCache()
    with (
        mock.patch.dict(os.environ, environment, clear=True),
        mock.patch.object(litellm, "provider_list", provider_list),
        mock.patch.object(litellm, "models_by_provider", models_by_provider),
        mock.patch.object(utils, "_model_cache", cache),
    ):
        results = [
            _call(call, classes, cache, default_global_check=global_check) for call in calls
        ]
    return {"calls": results}


def _read(
    cache: AvailableModelsCache,
    provider: str | None,
    params: LiteLLM_Params | None,
    *,
    mutate: bool,
) -> Json:
    op = "get_mutate" if mutate else "get"
    try:
        value = cache.get_cached_model_info(provider, params)
    except RuntimeError:
        return {"failed": {"op": op}}
    recorded: Json = None if value is None else list(value)
    if mutate and value is not None:
        value.append("injected")
    return {"get": recorded}


def _write(
    cache: AvailableModelsCache,
    step: Object,
    provider: str | None,
    params: LiteLLM_Params | None,
    *,
    mutate: bool,
) -> Json:
    if provider is None:
        raise ValueError("a set step needs a provider")
    models = string_list(step.get("models", []), "models")
    try:
        cache.set_cached_model_info(provider, params, models)
    except RuntimeError:
        return {"failed": {"op": "set_mutate" if mutate else "set"}}
    if mutate:
        models.append("injected")
    return {"set": None}


def _cache_step(cache: AvailableModelsCache, step: Object) -> Json:
    op = string_value(step.get("op"), "op")
    provider = optional_text(step, "provider")
    params = parameters(step, "parameters")
    if op == "config":
        return {
            "config": {
                "capacity": cache.max_size_in_memory,
                "default_ttl": float(cache.default_ttl),
            }
        }
    if op == "get":
        return _read(cache, provider, params, mutate=False)
    if op == "get_mutate":
        return _read(cache, provider, params, mutate=True)
    if op in {"set", "set_mutate"}:
        return _write(cache, step, provider, params, mutate=op == "set_mutate")
    raise ValueError(f"unknown cache step op: {op}")


def _cache(fields: Object) -> Json:
    steps = object_list(fields.get("steps"), "steps")
    cache = AvailableModelsCache()
    outputs: list[Json] = []
    for step in steps:
        _clocked(cache, step, "clock_fails")
        environment = string_map(step.get("environment", {}), "environment")
        with mock.patch.dict(os.environ, environment, clear=True):
            outputs.append(_cache_step(cache, step))
    return {"steps": outputs}


def run(argument: Json) -> Json:
    fields = object_value(argument, "input")
    if len(fields) != 1:
        raise ValueError("expected one available_models variant")
    name, value = next(iter(fields.items()))
    if name == "enumerate":
        return _enumerate(object_value(value, name))
    if name == "cache":
        return _cache(object_value(value, name))
    raise ValueError(f"unknown available_models case: {name}")
