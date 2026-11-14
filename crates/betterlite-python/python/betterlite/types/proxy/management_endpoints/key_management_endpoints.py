from datetime import datetime
from typing import ClassVar, Literal, Self, TypeAlias, TypedDict

from pydantic import ConfigDict
from typing_extensions import ReadOnly

from betterlite.types.proxy.management_endpoints.internal_user_endpoints import (
    InsensitiveContains,
)
from betterlite.types.utils import JsonValue, ResponseObject, model_validator
from betterlite.types.utils.base import DictJsonObject


class KeyTokenWhere(TypedDict):
    token: ReadOnly[str]


class KeyAliasContainsWhere(TypedDict):
    key_alias: ReadOnly[InsensitiveContains]


class KeySearchWhere(TypedDict):
    OR: ReadOnly[tuple[KeyTokenWhere, KeyAliasContainsWhere]]


class BulkUpdateKeyRequestItem(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    key: str
    budget_id: str | None = None
    max_budget: float | None = None
    team_id: str | None = None
    tags: list[str] | None = None
    object_permission: dict[str, JsonValue] | None = None


class BulkUpdateKeyRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    keys: list[BulkUpdateKeyRequestItem]


class SuccessfulKeyUpdate(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    key: str
    key_info: dict[str, JsonValue]


class FailedKeyUpdate(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    key: str
    key_info: dict[str, JsonValue] | None = None
    failed_reason: str


class BulkUpdateKeyResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    total_requested: int
    successful_updates: list[SuccessfulKeyUpdate]
    failed_updates: list[FailedKeyUpdate]


class KeyUpdateFields(ResponseObject):
    model_config = ConfigDict(extra="forbid", protected_namespaces=())

    max_budget: float | None = None
    budget_id: str | None = None
    budget_duration: str | None = None
    budget_limits: list[JsonValue] | None = None
    model_max_budget: dict[str, JsonValue] | None = None
    tpm_limit: int | None = None
    rpm_limit: int | None = None
    model_tpm_limit: dict[str, JsonValue] | None = None
    model_rpm_limit: dict[str, JsonValue] | None = None
    max_parallel_requests: int | None = None
    rpm_limit_type: (
        Literal["guaranteed_throughput", "best_effort_throughput", "dynamic"] | None
    ) = None
    tpm_limit_type: (
        Literal["guaranteed_throughput", "best_effort_throughput", "dynamic"] | None
    ) = None
    temp_budget_increase: float | None = None
    temp_budget_expiry: datetime | None = None
    duration: str | None = None
    tags: list[str] | None = None
    metadata: dict[str, JsonValue] | None = None

    @model_validator(mode="after")
    def validate_temp_budget(self) -> Self:
        increase = self.temp_budget_increase
        expiry = self.temp_budget_expiry
        if (increase is None) != (expiry is None):
            raise ValueError(
                "temp_budget_increase and temp_budget_expiry must be set together"
            )
        return self

    @model_validator(mode="after")
    def require_at_least_one_field(self) -> Self:
        if len(self.model_fields_set) == 0:
            raise ValueError("update_fields must specify at least one field to update.")
        return self


class BulkUpdateTeamKeysRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    team_id: str
    key_ids: list[str] | None = None
    all_keys_in_team: bool = False
    update_fields: KeyUpdateFields

    @model_validator(mode="after")
    def validate_selection(self) -> Self:
        has_key_ids = self.key_ids is not None and len(self.key_ids) > 0
        if has_key_ids and self.all_keys_in_team:
            raise ValueError("Provide either `key_ids` or `all_keys_in_team=True`, not both.")
        if not has_key_ids and not self.all_keys_in_team:
            raise ValueError(
                "Must provide either `key_ids` (non-empty) or `all_keys_in_team=True`."
            )
        return self


CustomKeyPolicyOperation: TypeAlias = Literal["generate", "update", "regenerate"]


class CustomKeyPolicyRequest(DictJsonObject):
    model_config = ConfigDict(protected_namespaces=(), frozen=True, extra="ignore")

    operation: CustomKeyPolicyOperation
    existing_key: dict[str, JsonValue] | None
    effective_key: dict[str, JsonValue]
    request: dict[str, JsonValue]

    def fields_set(self) -> set[str]:
        return self.model_fields_set
