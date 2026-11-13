from collections.abc import Callable
from typing import TypeAlias, cast

import litellm
from litellm.router_utils.pattern_match_deployments import PatternMatchRouter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

AddPattern = Callable[[str, dict[str, object]], None]
MatchDeployments = Callable[[str, list[str] | None], list[dict[str, object]] | None]
GetPatterns = Callable[[str, str | None], list[dict[str, object]] | None]
GetDeploymentList = Callable[[str, str | None], list[dict[str, object]]]


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_string(value: Json, name: str) -> str | None:
    if value is None:
        return None
    return string_value(value, name)


def deployment(step: dict[str, Json]) -> dict[str, object]:
    model_id = optional_string(step.get("model_id"), "model_id")
    model_info: dict[str, object] = {}
    if model_id is not None:
        model_info["id"] = model_id
    model = string_value(step.get("model"), "model")
    return {"litellm_params": {"model": model}, "model_info": model_info}


def resolved(value: dict[str, object]) -> Json:
    params = value.get("litellm_params")
    if not isinstance(params, dict):
        raise TypeError("a routed deployment must have litellm_params")
    model: object = cast("dict[str, object]", params).get("model")
    if not isinstance(model, str):
        raise TypeError("a routed deployment must have a model")
    info = value.get("model_info")
    model_id: Json = None
    if isinstance(info, dict):
        identifier: object = cast("dict[str, object]", info).get("id")
        if isinstance(identifier, str):
            model_id = identifier
    result: dict[str, Json] = {"model": model, "model_id": model_id}
    return result


def matched(deployments: list[dict[str, object]] | None) -> Json:
    if deployments is None:
        return None
    return [resolved(value) for value in deployments]


def filtered(step: dict[str, Json]) -> list[str] | None:
    names = step.get("filtered_model_names")
    if names is None:
        return None
    if not isinstance(names, list):
        raise TypeError("filtered_model_names must be a list")
    return [string_value(name, "filtered_model_names") for name in names]


def run(input_value: dict[str, Json]) -> Json:
    steps = input_value.get("steps")
    if not isinstance(steps, list):
        raise TypeError("steps must be a list")
    # an unmapped model name makes get_llm_provider print to stdout, where the answer goes
    litellm.suppress_debug_info = True
    router = PatternMatchRouter()
    add_pattern = cast("AddPattern", router.add_pattern)
    remove_deployment = cast("Callable[[str], None]", router.remove_deployment)
    match_deployments = cast("MatchDeployments", router.route)
    get_patterns = cast("GetPatterns", router.get_pattern)
    get_deployments = cast("GetDeploymentList", router.get_deployments_by_pattern)
    routes: list[Json] = []
    for value in steps:
        step = object_value(value, "step")
        action = step.get("action")
        if action == "add":
            add_pattern(string_value(step.get("pattern"), "pattern"), deployment(step))
        elif action == "remove":
            remove_deployment(string_value(step.get("model_id"), "model_id"))
        elif action == "route":
            request = string_value(step.get("request"), "request")
            routes.append(matched(match_deployments(request, filtered(step))))
        elif action == "get_pattern":
            model = string_value(step.get("model"), "model")
            provider = optional_string(step.get("provider"), "provider")
            routes.append(matched(get_patterns(model, provider)))
        elif action == "get_deployments":
            model = string_value(step.get("model"), "model")
            provider = optional_string(step.get("provider"), "provider")
            routes.append(matched(get_deployments(model, provider)))
        else:
            raise ValueError(f"unknown wildcard step: {action}")
    return {"routes": routes}
