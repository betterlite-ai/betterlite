import io
import json
import os
from collections.abc import Callable, Generator
from contextlib import contextmanager, redirect_stdout
from typing import NotRequired, TypeAlias, TypedDict, cast
from unittest import mock

import httpx
import litellm
import litellm.utils as litellm_utils
from litellm.exceptions import ModelNotMappedError
from litellm.litellm_core_utils import fallback_generalizations as engine
from litellm.llms.lemonade.chat.transformation import LemonadeChatConfig
from litellm.llms.ollama.common_utils import OllamaModelInfo
from litellm.utils import (  # pyright: ignore[reportPrivateUsage]
    _get_model_info_helper,  # pyright: ignore[reportPrivateUsage]
    _get_potential_model_names,  # pyright: ignore[reportPrivateUsage]
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
ModelCost: TypeAlias = dict[str, dict[str, object]]

set_rules = cast(
    Callable[[list[Json] | None], None], engine.set_fallback_generalizations
)
add_known_models = cast(Callable[[ModelCost], None], litellm.add_known_models)


class Query(TypedDict):
    model: str
    custom_llm_provider: NotRequired[str | None]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    dynamic: NotRequired[Object]
    hf: NotRequired[Object]


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


def optional_text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def object_list(value: Json, name: str) -> list[Object]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [object_value(item, name) for item in value]


def json_value(value: object) -> Json:
    return cast(Json, json.loads(json.dumps(value)))


def fixture_of(fields: Object) -> Object:
    return object_value(fields.get("fixture", {}), "fixture")


def model_cost_of(fixture: Object) -> ModelCost:
    return {
        name: cast(dict[str, object], entry)
        for name, entry in object_value(fixture.get("model_cost", {}), "model_cost").items()
    }


def environment_of(fixture: Object) -> dict[str, str]:
    environment = {
        name: string_value(entry, name)
        for name, entry in object_value(fixture.get("environment", {}), "environment").items()
    }
    environment["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"
    return environment


@contextmanager
def installed(fixture: Object) -> Generator[None]:
    costs = model_cost_of(fixture)
    llama_models: set[str] = set()
    mistral_models: set[str] = set()
    ai21_models: set[str] = set()
    with (
        mock.patch.object(litellm, "model_cost", costs),
        mock.patch.dict(os.environ, environment_of(fixture)),
        mock.patch.object(litellm, "vertex_llama3_models", llama_models),
        mock.patch.object(litellm, "vertex_mistral_models", mistral_models),
        mock.patch.object(litellm, "vertex_ai_ai21_models", ai21_models),
    ):
        add_known_models(costs)
        set_rules(cast(list[Json] | None, fixture.get("fallback_generalizations")))
        litellm_utils._model_cost_lowercase_map = None  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        yield


class FakeClient:
    def __init__(self, script: Object | None, requests: list[Json]) -> None:
        self._script = script
        self._requests = requests

    def get(self, url: str, timeout: float | None = None) -> httpx.Response:
        self._requests.append({"url": url, "timeout_seconds": timeout})
        script = self._script
        if script is None:
            raise httpx.ConnectError("no fixture transport")
        kind = string_value(script.get("kind"), "kind")
        if kind == "status":
            body = string_value(script.get("body"), "body")
            return httpx.Response(
                integer_value(script.get("status"), "status"),
                content=body.encode(),
                request=httpx.Request("GET", url),
            )
        if kind == "timeout":
            raise httpx.ReadTimeout("the fixture timed out")
        raise httpx.ConnectError("the fixture transport failed")


class DynamicScript:
    def __init__(self, script: Object | None, calls: list[Json]) -> None:
        self._script = script
        self._calls = calls

    def answer(self, adapter: str, model: str, api_base: str | None, api_key: str | None) -> Json:
        self._calls.append(
            {"adapter": adapter, "model": model, "api_base": api_base, "api_key": api_key}
        )
        script = self._script
        if script is None:
            return None
        kind = string_value(script.get("kind"), "kind")
        if kind == "value":
            return json_value(object_value(script.get("body"), "body"))
        if kind == "error":
            raise RuntimeError("dynamic model info failed")
        return None


def dynamic_handler(adapter: str, dynamic: DynamicScript) -> Callable[..., Json]:
    def handler(
        _self: object,
        model: str,
        api_base: str | None = None,
        api_key: str | None = None,
    ) -> Json:
        return dynamic.answer(adapter, model, api_base, api_key)

    return handler


def metadata_observation(query: Query, model: str, provider: str | None) -> Json:
    calls: list[Json] = []
    requests: list[Json] = []
    fields = cast(Object, query)
    dynamic = DynamicScript(query.get("dynamic"), calls)
    client = FakeClient(query.get("hf"), requests)
    api_base = optional_text(fields, "api_base")
    api_key = optional_text(fields, "api_key")
    with (
        mock.patch.object(OllamaModelInfo, "get_model_info", dynamic_handler("ollama", dynamic)),
        mock.patch.object(
            LemonadeChatConfig, "get_model_info", dynamic_handler("lemonade", dynamic)
        ),
        mock.patch.object(litellm, "module_level_client", client),
        redirect_stdout(io.StringIO()),
    ):
        answer: Object
        try:
            answer = {
                "result": json_value(
                    _get_model_info_helper(
                        model=model,
                        custom_llm_provider=provider,
                        api_base=api_base,
                        api_key=api_key,
                    )
                )
            }
        except ModelNotMappedError as error:
            answer = {"raised": "not_mapped", "message": str(error)}
        except Exception as error:  # noqa: BLE001 - every other source exception is the raised outcome under test
            answer = {"raised": "failed", "message": str(error)}
    answer["dynamic_calls"] = calls
    answer["hf_requests"] = requests
    return answer


def query_observation(scenario: str, fixture: Object, query: Object) -> Json:
    model = string_value(query.get("model"), "model")
    provider = optional_text(query, "custom_llm_provider")
    with installed(fixture):
        if scenario == "potential_names":
            # get_llm_provider prints its provider list to stdout when it cannot infer one
            with redirect_stdout(io.StringIO()):
                names = _get_potential_model_names(model=model, custom_llm_provider=provider)
            return {"result": json_value(names)}
        return metadata_observation(cast(Query, query), model, provider)


def run(input_value: dict[str, Json]) -> Json:
    scenario = string_value(input_value.get("scenario"), "scenario")
    fixture = fixture_of(input_value)
    queries = object_list(input_value.get("queries"), "queries")
    return [query_observation(scenario, fixture, query) for query in queries]
