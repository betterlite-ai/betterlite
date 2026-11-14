from typing import NotRequired, TypedDict

from litellm.types.proxy.management_endpoints.scim_v2 import (
    LiteLLM_UserScimMetadata,
    SCIMMultiValuedAttribute,
    SCIMUser,
    SCIMUserGroup,
)
from pydantic import JsonValue, TypeAdapter

# the two roles _resolve_scim_user_role hands back
ADMIN_ROLE = "proxy_admin"
VIEWER_ROLE = "internal_user_viewer"
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
ATTRIBUTES: TypeAdapter[list[SCIMMultiValuedAttribute]] = TypeAdapter(
    list[SCIMMultiValuedAttribute]
)


class Input(TypedDict):
    user: dict[str, JsonValue]
    admin_group: NotRequired[str | None]
    default_role: NotRequired[str | None]
    active: NotRequired[bool | None]


INPUT: TypeAdapter[Input] = TypeAdapter(Input)


def role_from_groups(groups: list[SCIMUserGroup], admin_group: str, default_role: str) -> str:
    for group in groups:
        if admin_group in (group.value, group.display):
            return ADMIN_ROLE
    return default_role


def user_role(
    groups: list[SCIMUserGroup] | None,
    admin_group: str | None,
    default_role: str | None,
) -> str | None:
    if admin_group is None:
        return None
    fallback = VIEWER_ROLE if default_role is None else default_role
    if groups is None:
        return fallback
    return role_from_groups(groups, admin_group, fallback)


def name_block(user: SCIMUser) -> JsonValue:
    name = user.name
    given_name = name.givenName if name is not None else None
    family_name = name.familyName if name is not None else None
    record = LiteLLM_UserScimMetadata(givenName=given_name, familyName=family_name)
    return JSON.validate_json(record.model_dump_json())


def scim_metadata(user: SCIMUser, *, active: bool | None) -> dict[str, JsonValue]:
    metadata: dict[str, JsonValue] = {"scim_metadata": name_block(user)}
    if active is not None:
        metadata["scim_active"] = active
    enterprise = user.enterprise_user
    if enterprise is not None:
        metadata["scim_enterprise"] = JSON.validate_json(
            enterprise.model_dump_json(by_alias=True, exclude_none=True)
        )
    entitlements = user.entitlements
    if entitlements is not None:
        metadata["scim_entitlements"] = JSON.validate_json(
            ATTRIBUTES.dump_json(entitlements, exclude_none=True)
        )
    roles = user.roles
    if roles is not None:
        metadata["scim_roles"] = JSON.validate_json(ATTRIBUTES.dump_json(roles, exclude_none=True))
    return metadata


def run(input_value: JsonValue) -> JsonValue:
    record = INPUT.validate_python(input_value)
    user = SCIMUser.model_validate(record["user"])
    name = user.name
    given_name = name.givenName if name is not None else None
    emails = user.emails
    groups = user.groups
    teams: list[str] = []
    if groups is not None:
        teams = [group.value for group in groups]
    email: str | None = None
    if emails:
        email = str(emails[0].value)
    alias: str | None = None
    if given_name:
        alias = given_name
    answer: dict[str, JsonValue] = {
        "user_email": email,
        "user_alias": alias,
        "sso_user_id": user.externalId,
        "teams": JSON.validate_python(teams),
        "active": user.active,
        "metadata": scim_metadata(user, active=record.get("active")),
        "user_role": user_role(groups, record.get("admin_group"), record.get("default_role")),
    }
    return answer
