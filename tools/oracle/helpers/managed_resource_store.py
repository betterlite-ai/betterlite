from typing import TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.common_utils.resource_ownership import (
    get_primary_resource_owner_scope,
    get_resource_owner_scopes,
    is_proxy_admin,
    user_can_access_resource_owner,
)


class Input(TypedDict):
    principal: dict[str, object] | None
    owner: str | None


def run(input_value: Input) -> dict[str, object]:
    raw = input_value["principal"]
    # These predicates receive resolved fields; validation would replace token with api_key.
    principal = UserAPIKeyAuth.model_construct(_fields_set=None, **raw) if raw is not None else None
    return {
        "is_proxy_admin": is_proxy_admin(principal),
        "scopes": get_resource_owner_scopes(principal),
        "primary": get_primary_resource_owner_scope(principal),
        "access": user_can_access_resource_owner(input_value["owner"], principal),
    }
