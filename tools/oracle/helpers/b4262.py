from types import SimpleNamespace
from typing import TypeAlias, cast

from litellm.proxy.common_utils.get_routes import GetRoutes
from litellm.proxy.common_utils.get_routes import NamedEndpoint as LiteLLMNamedEndpoint
from starlette.routing import BaseRoute

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class NestedApp:
    def __init__(self, routes: list[SimpleNamespace] | None = None) -> None:
        self.routes = routes or []

    @property
    def __name__(self) -> str:
        return "NestedApp"


class BrokenEndpoint:
    @property
    def __name__(self) -> str:
        raise RuntimeError("endpoint name unavailable")


def route(value: dict[str, Json]) -> SimpleNamespace:
    children = value.get("app")
    return SimpleNamespace(
        path=value.get("path"), methods=value.get("methods"), name=value.get("name"),
        endpoint=value.get("endpoint"),
        app=NestedApp([route(cast(dict[str, Json], child)) for child in cast(list[Json], children)])
        if isinstance(children, list) else None,
    )


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
        return cast(Json, GetRoutes._mounted_sub_route_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            cast(str, value["mount_path"]), route(cast(dict[str, Json], value["route"]))
        ))
    if function == "safe_get_endpoint_name":
        name = value.get("endpoint_name")
        endpoint: object = BrokenEndpoint() if cast(bool, value.get("fail", False)) else (
            SimpleNamespace(__name__=name) if name is not None else NestedApp()
        )
        return cast(Json, GetRoutes._safe_get_endpoint_name(endpoint))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    raise ValueError("unknown function")
