from typing import Literal

from betterlite.types.utils import JsonValue, ResponseObject, field_validator


class CustomerResponse(ResponseObject):
    user_id: str
    blocked: bool
    alias: str | None = None
    spend: float = 0.0
    allowed_model_region: Literal["eu", "us"] | None = None
    default_model: str | None = None
    budget_id: str | None = None
    litellm_budget_table: dict[str, JsonValue] | None = None
    object_permission_id: str | None = None
    object_permission: dict[str, JsonValue] | None = None

    @field_validator("spend", mode="before")
    @classmethod
    def default_spend(cls, value: JsonValue) -> JsonValue:
        if value is None:
            return 0.0
        return value


class BlockUsersResponse(ResponseObject):
    blocked_users: list[dict[str, JsonValue]]


class UnblockUsersResponse(ResponseObject):
    blocked_users: list[str]


class DeleteCustomersResponse(ResponseObject):
    deleted_customers: int
    message: str
