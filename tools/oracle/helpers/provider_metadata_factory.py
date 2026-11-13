import json
from collections.abc import Callable
from typing import NotRequired, TypeAlias, TypedDict, cast
from unittest import mock

import litellm
from litellm.llms.azure_ai.common_utils import AzureFoundryModelInfo
from litellm.llms.base_llm.base_utils import BaseLLMModelInfo
from litellm.llms.fireworks_ai.chat.transformation import FireworksAIConfig
from litellm.llms.lemonade.chat.transformation import LemonadeChatConfig
from litellm.llms.ollama.common_utils import OllamaModelInfo
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager, get_provider_info

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


class SelectionQuery(TypedDict):
    providers: list[str]
    model: NotRequired[str]


class SelectionInput(TypedDict):
    queries: list[SelectionQuery]


class CatalogModel(TypedDict):
    key: str
    model_info: dict[str, object]


class ProjectionStep(TypedDict):
    op: str
    model: str
    provider: NotRequired[str]


class ProjectionInput(TypedDict):
    catalog: NotRequired[list[CatalogModel]]
    steps: list[ProjectionStep]


class ConstructorsInput(TypedDict):
    fireworks: NotRequired[list[Object]]
    lemonade: NotRequired[list[Object]]
    foundry: NotRequired[list[str | None]]


class DynamicQuery(TypedDict):
    provider: str
    model: str


class DynamicInput(TypedDict):
    queries: list[DynamicQuery]


class FakeClient:
    def __init__(self) -> None:
        self.calls: list[str] = []

    def post(self, *_args: object, **_kwargs: object) -> object:
        self.calls.append("post")
        raise AssertionError("selection must not query the provider")

    def get(self, *_args: object, **_kwargs: object) -> object:
        self.calls.append("get")
        raise AssertionError("selection must not query the provider")


def object_value(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def object_list(value: Json, name: str) -> list[Object]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [object_value(item, name) for item in value]


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def string_list(value: Json, name: str) -> list[str]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [string_value(item, name) for item in value]


def optional_string_list(value: Json, name: str) -> list[str | None]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [None if item is None else string_value(item, name) for item in value]


def json_value(value: object) -> Json:
    return cast(Json, json.loads(json.dumps(value)))


def catalog_map(value: Json) -> dict[str, Object]:
    return {
        string_value(entry.get("key"), "key"): object_value(
            entry.get("model_info"), "model_info"
        )
        for entry in object_list(value, "catalog")
    }


def configure_fireworks(args: Object) -> Json:
    constructor = cast(Callable[..., FireworksAIConfig], FireworksAIConfig)
    return json_value(constructor(**args).get_config())


def configure_lemonade(args: Object) -> Json:
    constructor = cast(Callable[..., LemonadeChatConfig], LemonadeChatConfig)
    return json_value(constructor(**args).get_config())


def foundry_entry(model: str | None) -> Json:
    config = AzureFoundryModelInfo(model=model)
    return {"model": config._model}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def selection_entry(provider: str, config: BaseLLMModelInfo | None) -> Json:
    entry: dict[str, Json] = {"class": None if config is None else type(config).__name__}
    if provider == "azure_ai":
        foundry = None if config is None else cast(AzureFoundryModelInfo, config)
        entry["model"] = (
            None
            if foundry is None
            else foundry._model  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        )
    return entry


def run_metadata_adapter_selection(fields: Object) -> Json:
    observations: list[Json] = []
    for query in object_list(fields.get("queries"), "queries"):
        model = optional_text(query, "model")
        chosen: dict[str, Json] = {}
        for provider in string_list(query.get("providers"), "providers"):
            config = ProviderConfigManager.get_provider_model_info(
                model=model, provider=LlmProviders(provider)
            )
            chosen[provider] = selection_entry(provider, config)
        observations.append(chosen)
    return observations


def projection_step(step: Object) -> Json:
    op = string_value(step.get("op"), "op")
    model = string_value(step.get("model"), "model")
    provider = optional_text(step, "provider")
    if op == "free":
        return json_value(get_provider_info(model=model, custom_llm_provider=provider))
    if provider is None:
        raise ValueError(f"projection op {op} needs a provider")
    config = ProviderConfigManager.get_provider_model_info(
        model=model, provider=LlmProviders(provider)
    )
    if config is None:
        return None
    if op == "adapter_provider_info":
        return json_value(config.get_provider_info(model=model))
    if op == "adapter_cost_key":
        return json_value(config.get_model_cost_key(model))
    raise ValueError(f"unknown provider info projection op: {op}")


def run_provider_info_projection(fields: Object) -> Json:
    catalog = catalog_map(fields.get("catalog", []))
    steps = object_list(fields.get("steps"), "steps")
    with mock.patch.object(litellm, "model_cost", catalog):
        return [projection_step(step) for step in steps]


def run_metadata_constructors(fields: Object) -> Json:
    fireworks = [
        configure_fireworks(args)
        for args in object_list(fields.get("fireworks", []), "fireworks")
    ]
    lemonade = [
        configure_lemonade(args)
        for args in object_list(fields.get("lemonade", []), "lemonade")
    ]
    foundry = [
        foundry_entry(model)
        for model in optional_string_list(fields.get("foundry", []), "foundry")
    ]
    return {"fireworks": fireworks, "lemonade": lemonade, "foundry": foundry}


def dynamic_operation(config: BaseLLMModelInfo | None) -> str | None:
    if isinstance(config, OllamaModelInfo):
        return "ollama"
    if isinstance(config, LemonadeChatConfig):
        return "lemonade"
    return None


def run_dynamic_metadata_separation(fields: Object) -> Json:
    observations: list[Json] = []
    for query in object_list(fields.get("queries"), "queries"):
        provider = string_value(query.get("provider"), "provider")
        model = string_value(query.get("model"), "model")
        client = FakeClient()
        with mock.patch.object(litellm, "module_level_client", client):
            config = ProviderConfigManager.get_provider_model_info(
                model=model, provider=LlmProviders(provider)
            )
        observations.append(
            {
                "provider": provider,
                "class": None if config is None else type(config).__name__,
                "dynamic": dynamic_operation(config),
                "calls": len(client.calls),
            }
        )
    return observations


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "metadata_adapter_selection": run_metadata_adapter_selection,
    "provider_info_projection": run_provider_info_projection,
    "metadata_constructors": run_metadata_constructors,
    "dynamic_metadata_separation": run_dynamic_metadata_separation,
}


def run(argument: Json) -> Json:
    fields = object_value(argument, "input")
    if len(fields) != 1:
        raise ValueError("expected one provider_metadata_factory variant")
    name, value = next(iter(fields.items()))
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown provider_metadata_factory case: {name}")
    return handler(object_value(value, name))
