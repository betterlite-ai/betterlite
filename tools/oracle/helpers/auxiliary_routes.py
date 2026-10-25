from collections.abc import Sequence
from typing import TypeAlias, cast

from litellm.proxy._types import LiteLLMRoutes

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
RouteTable: TypeAlias = Sequence[str] | frozenset[str]

GROUPS: dict[str, RouteTable] = {
    "openai_route_names": LiteLLMRoutes.openai_route_names.value,
    "mapped_pass_through_routes": LiteLLMRoutes.mapped_pass_through_routes.value,
    "passthrough_routes_wildcard": LiteLLMRoutes.passthrough_routes_wildcard.value,
    "litellm_native_routes": LiteLLMRoutes.litellm_native_routes.value,
    "mcp_inference_routes": LiteLLMRoutes.mcp_inference_routes.value,
    "mcp_management_routes": LiteLLMRoutes.mcp_management_routes.value,
    "mcp_routes": LiteLLMRoutes.mcp_routes.value,
    "agent_inference_routes": LiteLLMRoutes.agent_inference_routes.value,
    "agent_management_routes": LiteLLMRoutes.agent_management_routes.value,
    "agent_routes": LiteLLMRoutes.agent_routes.value,
    "google_routes": LiteLLMRoutes.google_routes.value,
    "apply_guardrail_routes": LiteLLMRoutes.apply_guardrail_routes.value,
    "model_info_routes": LiteLLMRoutes.model_info_routes.value,
    "info_routes": LiteLLMRoutes.info_routes.value,
    "public_routes": LiteLLMRoutes.public_routes.value,
    "ui_routes": LiteLLMRoutes.ui_routes.value,
    "compliance_check_routes": LiteLLMRoutes.compliance_check_routes.value,
    "self_managed_routes": LiteLLMRoutes.self_managed_routes.value,
}


def run(input_value: dict[str, Json]) -> Json:
    if set(input_value) != {"group"}:
        raise ValueError("auxiliary routes input must contain only group")
    group = input_value["group"]
    if not isinstance(group, str) or group not in GROUPS:
        raise ValueError(f"unknown auxiliary route group: {group}")
    routes = GROUPS[group]
    result = sorted(routes) if isinstance(routes, frozenset) else list(routes)
    return cast(list[Json], result)
