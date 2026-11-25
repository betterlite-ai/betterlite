import asyncio
from typing import NotRequired, TypedDict, cast

from litellm.models.object_permission import LiteLLM_ObjectPermissionTable
from litellm.models.team import LiteLLM_TeamTableCachedObj
from litellm.proxy._types import ObjectPermissionDict
from litellm.proxy.management_helpers.object_permission_utils import (
    _extract_requested_mcp_access_groups,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L628-L638
    _extract_requested_mcp_server_ids,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L602-L625
    _extract_requested_mcp_toolsets,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L641-L651
    _extract_requested_search_tools,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L882-L891
    _extract_requested_vector_stores,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L842-L851
    _validate_requested_toolsets,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L797-L839
    validate_key_search_tools_against_team,
    validate_key_vector_stores_against_team,
)
from pydantic import JsonValue


class TeamInput(TypedDict):
    team_id: str
    mcp_toolsets: NotRequired[list[str] | None]
    search_tools: NotRequired[list[str] | None]


class Input(TypedDict):
    function: str
    permission: NotRequired[JsonValue]
    team: NotRequired[TeamInput | None]
    is_proxy_admin: NotRequired[bool]


def permission_of(input_value: Input) -> ObjectPermissionDict | None:
    raw = input_value.get("permission")
    if raw is None:
        return None
    return cast(ObjectPermissionDict, raw)  # the case supplies raw stored JSON: https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L602-L625


def team_of(input_value: Input) -> LiteLLM_TeamTableCachedObj | None:
    raw = input_value.get("team")
    if raw is None:
        return None
    permission = LiteLLM_ObjectPermissionTable(
        object_permission_id="team-permission",
        mcp_toolsets=raw.get("mcp_toolsets"),
        search_tools=raw.get("search_tools"),
    )
    return LiteLLM_TeamTableCachedObj(
        team_id=raw["team_id"],
        object_permission=permission,
    )


def admin_flag(input_value: Input) -> bool:
    value = input_value.get("is_proxy_admin")
    if value is None:
        return False
    return value


def run(input_value: Input) -> JsonValue | list[str]:  # noqa: PLR0911  # one branch per operation, mirroring https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L654-L941
    function = input_value["function"]
    if function == "extract_mcp_servers":
        return sorted(_extract_requested_mcp_server_ids(permission_of(input_value)))
    if function == "extract_mcp_access_groups":
        return sorted(_extract_requested_mcp_access_groups(permission_of(input_value)))
    if function == "extract_mcp_toolsets":
        return sorted(_extract_requested_mcp_toolsets(permission_of(input_value)))
    if function == "extract_vector_stores":
        return sorted(_extract_requested_vector_stores(permission_of(input_value)))
    if function == "extract_search_tools":
        return _extract_requested_search_tools(permission_of(input_value))
    if function == "validate_toolsets":
        _validate_requested_toolsets(
            requested_toolsets=_extract_requested_mcp_toolsets(permission_of(input_value)),
            team_obj=team_of(input_value),
            is_proxy_admin=admin_flag(input_value),
        )
        return None
    if function == "validate_vector_stores":
        asyncio.run(
            validate_key_vector_stores_against_team(
                object_permission=permission_of(input_value),
                team_obj=team_of(input_value),
                is_proxy_admin=admin_flag(input_value),
            )
        )
        return None
    if function == "validate_search_tools":
        asyncio.run(
            validate_key_search_tools_against_team(
                object_permission=permission_of(input_value),
                team_obj=team_of(input_value),
                is_proxy_admin=admin_flag(input_value),
            )
        )
        return None
    raise ValueError(f"unsupported resource_delegation function {function}")
