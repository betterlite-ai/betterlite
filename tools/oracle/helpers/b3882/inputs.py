from typing import Literal, TypedDict


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
    spend: float | None
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

CustomerInput = (
    CustomerCall | TeamMembersCall | ProviderCall | KeyListCall | OrgListCall | CustomerRowCall
)
