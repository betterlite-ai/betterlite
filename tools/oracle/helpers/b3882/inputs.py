from datetime import datetime
from typing import Final, Literal, TypedDict

from litellm.types.integrations.prometheus import DEFINED_PROMETHEUS_METRICS

BUDGET_METRICS: Final[tuple[DEFINED_PROMETHEUS_METRICS, ...]] = (
    "litellm_remaining_team_budget_metric",
    "litellm_team_max_budget_metric",
    "litellm_team_budget_remaining_hours_metric",
    "litellm_team_members_metric",
    "litellm_remaining_org_budget_metric",
    "litellm_org_max_budget_metric",
    "litellm_org_budget_remaining_hours_metric",
    "litellm_remaining_api_key_budget_metric",
    "litellm_api_key_max_budget_metric",
    "litellm_api_key_budget_remaining_hours_metric",
    "litellm_remaining_user_budget_metric",
    "litellm_user_max_budget_metric",
    "litellm_user_budget_remaining_hours_metric",
    "litellm_remaining_customer_budget_metric",
    "litellm_customer_max_budget_metric",
    "litellm_customer_budget_remaining_hours_metric",
)

# LiteLLM gives this gauge a fixed api_provider label and leaves its name out of DEFINED_PROMETHEUS_METRICS
PROVIDER_METRIC: Final = "litellm_provider_remaining_budget_metric"
ALL_METRICS: Final = (*BUDGET_METRICS, PROVIDER_METRIC)


class RemainingBudgetCall(TypedDict):
    function: Literal["remaining_budget"]
    max_budget: float | None
    spend: float | None


class RemainingHoursCall(TypedDict):
    function: Literal["remaining_hours"]
    reset_at: str
    now: str


class TeamCall(TypedDict):
    function: Literal["team"]
    team_id: str
    team_alias: str | None
    spend: float | None
    max_budget: float | None
    budget_reset_at: str | None
    now: str
    labels_by_metric: dict[str, list[str]]


class OrgCall(TypedDict):
    function: Literal["org"]
    org_id: str
    org_alias: str
    spend: float
    max_budget: float | None
    budget_reset_at: str | None
    now: str
    labels_by_metric: dict[str, list[str]]


class KeyCall(TypedDict):
    function: Literal["key"]
    token: str
    key_alias: str | None
    spend: float
    max_budget: float | None
    budget_reset_at: str | None
    now: str
    labels_by_metric: dict[str, list[str]]


class UserCall(TypedDict):
    function: Literal["user"]
    user_id: str
    user_email: str | None
    user_alias: str | None
    spend: float
    max_budget: float | None
    budget_reset_at: str | None
    now: str
    labels_by_metric: dict[str, list[str]]


class CustomerCall(TypedDict):
    function: Literal["customer"]
    end_user_id: str
    spend: float
    max_budget: float | None
    budget_reset_at: str | None
    now: str
    enable_end_user_tracking: bool
    labels_by_metric: dict[str, list[str]]


class TeamMembersCall(TypedDict):
    function: Literal["team_members"]
    team_id: str
    team_alias: str | None
    member_count: int
    labels_by_metric: dict[str, list[str]]


class ProviderCall(TypedDict):
    function: Literal["provider"]
    provider: str
    spend: float
    budget_limit: float


class KeyListEntryRecord(TypedDict):
    kind: Literal["record"]
    token: str
    key_alias: str | None
    spend: float
    max_budget: float | None
    budget_reset_at: str | None


class KeyListEntryToken(TypedDict):
    kind: Literal["token"]
    token: str


KeyListEntry = KeyListEntryRecord | KeyListEntryToken


class KeyListCall(TypedDict):
    function: Literal["key_list"]
    entries: list[KeyListEntry]
    now: str
    labels_by_metric: dict[str, list[str]]


class BudgetRowInput(TypedDict):
    max_budget: float | None
    budget_reset_at: str | None


class OrgRowInput(TypedDict):
    organization_id: str | None
    organization_alias: str | None
    spend: float | None
    budget: BudgetRowInput | None


class OrgListCall(TypedDict):
    function: Literal["org_list"]
    rows: list[OrgRowInput]
    now: str
    labels_by_metric: dict[str, list[str]]


class CustomerRowInput(TypedDict):
    user_id: str
    spend: float
    budget: BudgetRowInput | None


class CustomerRowCall(TypedDict):
    function: Literal["customer_row"]
    customer: CustomerRowInput
    default_budget: BudgetRowInput | None
    now: str
    enable_end_user_tracking: bool
    labels_by_metric: dict[str, list[str]]


class BudgetRow:
    def __init__(self, max_budget: float | None, budget_reset_at: datetime | None) -> None:
        self.max_budget = max_budget
        self.budget_reset_at = budget_reset_at


class OrganizationRow:
    def __init__(
        self,
        organization_id: str | None,
        organization_alias: str | None,
        spend: float | None,
        litellm_budget_table: BudgetRow | None,
    ) -> None:
        self.organization_id = organization_id
        self.organization_alias = organization_alias
        self.spend = spend
        self.litellm_budget_table = litellm_budget_table


class CustomerRow:
    def __init__(
        self,
        user_id: str,
        spend: float,
        litellm_budget_table: BudgetRow | None,
    ) -> None:
        self.user_id = user_id
        self.spend = spend
        self.litellm_budget_table = litellm_budget_table


def parse_optional(text: str | None) -> datetime | None:
    if text is None:
        return None
    return datetime.fromisoformat(text)


def budget_row(call: BudgetRowInput | None) -> BudgetRow | None:
    if call is None:
        return None
    return BudgetRow(call["max_budget"], parse_optional(call["budget_reset_at"]))


Input = (
    RemainingBudgetCall
    | RemainingHoursCall
    | TeamCall
    | OrgCall
    | KeyCall
    | UserCall
    | CustomerCall
    | TeamMembersCall
    | ProviderCall
    | KeyListCall
    | OrgListCall
    | CustomerRowCall
)

EntityInput = (
    TeamCall
    | OrgCall
    | KeyCall
    | UserCall
    | CustomerCall
    | TeamMembersCall
    | ProviderCall
    | KeyListCall
    | OrgListCall
    | CustomerRowCall
)

EntityCall = TeamCall | OrgCall | KeyCall | UserCall

CustomerInput = (
    CustomerCall | TeamMembersCall | ProviderCall | KeyListCall | OrgListCall | CustomerRowCall
)
