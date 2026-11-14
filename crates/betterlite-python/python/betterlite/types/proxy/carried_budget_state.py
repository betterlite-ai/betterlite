import typing
from collections.abc import Mapping
from datetime import datetime
from types import MappingProxyType
from typing import ClassVar

from pydantic import BaseModel, ConfigDict, Field, JsonValue, ValidationError


class _BudgetSnapshot(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(
        frozen=True, populate_by_name=True, extra="ignore"
    )

    def metadata_entries(self) -> Mapping[str, JsonValue]:
        entries: dict[str, JsonValue] = self.model_dump(by_alias=True, mode="json")
        return MappingProxyType(entries)

    @classmethod
    def from_metadata(cls, metadata: Mapping[str, JsonValue]) -> "typing.Self | None":
        try:
            return cls.model_validate(metadata)
        except ValidationError:
            return None


class KeyBudgetSnapshot(_BudgetSnapshot):
    budget_reset_at: datetime | None = Field(
        validation_alias="user_api_key_budget_reset_at",
        serialization_alias="user_api_key_budget_reset_at",
    )


class TeamBudgetSnapshot(_BudgetSnapshot):
    budget_reset_at: datetime | None = Field(
        validation_alias="user_api_key_team_budget_reset_at",
        serialization_alias="user_api_key_team_budget_reset_at",
    )
    max_budget: float | None = Field(
        validation_alias="user_api_key_team_table_max_budget",
        serialization_alias="user_api_key_team_table_max_budget",
    )


class UserBudgetSnapshot(_BudgetSnapshot):
    budget_reset_at: datetime | None = Field(
        validation_alias="user_api_key_user_budget_reset_at",
        serialization_alias="user_api_key_user_budget_reset_at",
    )
    max_budget: float | None = Field(
        validation_alias="user_api_key_user_table_max_budget",
        serialization_alias="user_api_key_user_table_max_budget",
    )
    user_alias: str | None = Field(
        validation_alias="user_api_key_user_alias",
        serialization_alias="user_api_key_user_alias",
    )


class OrgBudgetSnapshot(_BudgetSnapshot):
    spend: float = Field(
        validation_alias="user_api_key_org_spend",
        serialization_alias="user_api_key_org_spend",
    )
    max_budget: float | None = Field(
        validation_alias="user_api_key_org_max_budget",
        serialization_alias="user_api_key_org_max_budget",
    )
