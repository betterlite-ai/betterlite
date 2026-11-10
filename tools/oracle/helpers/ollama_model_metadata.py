import os
from collections.abc import Callable
from decimal import Decimal
from functools import partial
from typing import TypeAlias, TypedDict, cast
from unittest import mock

import httpx
import litellm
from litellm.llms.ollama.common_utils import OllamaModelInfo
from litellm.secret_managers import main as secret_managers

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


class Recorded(TypedDict):
    url: str
    authorization: str | None
    name: str


class FakeClient:
    def __init__(self, scenario: Object) -> None:
        self._scenario = scenario
        self.calls: list[Recorded] = []

    def post(
        self,
        url: str,
        json: Json,
        headers: dict[str, str],
    ) -> httpx.Response:
        body = object_value(json, "body")
        self.calls.append(
            {
                "url": url,
                "authorization": headers.get("Authorization"),
                "name": string_value(body.get("name"), "name"),
            }
        )
        return response_for(self._scenario)


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


def string_list(value: Json, name: str) -> list[str]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [string_value(item, name) for item in value]


def environment_map(fixture: Object) -> dict[str, str]:
    raw = fixture.get("env", {})
    environment = object_value(raw, "env")
    variables = {name: string_value(value, name) for name, value in environment.items()}
    variables["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"
    return variables


def catalog_map(fixture: Object) -> dict[str, dict[str, object]]:
    return {key: {} for key in string_list(fixture.get("catalog", []), "catalog")}


def secret_lookup(
    secret_manager_api_key: str | None,
    name: str,
    default_value: str | None = None,
) -> str | None:
    del default_value
    if name == "OLLAMA_API_KEY" and secret_manager_api_key is not None:
        return secret_manager_api_key
    return os.environ.get(name)


def response_for(scenario: Object) -> httpx.Response:
    kind = string_value(scenario.get("kind"), "kind")
    if kind == "failure":
        raise httpx.ConnectError("fixture transport failure")
    body = string_value(scenario.get("body"), "body")
    status = 200 if kind == "success" else integer_value(scenario.get("status"), "status")
    request = httpx.Request("POST", "http://fixture.invalid/api/show")
    return httpx.Response(status_code=status, text=body, request=request)


def single_call(client: FakeClient) -> Recorded:
    if len(client.calls) != 1:
        raise AssertionError(f"expected one request, recorded {len(client.calls)}")
    return client.calls[0]


def recorded_json(recorded: Recorded) -> Json:
    return {
        "url": recorded["url"],
        "authorization": recorded["authorization"],
        "name": recorded["name"],
    }


def decimal_text(value: float) -> str:
    return format(Decimal(repr(value)).normalize(), "f")


def projected_info(info: dict[str, object]) -> Json:
    projected: dict[str, Json] = {
        "key": cast(str, info["key"]),
        "litellm_provider": cast(str, info["litellm_provider"]),
        "mode": cast(str, info["mode"]),
        "input_cost_per_token": decimal_text(cast(float, info["input_cost_per_token"])),
        "output_cost_per_token": decimal_text(cast(float, info["output_cost_per_token"])),
        "max_tokens": cast(int | None, info["max_tokens"]),
        "max_input_tokens": cast(int | None, info["max_input_tokens"]),
        "max_output_tokens": cast(int | None, info["max_output_tokens"]),
    }
    if "supports_function_calling" in info:
        projected["supports_function_calling"] = cast(bool, info["supports_function_calling"])
    return projected


def run_origin_auth(fields: Object) -> Json:
    model = string_value(fields.get("model"), "model")
    groups = object_list(fields.get("groups"), "groups")
    observed_groups: list[Json] = []
    for group in groups:
        fixture = object_value(group.get("fixture"), "fixture")
        queries = object_list(group.get("queries"), "queries")
        globals_value = object_value(fixture.get("globals", {}), "globals")
        secret_manager_api_key = optional_text(fixture, "secret_manager_api_key")
        observations: list[Json] = []
        with (
            mock.patch.dict(os.environ, environment_map(fixture), clear=True),
            mock.patch.object(litellm, "api_key", optional_text(globals_value, "api_key")),
            mock.patch.object(litellm, "openai_key", optional_text(globals_value, "openai_key")),
            mock.patch.object(
                secret_managers,
                "get_secret_str",
                partial(secret_lookup, secret_manager_api_key),
            ),
            mock.patch.object(litellm, "model_cost", catalog_map(fixture)),
        ):
            for query in queries:
                client = FakeClient({"kind": "success", "body": "{}"})
                with mock.patch.object(litellm, "module_level_client", client):
                    _ = OllamaModelInfo().get_runtime_model_info(
                        model=model,
                        api_base=optional_text(query, "api_base"),
                        api_key=optional_text(query, "api_key"),
                    )
                observations.append(recorded_json(single_call(client)))
        observed_groups.append(observations)
    return observed_groups


def run_static_catalog(fields: Object) -> Json:
    fixture = object_value(fields.get("fixture"), "fixture")
    models = string_list(fields.get("models"), "models")
    observations: list[Json] = []
    globals_value = object_value(fixture.get("globals", {}), "globals")
    secret_manager_api_key = optional_text(fixture, "secret_manager_api_key")
    with (
        mock.patch.dict(os.environ, environment_map(fixture), clear=True),
        mock.patch.object(litellm, "api_key", optional_text(globals_value, "api_key")),
        mock.patch.object(litellm, "openai_key", optional_text(globals_value, "openai_key")),
        mock.patch.object(
            secret_managers,
            "get_secret_str",
            partial(secret_lookup, secret_manager_api_key),
        ),
        mock.patch.object(litellm, "model_cost", catalog_map(fixture)),
    ):
        for model in models:
            client = FakeClient({"kind": "success", "body": "{}"})
            with mock.patch.object(litellm, "module_level_client", client):
                info = OllamaModelInfo().get_model_info(model=model, api_base=None, api_key=None)
            observations.append(
                {
                    "result": None
                    if info is None
                    else projected_info(cast("dict[str, object]", info)),
                    "calls": len(client.calls),
                }
            )
    return observations


def run_model_query(fields: Object) -> Json:
    fixture = object_value(fields.get("fixture"), "fixture")
    model = string_value(fields.get("model"), "model")
    scenarios = object_list(fields.get("scenarios"), "scenarios")
    api_base = optional_text(fields, "api_base")
    api_key = optional_text(fields, "api_key")
    observations: list[Json] = []
    globals_value = object_value(fixture.get("globals", {}), "globals")
    secret_manager_api_key = optional_text(fixture, "secret_manager_api_key")
    with (
        mock.patch.dict(os.environ, environment_map(fixture), clear=True),
        mock.patch.object(litellm, "api_key", optional_text(globals_value, "api_key")),
        mock.patch.object(litellm, "openai_key", optional_text(globals_value, "openai_key")),
        mock.patch.object(
            secret_managers,
            "get_secret_str",
            partial(secret_lookup, secret_manager_api_key),
        ),
        mock.patch.object(litellm, "model_cost", catalog_map(fixture)),
    ):
        for scenario in scenarios:
            client = FakeClient(scenario)
            with mock.patch.object(litellm, "module_level_client", client):
                info = OllamaModelInfo().get_runtime_model_info(
                    model=model, api_base=api_base, api_key=api_key
                )
            recorded = single_call(client)
            observations.append(
                {
                    **cast("dict[str, Json]", recorded_json(recorded)),
                    "info": projected_info(cast("dict[str, object]", info)),
                }
            )
    return observations


def run_model_fields(fields: Object) -> Json:
    fixture = object_value(fields.get("fixture"), "fixture")
    model = string_value(fields.get("model"), "model")
    bodies = string_list(fields.get("bodies"), "bodies")
    observations: list[Json] = []
    globals_value = object_value(fixture.get("globals", {}), "globals")
    secret_manager_api_key = optional_text(fixture, "secret_manager_api_key")
    with (
        mock.patch.dict(os.environ, environment_map(fixture), clear=True),
        mock.patch.object(litellm, "api_key", optional_text(globals_value, "api_key")),
        mock.patch.object(litellm, "openai_key", optional_text(globals_value, "openai_key")),
        mock.patch.object(
            secret_managers,
            "get_secret_str",
            partial(secret_lookup, secret_manager_api_key),
        ),
        mock.patch.object(litellm, "model_cost", catalog_map(fixture)),
    ):
        for body in bodies:
            client = FakeClient({"kind": "success", "body": body})
            info: dict[str, object] | None
            with mock.patch.object(litellm, "module_level_client", client):
                try:
                    info = cast(
                        "dict[str, object]",
                        OllamaModelInfo().get_runtime_model_info(
                            model=model, api_base=None, api_key=None
                        ),
                    )
                except Exception:  # noqa: BLE001 - a failed reply decode is the outcome under test
                    info = None
            observations.append({"info": None if info is None else projected_info(info)})
    return observations


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "configured_origin_auth": run_origin_auth,
    "static_catalog_first": run_static_catalog,
    "runtime_model_query": run_model_query,
    "runtime_model_fields": run_model_fields,
}


def run(argument: Json) -> Json:
    fields = object_value(argument, "input")
    if len(fields) != 1:
        raise ValueError("expected one ollama_model_metadata variant")
    name, value = next(iter(fields.items()))
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown ollama_model_metadata case: {name}")
    return handler(object_value(value, name))
