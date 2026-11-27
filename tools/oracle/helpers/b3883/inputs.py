from datetime import UTC, datetime
from typing import Final, Literal, TypedDict

from litellm.models.budget import LiteLLM_BudgetTableFull
from litellm.types.proxy.carried_budget_state import (
    KeyBudgetSnapshot,
    OrgBudgetSnapshot,
    TeamBudgetSnapshot,
    UserBudgetSnapshot,
)

TEAM_METRICS: Final[tuple[str, ...]] = (
    "litellm_remaining_team_budget_metric",
    "litellm_team_max_budget_metric",
    "litellm_team_budget_remaining_hours_metric",
)
ORG_METRICS: Final[tuple[str, ...]] = (
    "litellm_remaining_org_budget_metric",
    "litellm_org_max_budget_metric",
    "litellm_org_budget_remaining_hours_metric",
)
KEY_METRICS: Final[tuple[str, ...]] = (
    "litellm_remaining_api_key_budget_metric",
    "litellm_api_key_max_budget_metric",
    "litellm_api_key_budget_remaining_hours_metric",
)
USER_METRICS: Final[tuple[str, ...]] = (
    "litellm_remaining_user_budget_metric",
    "litellm_user_max_budget_metric",
    "litellm_user_budget_remaining_hours_metric",
)
CUSTOMER_METRICS: Final[tuple[str, ...]] = (
    "litellm_remaining_customer_budget_metric",
    "litellm_customer_max_budget_metric",
    "litellm_customer_budget_remaining_hours_metric",
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


class TeamCall(TypedDict):
    function: Literal["team"]
    team_id: str | None
    team_alias: str | None
    team_spend: float | None
    team_max_budget: float | None
    response_cost: float
    carried: TeamSnapshotInput | None
    lookup: LookupCall
    now: str
    noop: bool
    labels_by_metric: dict[str, list[str]]


class OrgCall(TypedDict):
    function: Literal["org"]
    org_id: str | None
    org_alias: str | None
    response_cost: float
    carried: OrgSnapshotInput | None
    database: bool
    lookup: LookupCall
    now: str
    noop: bool
    labels_by_metric: dict[str, list[str]]


class CustomerCall(TypedDict):
    function: Literal["customer"]
    end_user_id: str | None
    response_cost: float
    enable_prometheus_only: bool | None
    disable_end_user_cost_tracking: bool | None
    lookup: LookupCall
    now: str
    noop: bool
    labels_by_metric: dict[str, list[str]]


class KeyCall(TypedDict):
    function: Literal["key"]
    user_api_key: str | None
    user_api_key_alias: str | None
    response_cost: float
    key_max_budget: float | None
    key_spend: float | None
    carried: KeySnapshotInput | None
    lookup: LookupCall
    now: str
    noop: bool
    labels_by_metric: dict[str, list[str]]


class UserCall(TypedDict):
    function: Literal["user"]
    user_id: str | None
    user_spend: float | None
    user_max_budget: float | None
    response_cost: float
    carried: UserSnapshotInput | None
    user_email: str | None
    lookup: LookupCall
    now: str
    noop: bool
    labels_by_metric: dict[str, list[str]]


class DefaultBudgetCall(TypedDict):
    function: Literal["default_customer_budget"]
    default_budget_id: str | None
    lookup: LookupCall


Input = TeamCall | OrgCall | CustomerCall | KeyCall | UserCall | DefaultBudgetCall


def parse_optional(text: str | None) -> datetime | None:
    if text is None:
        return None
    return datetime.fromisoformat(text)


def budget_table(call: BudgetRowInput | None) -> LiteLLM_BudgetTableFull | None:
    if call is None:
        return None
    return LiteLLM_BudgetTableFull(
        max_budget=call["max_budget"],
        budget_reset_at=parse_optional(call["budget_reset_at"]),
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
