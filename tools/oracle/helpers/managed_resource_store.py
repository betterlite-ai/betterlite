from typing import NotRequired, TypedDict

from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.common_utils.resource_ownership import (
    get_primary_resource_owner_scope,
    get_resource_owner_scopes,
    is_proxy_admin,
    user_can_access_resource_owner,
)


class Query(TypedDict):
    principal: dict[str, object] | None
    owner: str | None


class Input(Query):
    probes: NotRequired[list[Query]]


def evaluate(query: Query) -> dict[str, object]:
    raw = query["principal"]
    # These predicates receive resolved fields; validation would replace token with api_key.
    principal = UserAPIKeyAuth.model_construct(_fields_set=None, **raw) if raw is not None else None
    if principal is not None and raw is not None and raw.get("role_as_enum"):
        principal.user_role = LitellmUserRoles(raw["user_role"])
    return {
        "is_proxy_admin": is_proxy_admin(principal),
        "scopes": get_resource_owner_scopes(principal),
        "primary": get_primary_resource_owner_scope(principal),
        "access": user_can_access_resource_owner(query["owner"], principal),
    }


def run(input_value: Input) -> dict[str, object]:
    return {
        "result": evaluate(input_value),
        "probes": [evaluate(query) for query in input_value.get("probes", [])],
    }
