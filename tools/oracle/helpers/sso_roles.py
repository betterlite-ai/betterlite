from typing import TypedDict

from litellm.proxy.management_endpoints.sso_helper_utils import (
    check_is_admin_only_access,
    has_admin_ui_access,
)
from litellm.proxy.management_endpoints.types import (
    get_litellm_user_role,
    is_valid_litellm_user_role,
)
from pydantic import JsonValue


class Input(TypedDict):
    claims: list[JsonValue]
    role_names: list[str]
    ui_roles: list[str]
    access_modes: list[str | dict[str, str]]


class Output(TypedDict):
    roles: list[str | None]
    valid: list[bool]
    admin_ui: list[bool]
    admin_only: list[bool]


def run(input_value: Input) -> Output:
    resolved = [get_litellm_user_role(claim) for claim in input_value["claims"]]
    roles = [role.value if role is not None else None for role in resolved]
    valid = [is_valid_litellm_user_role(name) for name in input_value["role_names"]]
    admin_ui = [has_admin_ui_access(role) for role in input_value["ui_roles"]]
    admin_only = [check_is_admin_only_access(mode) for mode in input_value["access_modes"]]
    return {"roles": roles, "valid": valid, "admin_ui": admin_ui, "admin_only": admin_only}
