from datetime import UTC, datetime
from typing import Final, Literal, TypedDict

from litellm.models.budget import LiteLLM_BudgetTableFull
from litellm.models.end_user import LiteLLM_EndUserTable
from litellm.models.organization import LiteLLM_OrganizationTable
from litellm.models.team import LiteLLM_TeamTableCachedObj
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._types import UserAPIKeyAuth
from litellm.types.proxy.carried_budget_state import (
    KeyBudgetSnapshot,
    OrgBudgetSnapshot,
    TeamBudgetSnapshot,
    UserBudgetSnapshot,
)

CREATED_AT: Final = datetime(2026, 1, 1, tzinfo=UTC)


class BudgetRowInput(TypedDict):
    max_budget: float | None
    budget_reset_at: str | None


class TeamSnapshotInput(TypedDict):
    budget_reset_at: str | None
    max_budget: float | None


class OrgSnapshotInput(TypedDict):
    spend: float
    max_budget: float | None


class KeySnapshotInput(TypedDict):
    budget_reset_at: str | None


class UserSnapshotInput(TypedDict):
    budget_reset_at: str | None
    max_budget: float | None
    user_alias: str | None


class RecordLookup(TypedDict):
    kind: Literal["record"]
    budget_reset_at: str | None
    max_budget: float | None
    spend: float | None
    organization_alias: str | None
    user_email: str | None
    user_alias: str | None
    budget: BudgetRowInput | None


class MissingLookup(TypedDict):
    kind: Literal["missing"]


LookupCall = RecordLookup | MissingLookup


class SampleCall(TypedDict):
    lookup: LookupCall
    now: str
    noop: bool
    labels_by_metric: dict[str, list[str]]


class TeamCall(SampleCall):
    function: Literal["team"]
    team_id: str | None
    team_alias: str | None
    team_spend: float | None
    team_max_budget: float | None
    response_cost: float
    carried: TeamSnapshotInput | None


class OrgCall(SampleCall):
    function: Literal["org"]
    org_id: str | None
    org_alias: str | None
    response_cost: float
    carried: OrgSnapshotInput | None
    database: bool


class CustomerCall(SampleCall):
    function: Literal["customer"]
    end_user_id: str | None
    response_cost: float
    enable_prometheus_only: bool | None
    disable_end_user_cost_tracking: bool | None


class KeyCall(SampleCall):
    function: Literal["key"]
    user_api_key: str | None
    user_api_key_alias: str | None
    response_cost: float
    key_max_budget: float | None
    key_spend: float | None
    carried: KeySnapshotInput | None


class UserCall(SampleCall):
    function: Literal["user"]
    user_id: str | None
    user_spend: float | None
    user_max_budget: float | None
    response_cost: float
    carried: UserSnapshotInput | None
    user_email: str | None


class DefaultBudgetCall(TypedDict):
    function: Literal["default_customer_budget"]
    default_budget_id: str | None
    lookup: LookupCall


EntityCall = TeamCall | OrgCall | CustomerCall | KeyCall | UserCall
Input = EntityCall | DefaultBudgetCall


def parse_optional(text: str | None) -> datetime | None:
    if text is None:
        return None
    return datetime.fromisoformat(text)


def budget_table(call: BudgetRowInput | None) -> LiteLLM_BudgetTableFull | None:
    if call is None:
        return None
    return LiteLLM_BudgetTableFull(
        max_budget=call["max_budget"], budget_reset_at=parse_optional(call["budget_reset_at"]),
        created_at=CREATED_AT,
    )


def team_snapshot(call: TeamSnapshotInput | None) -> TeamBudgetSnapshot | None:
    if call is None:
        return None
    return TeamBudgetSnapshot(
        budget_reset_at=parse_optional(call["budget_reset_at"]),
        max_budget=call["max_budget"],
    )


def org_snapshot(call: OrgSnapshotInput | None) -> OrgBudgetSnapshot | None:
    if call is None:
        return None
    return OrgBudgetSnapshot(spend=call["spend"], max_budget=call["max_budget"])


def key_snapshot(call: KeySnapshotInput | None) -> KeyBudgetSnapshot | None:
    if call is None:
        return None
    return KeyBudgetSnapshot(budget_reset_at=parse_optional(call["budget_reset_at"]))


def user_snapshot(call: UserSnapshotInput | None) -> UserBudgetSnapshot | None:
    if call is None:
        return None
    return UserBudgetSnapshot(
        budget_reset_at=parse_optional(call["budget_reset_at"]),
        max_budget=call["max_budget"],
        user_alias=call["user_alias"],
    )


def team_object(team_id: str, record: RecordLookup) -> LiteLLM_TeamTableCachedObj:
    return LiteLLM_TeamTableCachedObj(
        team_id=team_id,
        max_budget=record["max_budget"],
        budget_reset_at=parse_optional(record["budget_reset_at"]),
    )


def org_object(org_id: str, record: RecordLookup) -> LiteLLM_OrganizationTable:
    spend = record["spend"]
    return LiteLLM_OrganizationTable(
        organization_id=org_id,
        organization_alias=record["organization_alias"],
        spend=0.0 if spend is None else spend,
        budget_id="scripted-budget",
        created_by="oracle",
        updated_by="oracle",
        litellm_budget_table=budget_table(record["budget"]),
    )


def key_object(hashed_token: str, record: RecordLookup) -> UserAPIKeyAuth:
    return UserAPIKeyAuth(
        token=hashed_token,
        budget_reset_at=parse_optional(record["budget_reset_at"]),
    )


def user_object(user_id: str, record: RecordLookup) -> LiteLLM_UserTable:
    return LiteLLM_UserTable(
        user_id=user_id,
        max_budget=record["max_budget"],
        budget_reset_at=parse_optional(record["budget_reset_at"]),
        user_email=record["user_email"],
        user_alias=record["user_alias"],
    )


def end_user_object(key: str, record: RecordLookup) -> LiteLLM_EndUserTable:
    spend = record["spend"]
    return LiteLLM_EndUserTable(
        user_id=key.removeprefix("end_user_id:"), blocked=False,
        spend=0.0 if spend is None else spend,
        litellm_budget_table=budget_table(record["budget"]),
    )
