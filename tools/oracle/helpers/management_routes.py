from collections.abc import Sequence
from typing import TypeAlias, cast

from litellm.proxy._types import KeyManagementRoutes, LiteLLMRoutes

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
RouteTable: TypeAlias = Sequence[str] | tuple[str, ...]

GROUPS: dict[str, RouteTable] = {
    "master_key_only_routes": LiteLLMRoutes.master_key_only_routes.value,
    "key_management_routes": LiteLLMRoutes.key_management_routes.value,
    "team_service_account_key_routes": LiteLLMRoutes.team_service_account_key_routes.value,
    "management_routes": LiteLLMRoutes.management_routes.value,
    "spend_tracking_routes": LiteLLMRoutes.spend_tracking_routes.value,
    "global_spend_tracking_routes": LiteLLMRoutes.global_spend_tracking_routes.value,
    "internal_user_routes": LiteLLMRoutes.internal_user_routes.value,
    "internal_user_view_only_routes": LiteLLMRoutes.internal_user_view_only_routes.value,
    "org_admin_only_routes": LiteLLMRoutes.org_admin_only_routes.value,
    "admin_viewer_routes": LiteLLMRoutes.admin_viewer_routes.value,
    "org_admin_allowed_routes": LiteLLMRoutes.org_admin_allowed_routes.value,
}


def run(input_value: dict[str, Json]) -> list[Json]:
    if set(input_value) != {"group"}:
        raise ValueError("management routes input must contain only group")
    group = input_value["group"]
    if group == "key_enum":
        return cast(list[Json], [[route.name, route.value] for route in KeyManagementRoutes])
    if not isinstance(group, str) or group not in GROUPS:
        raise ValueError(f"unknown management route group: {group}")
    return cast(list[Json], list(GROUPS[group]))
