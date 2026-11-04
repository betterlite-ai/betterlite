from typing import NotRequired, TypedDict

from litellm.llms.base_llm.managed_resources import isolation
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from pydantic import BaseModel


class Principal(TypedDict):
    user_id: str | None
    team_id: str | None
    token: str | None
    api_key: str | None
    user_role: str


class Record(TypedDict):
    id: str
    created_by: str | None
    team_id: str | None


class ResourceItem(BaseModel):
    id: str
    created_by: str | None
    team_id: str | None


class Input(TypedDict):
    principal: Principal
    records: NotRequired[list[Record]]
    has_more: NotRequired[bool]


def run(input_value: Input) -> dict[str, object]:
    raw = input_value["principal"]
    roles = {
        "proxy_admin": LitellmUserRoles.PROXY_ADMIN,
        "proxy_admin_viewer": LitellmUserRoles.PROXY_ADMIN_VIEW_ONLY,
    }
    principal = UserAPIKeyAuth.model_construct(
        user_id=raw["user_id"],
        team_id=raw["team_id"],
        token=raw["token"],
        api_key=raw["api_key"],
        user_role=roles.get(raw["user_role"], LitellmUserRoles.INTERNAL_USER),
    )
    records = [ResourceItem(**row) for row in input_value.get("records", [])]
    page = isolation.build_list_page(records, input_value.get("has_more", False))
    page["data"] = [row.model_dump() for row in records]
    return {
        "owner": isolation.resolve_resource_owner_id(principal),
        "filter": isolation.build_owner_filter(principal),
        "access": [
            isolation.can_access_resource(principal, row.created_by, row.team_id)
            for row in records
        ],
        "page": page,
    }
