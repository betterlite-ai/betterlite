from datetime import datetime

from .inputs import BudgetRowInput


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
