import json
from contextlib import ExitStack
from typing import NotRequired, TypeAlias, TypedDict, cast
from unittest.mock import patch

import litellm
import litellm.utils as litellm_utils
from litellm.llms.fireworks_ai.chat.transformation import FireworksAIConfig
from litellm.llms.fireworks_ai.common_utils import resolve_fireworks_resource_name
from litellm.types.utils import ModelInfo
from litellm.utils import (
    _invalidate_model_cost_lowercase_map,  # pyright: ignore[reportPrivateUsage]
    get_model_cost_mutation_generation,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class CatalogModel(TypedDict):
    key: str
    model_info: dict[str, object]


class Step(TypedDict):
    op: str
    model: NotRequired[str]
    capability: NotRequired[str]
    key: NotRequired[str]
    model_info: NotRequired[dict[str, object]]
    catalog: NotRequired[list[CatalogModel]]


class Input(TypedDict):
    catalog: NotRequired[list[CatalogModel]]
    steps: NotRequired[list[Step]]


def _catalog(entries: list[CatalogModel]) -> dict[str, dict[str, object]]:
    return {entry["key"]: entry["model_info"] for entry in entries}


def _mutate(
    step: Step,
    catalogs: list[dict[str, dict[str, object]]],
    stack: ExitStack,
) -> object:
    op = step["op"]
    if op == "add":
        key = step.get("key", "")
        model_info = step.get("model_info", {})
        litellm.model_cost[key] = cast(ModelInfo, model_info)  # pyright: ignore[reportUnknownMemberType]
        return None
    if op == "remove":
        key = step.get("key", "")
        litellm.model_cost.pop(key, None)  # pyright: ignore[reportUnknownMemberType]
        return None
    if op == "invalidate":
        _invalidate_model_cost_lowercase_map()
        return get_model_cost_mutation_generation()
    replacement = _catalog(step.get("catalog", []))
    catalogs.append(replacement)
    stack.enter_context(patch.object(litellm, "model_cost", replacement))
    return None


def _query(step: Step, config: FireworksAIConfig) -> object:
    op = step["op"]
    model = step.get("model", "")
    if op == "resolve_resource":
        return resolve_fireworks_resource_name(model)
    if op == "cost_key":
        return config.get_model_cost_key(model)
    if op == "exact_capability":
        return config._get_model_cost_capability_exact(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            model=model, capability=step.get("capability", "")
        )
    if op == "capability":
        return config._get_model_cost_capability(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            model=model, capability=step.get("capability", "")
        )
    return cast(Json, json.loads(json.dumps(config.get_provider_info(model))))


def _apply(
    step: Step,
    config: FireworksAIConfig,
    catalogs: list[dict[str, dict[str, object]]],
    stack: ExitStack,
) -> object:
    if step["op"] in {"add", "remove", "invalidate", "replace_catalog"}:
        return _mutate(step, catalogs, stack)
    return _query(step, config)


def run(data: Input) -> list[object]:
    config = FireworksAIConfig()
    catalogs: list[dict[str, dict[str, object]]] = [_catalog(data.get("catalog", []))]
    stack = ExitStack()
    with stack:
        stack.enter_context(patch.object(litellm, "model_cost", catalogs[0]))
        stack.enter_context(patch.object(litellm_utils, "_model_cost_mutation_generation", 0))
        stack.enter_context(patch.object(FireworksAIConfig, "_fireworks_index_cache", None))
        return [_apply(step, config, catalogs, stack) for step in data.get("steps", [])]
