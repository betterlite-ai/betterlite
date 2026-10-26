import os
from collections.abc import MutableMapping
from typing import TypeAlias, cast

from litellm.proxy.pass_through_endpoints import pass_through_endpoints
from litellm.proxy.utils import normalize_route_for_root_path

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
init_helpers = pass_through_endpoints.InitPassThroughEndpointHelpers


def run(input_value: dict[str, Json]) -> Json:
    if set(input_value) != {"root_path", "registrations", "queries"}:
        raise ValueError("registry input fields are root_path, registrations, and queries")
    root_path = input_value["root_path"]
    registrations = input_value["registrations"]
    queries = input_value["queries"]
    if (
        not isinstance(root_path, str)
        or not isinstance(registrations, list)
        or not isinstance(queries, list)
    ):
        raise ValueError("invalid registry input types")
    previous_root = os.environ.get("SERVER_ROOT_PATH")
    registry = pass_through_endpoints._registered_pass_through_routes  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    previous_registry = registry.copy()
    try:
        configure(root_path, registrations, cast(MutableMapping[str, object], registry))
        return cast(Json, query_registry(queries))
    finally:
        registry.clear()
        registry.update(previous_registry)
        if previous_root is None:
            os.environ.pop("SERVER_ROOT_PATH", None)
        else:
            os.environ["SERVER_ROOT_PATH"] = previous_root


def configure(
    root_path: str,
    registrations: list[Json],
    registry: MutableMapping[str, object],
) -> None:
    if root_path:
        os.environ["SERVER_ROOT_PATH"] = root_path
    else:
        os.environ.pop("SERVER_ROOT_PATH", None)
    registry.clear()
    for registration in registrations:
        if not isinstance(registration, dict):
            raise ValueError("registration must be an object")
        key = registration.get("key")
        if not isinstance(key, str):
            raise ValueError("registration key must be a string")
        registry[key] = {name: value for name, value in registration.items() if name != "key"}


def query_registry(queries: list[Json]) -> list[dict[str, Json]]:
    answers: list[dict[str, Json]] = []
    for query in queries:
        if not isinstance(query, dict) or not isinstance(query.get("route"), str):
            raise ValueError("query must contain a route")
        route_value = query["route"]
        if not isinstance(route_value, str):
            raise ValueError("query route must be a string")
        route = route_value
        method = query.get("method")
        if method is not None and not isinstance(method, str):
            raise ValueError("query method must be a string or null")
        normalized = normalize_route_for_root_path(route)
        comparison = normalized if normalized is not None else route
        metadata = init_helpers.get_registered_pass_through_route(route, method)
        registered = init_helpers.is_registered_pass_through_route(route)
        answers.append({"normalized": comparison, "registered": registered, "metadata": metadata})
    return answers
