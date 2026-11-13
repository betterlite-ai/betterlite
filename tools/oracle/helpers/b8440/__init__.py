from operator import attrgetter
from typing import Protocol, TypeAlias, cast

import litellm.router
import litellm.utils
from litellm.router import Router

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _SilentTargets(Protocol):
    def __call__(self, silent_model: object) -> tuple[str, ...]: ...


class _SilentKwargs(Protocol):
    def __call__(self, **kwargs: Json) -> dict[str, Json]: ...


class _ExcludedFilter(Protocol):
    def __call__(
        self,
        healthy_deployments: list[dict[str, object]],
        excluded_deployment_ids: list[str],
    ) -> list[dict[str, object]]: ...


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def array_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def silent_targets(input_value: dict[str, Json]) -> Json:
    targets = cast(_SilentTargets, attrgetter("_silent_experiment_targets")(litellm.router))
    return cast(Json, list(targets(input_value.get("value"))))


def silent_kwargs(input_value: dict[str, Json]) -> Json:
    request = object_value(input_value.get("request"), "request")
    kwargs = cast(_SilentKwargs, attrgetter("_get_silent_experiment_kwargs")(Router(model_list=[])))
    return cast(Json, kwargs(**request))


def drop_excluded(input_value: dict[str, Json]) -> Json:
    candidates = array_value(input_value.get("deployments"), "deployments")
    excluded_ids = array_value(input_value.get("excluded"), "excluded")
    filter_excluded = cast(
        _ExcludedFilter, attrgetter("get_excluded_filtered_deployments")(litellm.utils)
    )
    filtered = filter_excluded(
        cast(list[dict[str, object]], candidates),
        cast(list[str], excluded_ids),
    )
    return cast(Json, filtered)


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    match function:
        case "silent_targets":
            return silent_targets(input_value)
        case "silent_kwargs":
            return silent_kwargs(input_value)
        case "drop_excluded":
            return drop_excluded(input_value)
        case _:
            raise ValueError(f"unknown function {function}")
