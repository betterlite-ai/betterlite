from typing import NotRequired, TypedDict, cast
from unittest.mock import patch

import litellm
import litellm.utils as litellm_utils
from litellm.utils import (
    PotentialModelNamesAndCustomLLMProvider,
    _cached_get_model_info,  # pyright: ignore[reportPrivateUsage]
    _cached_get_model_info_helper,  # pyright: ignore[reportPrivateUsage]
    _check_provider_match,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
    _get_model_cost_key,  # pyright: ignore[reportPrivateUsage]
    _get_model_info_from_model_cost,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
    _invalidate_model_cost_lowercase_map,  # pyright: ignore[reportPrivateUsage]
    _is_potential_model_name_in_model_cost,  # pyright: ignore[reportPrivateUsage]
    _strip_model_name,  # pyright: ignore[reportPrivateUsage]
    get_model_cost_mutation_generation,
)


class PotentialNames(TypedDict):
    split_model: str
    combined_model_name: str
    stripped_model_name: str
    combined_stripped_model_name: str
    provider_prefixed_model_name: str
    custom_llm_provider: str | None


class Step(TypedDict):
    op: str
    model: NotRequired[str]
    key: NotRequired[str]
    custom_llm_provider: NotRequired[str | None]
    litellm_provider: NotRequired[object]
    names: NotRequired[PotentialNames]


class Input(TypedDict):
    model_cost: list[str]
    steps: list[Step]


def _entry(position: int) -> dict[str, object]:
    return {"litellm_provider": "openai", "mode": "chat", "position": position}


def _cache_sizes() -> list[int]:
    return [
        _cached_get_model_info.cache_info().currsize,  # pyright: ignore[reportUnknownMemberType]
        _cached_get_model_info_helper.cache_info().currsize,  # pyright: ignore[reportUnknownMemberType]
    ]


def _mutate(step: Step, model_cost: dict[str, dict[str, object]], seed_model: str) -> object:
    op = step["op"]
    key = step.get("key", "")
    if op == "remove":
        model_cost.pop(key, None)
    elif op == "add":
        model_cost[key] = _entry(len(model_cost))
    elif op == "invalidate":
        _cached_get_model_info_helper(seed_model, None)
        _cached_get_model_info(seed_model)
        before = _cache_sizes()
        _invalidate_model_cost_lowercase_map()
        return {
            "before": before,
            "after": _cache_sizes(),
            "index": litellm_utils._model_cost_lowercase_map,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            "generation": get_model_cost_mutation_generation(),
        }
    return None


def _provider_match(step: Step) -> object:
    model_info: dict[str, object] = {}
    if "litellm_provider" in step:
        model_info["litellm_provider"] = step["litellm_provider"]
    try:
        return _check_provider_match(model_info, step.get("custom_llm_provider"))
    except AttributeError:
        return {"error": "raised"}


def _direct(key: str) -> object:
    try:
        entry = cast("dict[str, object]", _get_model_info_from_model_cost(key))
    except KeyError:
        return {"error": "raised"}
    return {"position": entry["position"]}


def _lookup(step: Step) -> object:
    op = step["op"]
    if op == "check_provider_match":
        return _provider_match(step)
    if op == "direct":
        return _direct(step.get("key", ""))
    names = cast("PotentialModelNamesAndCustomLLMProvider", step.get("names"))
    return _is_potential_model_name_in_model_cost(names)


def _query(step: Step) -> object:
    op = step["op"]
    if op == "strip_model_name":
        return _strip_model_name(step.get("model", ""), step.get("custom_llm_provider"))
    if op == "get_key":
        return _get_model_cost_key(step.get("key", ""))
    if op == "index":
        return litellm_utils._model_cost_lowercase_map  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    if op == "generation":
        return get_model_cost_mutation_generation()
    return _lookup(step)


def _apply(step: Step, model_cost: dict[str, dict[str, object]], seed_model: str) -> object:
    if step["op"] in {"remove", "add", "invalidate"}:
        return _mutate(step, model_cost, seed_model)
    return _query(step)


def run(data: Input) -> list[object]:
    model_cost = {key: _entry(position) for position, key in enumerate(data["model_cost"])}
    with (
        patch.object(litellm, "model_cost", model_cost),
        patch.object(litellm_utils, "_model_cost_mutation_generation", 0),
        patch.object(litellm_utils, "_model_cost_lowercase_map", None),
    ):
        return [_apply(step, model_cost, data["model_cost"][0]) for step in data["steps"]]
