import io
import json
from collections.abc import Callable, Generator, Mapping
from contextlib import contextmanager, redirect_stdout
from functools import lru_cache
from typing import Literal, Protocol, TypeAlias, TypedDict, cast
from unittest import mock

import httpx
import litellm
import litellm.utils as litellm_utils
from litellm.exceptions import ModelNotMappedError
from litellm.litellm_core_utils import fallback_generalizations as engine
from litellm.litellm_core_utils.get_supported_openai_params import (
    get_supported_openai_params,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.llms.lemonade.chat.transformation import LemonadeChatConfig
from litellm.llms.ollama.common_utils import OllamaModelInfo
from litellm.utils import (
    _invalidate_model_cost_lowercase_map,  # pyright: ignore[reportPrivateUsage]
    get_model_cost_mutation_generation,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
ModelCost: TypeAlias = dict[str, Object]
RequestType: TypeAlias = Literal["chat_completion", "embeddings", "transcription"]


class CacheInfoView(Protocol):
    @property
    def hits(self) -> int: ...

    @property
    def misses(self) -> int: ...

    @property
    def maxsize(self) -> int | None: ...

    @property
    def currsize(self) -> int: ...


class CachedLookup(Protocol):
    def __call__(self, *args: object, **kwargs: object) -> Object: ...

    def cache_clear(self) -> None: ...

    def cache_info(self) -> CacheInfoView: ...


class SupportedParamsCall(Protocol):
    def __call__(
        self,
        model: str,
        custom_llm_provider: str | None = None,
        request_type: RequestType = "chat_completion",
        base_model: str | None = None,
    ) -> list[object] | None: ...


class Outcome(TypedDict):
    result: Json
    error: bool


class ParameterQueryError(Exception):
    """The fixture asked the parameter query to fail before the helper runs."""


REAL_PARAMS = cast(SupportedParamsCall, get_supported_openai_params)
add_known_models = cast(Callable[[ModelCost], None], litellm.add_known_models)
set_rules = cast(
    Callable[[list[Json] | None], None], engine.set_fallback_generalizations
)


def object_value(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def integer_value(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(f"{name} must be an integer")
    return value


def optional_text(fields: Mapping[str, Json], name: str) -> str | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def object_value_or_none(value: Json, name: str) -> Object | None:
    return None if value is None else object_value(value, name)


def object_list(value: Json, name: str) -> list[Object]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [object_value(item, name) for item in value]


def json_value(value: object) -> Json:
    return cast("Json", json.loads(json.dumps(value)))


class Driver:
    def __init__(self, fixture: Object) -> None:
        self.calls: list[Json] = []
        self.records: list[Object] = []
        self.script: Object | None = None
        self.costs: ModelCost = {}
        self.dynamic = object_value_or_none(fixture.get("dynamic"), "dynamic")


class FakeClient:
    def get(self, *_args: object, **_kwargs: object) -> httpx.Response:
        raise httpx.ConnectError("no fixture transport")


def model_cost_of(fixture: Object) -> ModelCost:
    return {
        name: object_value(entry, name)
        for name, entry in object_value(fixture.get("model_cost", {}), "model_cost").items()
    }


def fixture_rules(fixture: Object) -> list[Json]:
    rules = fixture.get("fallback_generalizations")
    if rules is None:
        return []
    if not isinstance(rules, list):
        raise TypeError("fallback_generalizations must be a list")
    return rules


def dynamic_handler(adapter: str, driver: Driver) -> Callable[..., Json]:
    def handler(
        _self: object,
        model: str,
        api_base: str | None = None,
        api_key: str | None = None,
    ) -> Json:
        driver.calls.append(
            {
                "kind": "dynamic",
                "adapter": adapter,
                "model": model,
                "api_base": api_base,
                "authenticated": api_key is not None,
            }
        )
        script = driver.dynamic
        if script is None:
            return None
        if string_value(script.get("kind"), "kind") == "error":
            raise RuntimeError("dynamic model info failed")
        return json_value(script.get("body"))

    return handler


def b175_answer(
    script: Object,
    model: str,
    provider: str | None,
    request_type: str,
    base_model: str | None,
) -> Json:
    selected_type = string_value(script.get("call_type", request_type), "call_type")
    selected_base = optional_text(script, "base_model")
    if selected_base is None:
        selected_base = base_model
    result = REAL_PARAMS(
        model=model,
        custom_llm_provider=provider,
        request_type=cast("RequestType", selected_type),
        base_model=selected_base,
    )
    return cast("Json", result)


def parameter_answer(
    script: Object | None,
    model: str,
    provider: str | None,
    request_type: str,
    base_model: str | None,
) -> Json:
    if script is None:
        return None
    kind = string_value(script.get("kind"), "kind")
    if kind == "value":
        body = script.get("body")
        if body is None:
            return None
        if not isinstance(body, list):
            raise TypeError("body must be a list")
        return [string_value(item, "body") for item in body]
    if kind == "b175":
        return b175_answer(script, model, provider, request_type, base_model)
    return None


def parameter_probe(driver: Driver) -> Callable[..., Json]:
    def probe(
        model: str,
        custom_llm_provider: str | None = None,
        request_type: str = "chat_completion",
        base_model: str | None = None,
    ) -> Json:
        script = driver.script
        if script is not None and script.get("kind") == "error":
            driver.calls.append(
                {
                    "kind": "parameters",
                    "model": model,
                    "provider": custom_llm_provider,
                    "raised": True,
                }
            )
            raise ParameterQueryError
        answer = parameter_answer(
            script, model, custom_llm_provider, request_type, base_model
        )
        driver.calls.append(
            {
                "kind": "parameters",
                "model": model,
                "provider": custom_llm_provider,
                "answer": answer,
            }
        )
        return answer

    return probe


def install_caches(maxsize: int) -> None:
    helper_of = litellm_utils._get_model_info_helper  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    build = litellm_utils._build_model_info  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    helper = cast(CachedLookup, lru_cache(maxsize=maxsize)(helper_of))
    full = cast(CachedLookup, lru_cache(maxsize=maxsize)(build))
    litellm_utils._cached_get_model_info_helper = helper  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    litellm_utils._cached_get_model_info = full  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    setattr(litellm_utils.get_model_info, "cache_clear", full.cache_clear)  # noqa: B010
    setattr(litellm_utils.get_model_info, "cache_info", full.cache_info)  # noqa: B010


@contextmanager
def installed(fixture: Object, driver: Driver) -> Generator[None]:
    driver.costs = model_cost_of(fixture)
    costs = driver.costs
    with (
        mock.patch.object(litellm, "model_cost", costs),
        mock.patch.object(litellm, "module_level_client", FakeClient()),
        mock.patch.object(OllamaModelInfo, "get_model_info", dynamic_handler("ollama", driver)),
        mock.patch.object(
            LemonadeChatConfig, "get_model_info", dynamic_handler("lemonade", driver)
        ),
        mock.patch.object(litellm, "get_supported_openai_params", parameter_probe(driver)),
        mock.patch.object(litellm_utils, "_model_cost_lowercase_map", None),
        mock.patch.object(litellm_utils, "_model_cost_mutation_generation", 0),
        mock.patch.object(litellm, "vertex_llama3_models", set[str]()),
        mock.patch.object(litellm, "vertex_mistral_models", set[str]()),
        mock.patch.object(litellm, "vertex_ai_ai21_models", set[str]()),
        redirect_stdout(io.StringIO()),
    ):
        add_known_models(costs)
        set_rules(fixture_rules(fixture))
        litellm_utils._model_cost_lowercase_map = None  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        yield


def raised_category(error: Exception) -> str:
    if isinstance(error, ParameterQueryError):
        return "parameters"
    if isinstance(error, ModelNotMappedError):
        return "not_mapped"
    if isinstance(error, TypeError):
        if "multiple values" in str(error):
            return "duplicate_supported_openai_params"
        return "record"
    return "failed"


def execute_call(step: Object) -> Outcome:
    model = string_value(step.get("model"), "model")
    provider = optional_text(step, "custom_llm_provider")
    api_base = optional_text(step, "api_base")
    api_key = optional_text(step, "api_key")
    try:
        result = litellm_utils.get_model_info(model, provider, api_base, api_key)
    except Exception as error:  # noqa: BLE001 - every source exception is the raised outcome under test
        return {"result": {"raised": raised_category(error)}, "error": True}
    return {"result": json_value(result), "error": False}


def execute_helper_call(step: Object) -> Outcome:
    model = string_value(step.get("model"), "model")
    provider = optional_text(step, "custom_llm_provider")
    api_base = optional_text(step, "api_base")
    try:
        result = litellm_utils._cached_get_model_info_helper(model, provider, api_base)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    except Exception as error:  # noqa: BLE001 - every source exception is the raised outcome under test
        return {"result": {"raised": raised_category(error)}, "error": True}
    return {"result": json_value(cast("object", result)), "error": False}


def cache_info_json(info: CacheInfoView) -> Json:
    return {
        "hits": info.hits,
        "misses": info.misses,
        "maxsize": info.maxsize,
        "currsize": info.currsize,
    }


def caches_json() -> Json:
    full = litellm_utils._cached_get_model_info.cache_info()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    helper = litellm_utils._cached_get_model_info_helper.cache_info()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return {"full": cache_info_json(full), "helper": cache_info_json(helper)}


def observation(driver: Driver, op: str, outcome: Outcome, start: int) -> Json:
    calls = driver.calls[start:]
    caches = caches_json()
    if outcome["error"]:
        return {"op": op, "result": outcome["result"], "calls": calls, "caches": caches}
    result = object_value(outcome["result"], "result")
    same_as = next(
        (index for index, previous in enumerate(driver.records) if previous is result),
        None,
    )
    record = len(driver.records)
    driver.records.append(result)
    return {
        "op": op,
        "result": result,
        "record": record,
        "same_as": same_as,
        "calls": calls,
        "caches": caches,
    }


def call_step(driver: Driver, step: Object) -> Json:
    driver.script = object_value_or_none(step.get("parameters"), "parameters")
    start = len(driver.calls)
    return observation(driver, "call", execute_call(step), start)


def helper_call_step(driver: Driver, step: Object) -> Json:
    start = len(driver.calls)
    return observation(driver, "helper_call", execute_helper_call(step), start)


def mutate_step(driver: Driver, step: Object) -> Json:
    record = integer_value(step.get("record"), "record")
    field = string_value(step.get("field"), "field")
    target = driver.records[record]
    target[field] = step.get("value")
    return {
        "op": "mutate",
        "record": record,
        "field": field,
        "value": target.get(field),
    }


def invalidate_step() -> Json:
    _invalidate_model_cost_lowercase_map()
    return {
        "op": "invalidate",
        "generation": get_model_cost_mutation_generation(),
        "caches": caches_json(),
    }


def clear_step() -> Json:
    alias = litellm_utils.get_model_info.cache_clear  # pyright: ignore[reportAny, reportFunctionMemberAccess]
    cast("Callable[[], None]", alias)()
    return {"op": "clear", "caches": caches_json()}


def replace_step(driver: Driver, step: Object) -> Json:
    key = string_value(step.get("key"), "key")
    entry = object_value(step.get("entry"), "entry")
    driver.costs[key] = entry
    return {"op": "replace", "key": key}


def apply_step(driver: Driver, step: Object) -> Json:
    op = string_value(step.get("op"), "op")
    if op == "call":
        result = call_step(driver, step)
    elif op == "helper_call":
        result = helper_call_step(driver, step)
    elif op == "mutate":
        result = mutate_step(driver, step)
    elif op == "invalidate":
        result = invalidate_step()
    elif op == "clear":
        result = clear_step()
    elif op == "replace":
        result = replace_step(driver, step)
    else:
        raise ValueError(f"unknown step op {op}")
    return result


def run(data: dict[str, Json]) -> Json:
    fixture = object_value(data.get("fixture"), "fixture")
    driver = Driver(fixture)
    with installed(fixture, driver):
        install_caches(integer_value(fixture.get("cache_maxsize", 64), "cache_maxsize"))
        return [
            apply_step(driver, step) for step in object_list(data.get("steps"), "steps")
        ]
