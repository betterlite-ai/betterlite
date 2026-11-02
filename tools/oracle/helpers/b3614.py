from collections.abc import Mapping
from types import MappingProxyType
from typing import TYPE_CHECKING, TypeAlias, cast

from fastapi import FastAPI
from litellm.proxy._types import LiteLLMRoutes
from litellm.proxy.pass_through_endpoints import pass_through_endpoints

if TYPE_CHECKING:
    from starlette.routing import BaseRoute

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
SafeRouteAdder = pass_through_endpoints.SafeRouteAdder
InitPassThroughEndpointHelpers = pass_through_endpoints.InitPassThroughEndpointHelpers


async def _dummy_endpoint() -> dict[str, bool]:
    return {"ok": True}


def collision(fields: dict[str, Json]) -> Json:
    existing_routes = fields["existing_routes"]
    if not isinstance(existing_routes, list):
        raise ValueError("existing_routes must be a list")
    app = FastAPI(docs_url=None, redoc_url=None, openapi_url=None)
    labels: dict[int, str] = {}
    lazy_owned_routes: list[BaseRoute] = []
    for index, raw_route in enumerate(existing_routes):
        if not isinstance(raw_route, dict):
            raise ValueError("existing route must be an object")
        path = raw_route["path"]
        methods = raw_route["methods"]
        if not isinstance(path, str) or not isinstance(methods, list):
            raise ValueError("existing route path and methods have the wrong type")
        app.add_api_route(path=path, endpoint=_dummy_endpoint, methods=cast(list[str], methods))
        route = app.router.routes[-1]
        labels[id(route)] = f"existing[{index}]"
        if raw_route.get("lazy_owned"):
            lazy_owned_routes.append(route)
    if lazy_owned_routes:
        app.state.lazy_routes = MappingProxyType({"fake_lazy_feature": tuple(lazy_owned_routes)})
    new_path = fields["new_path"]
    new_methods = fields["new_methods"]
    if not isinstance(new_path, str) or not isinstance(new_methods, list):
        raise ValueError("new_path and new_methods have the wrong type")
    added = SafeRouteAdder.add_api_route_if_not_exists(  # pyright: ignore[reportUnknownMemberType]
        app=app,
        path=new_path,
        endpoint=_dummy_endpoint,
        methods=cast(list[str], new_methods),
    )
    route_order = [labels.get(id(route), "new") for route in app.router.routes]
    return cast(Json, {"added": added, "route_order": route_order})


def remove(fields: dict[str, Json]) -> Json:
    registrations = fields["registrations"]
    openai_routes_input = fields["openai_routes"]
    endpoint_id = fields["endpoint_id"]
    if (
        not isinstance(registrations, list)
        or not isinstance(openai_routes_input, list)
        or not isinstance(endpoint_id, str)
    ):
        raise ValueError("remove input fields have the wrong type")
    registry = pass_through_endpoints._registered_pass_through_routes  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    previous_registry = registry.copy()
    previous_openai_routes = list(LiteLLMRoutes.openai_routes.value)
    try:
        registry.clear()
        for raw_registration in registrations:
            if not isinstance(raw_registration, dict):
                raise ValueError("registration must be an object")
            key = raw_registration["key"]
            if not isinstance(key, str):
                raise ValueError("registration key must be a string")
            registry[key] = cast(
                dict[str, str | bool | list[str] | Mapping[str, object]],
                {
                    "endpoint_id": raw_registration["endpoint_id"],
                    "path": raw_registration["path"],
                    "type": raw_registration["route_type"],
                    "methods": raw_registration.get("methods", []),
                    "auth": raw_registration.get("auth"),
                },
            )
        LiteLLMRoutes.openai_routes.value[:] = cast(list[str], openai_routes_input)
        InitPassThroughEndpointHelpers.remove_endpoint_routes(endpoint_id)
        registered_keys = InitPassThroughEndpointHelpers.get_all_registered_pass_through_routes()
        return cast(
            Json,
            {
                "registered_keys": registered_keys,
                "openai_routes": list(LiteLLMRoutes.openai_routes.value),
            },
        )
    finally:
        registry.clear()
        registry.update(previous_registry)
        LiteLLMRoutes.openai_routes.value[:] = previous_openai_routes


def run(input_value: dict[str, Json]) -> Json:
    [(operation, request)] = input_value.items()
    if not isinstance(request, dict):
        raise ValueError("request must be an object")
    fields = cast(dict[str, Json], request)
    match operation:
        case "collision":
            return collision(fields)
        case "remove":
            return remove(fields)
        case _:
            raise ValueError(f"unsupported operation {operation}")
