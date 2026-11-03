from typing import TypeAlias, cast

from litellm.proxy.auth.auth_checks import get_actual_routes  # pyright: ignore[reportUnknownVariableType]
from litellm.proxy.auth.route_checks import RouteChecks

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:  # noqa: C901, PLR0911
    function = input_value["function"]
    route = input_value.get("route")
    pattern = input_value.get("pattern")
    if function == "is_llm_api_route":
        return RouteChecks.is_llm_api_route(cast(str, route))
    if function == "is_azure_openai_route":
        return RouteChecks._is_azure_openai_route(cast(str, route))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    if function == "is_management_route":
        return isinstance(route, str) and RouteChecks.is_management_route(route)
    if function == "is_info_route":
        return isinstance(route, str) and RouteChecks.is_info_route(route)
    if function == "is_mcp_discovery_route":
        request = (
            None
            if input_value.get("method") is None
            else _Request(cast(str, input_value["method"]))
        )
        return isinstance(route, str) and RouteChecks._is_get_mcp_server_discovery_route(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            route, request  # pyright: ignore[reportArgumentType]
        )
    if function == "route_matches_pattern":
        return (
            isinstance(route, str)
            and isinstance(pattern, str)
            and RouteChecks._route_matches_pattern(route, pattern)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        )
    if function == "is_wildcard_pattern":
        return isinstance(pattern, str) and RouteChecks._is_wildcard_pattern(pattern)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    if function == "route_matches_wildcard_pattern":
        return (
            isinstance(route, str)
            and isinstance(pattern, str)
            and RouteChecks.route_matches_wildcard_pattern(route, pattern)
        )
    if function == "route_matches_allowed_route":
        allowed = input_value.get("allowed_route")
        return (
            isinstance(route, str)
            and isinstance(allowed, str)
            and RouteChecks._route_matches_allowed_route(route, allowed)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        )
    if function == "check_route_access":
        allowed = input_value.get("allowed_routes")
        return (
            isinstance(route, str)
            and isinstance(allowed, list)
            and RouteChecks.check_route_access(route, cast(list[str], allowed))
        )
    if function == "get_actual_routes":
        return cast(
            list[Json],
            get_actual_routes(cast(list[str], input_value.get("routes", []))),  # pyright: ignore[reportUnknownVariableType]
        )
    if function == "get_request_method":
        request = (
            None
            if input_value.get("method") is None
            else _Request(cast(str, input_value["method"]))
        )
        return RouteChecks._get_request_method(request)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportArgumentType]
    raise ValueError(f"unknown route_match function: {function}")


class _Request:
    def __init__(self, method: str) -> None:
        self.method = method
