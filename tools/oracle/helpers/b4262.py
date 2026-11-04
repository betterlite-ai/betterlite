from types import SimpleNamespace
from typing import TypeAlias, cast

from litellm.proxy.common_utils.get_routes import GetRoutes
from litellm.proxy.common_utils.get_routes import NamedEndpoint as LiteLLMNamedEndpoint
from starlette.routing import BaseRoute

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class BrokenEndpoint:
    @property
    def __name__(self) -> str:
        raise RuntimeError("endpoint name unavailable")


def route(value: dict[str, Json]) -> SimpleNamespace:
    children = value.get("app")
    attributes: dict[str, object] = {}
    for field in ("path", "methods", "name"):
        if field in value:
            attributes[field] = value[field]
    if "endpoint" in value:
        endpoint_name = value["endpoint"]
        attributes["endpoint"] = (
            SimpleNamespace(__name__=endpoint_name)
            if isinstance(endpoint_name, str)
            else endpoint_name
        )
    if "app" in value:
        if isinstance(children, dict):
            app_record = cast(dict[str, Json], children)
            app_routes = [
                route(cast(dict[str, Json], child))
                for child in cast(list[Json], app_record.get("routes", []))
            ]
            app_type = type(
                cast(str, app_record["name"]),
                (),
                {"routes": app_routes},
            )
            attributes["app"] = app_type()
        else:
            attributes["app"] = children
    return SimpleNamespace(**attributes)


def run(value: dict[str, Json]) -> Json:
    function = value["function"]
    if function == "get_app_routes":
        name = value.get("endpoint_name")
        endpoint = SimpleNamespace(__name__=name)
        selected_route = route(cast(dict[str, Json], value["route"]))
        return cast(
            Json,
            GetRoutes.get_app_routes(
                cast(BaseRoute, selected_route), cast(LiteLLMNamedEndpoint, endpoint)
            ),
        )
    if function == "get_routes_for_mounted_app":
        selected_route = route(cast(dict[str, Json], value["route"]))
        return cast(Json, GetRoutes.get_routes_for_mounted_app(cast(BaseRoute, selected_route)))
    if function == "mounted_sub_route_info":
        result = GetRoutes._mounted_sub_route_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            cast(str, value["mount_path"]), route(cast(dict[str, Json], value["route"]))
        )
        return {"result": cast(Json, result)}
    if function == "safe_get_endpoint_name":
        name = value.get("endpoint_name")
        endpoint: object = BrokenEndpoint() if cast(bool, value.get("fail", False)) else (
            SimpleNamespace(__name__=name)
            if name is not None
            else type(cast(str, value.get("class_name", "NestedApp")), (), {})()
        )
        result = GetRoutes._safe_get_endpoint_name(endpoint)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        return {"result": cast(Json, result)}
    raise ValueError("unknown function")
