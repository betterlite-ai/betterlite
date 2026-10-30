from datetime import datetime
from typing import Literal, NotRequired, TypedDict

from litellm.models.budget import LiteLLM_BudgetTable, LiteLLM_BudgetTableFull
from litellm.models.object_permission import LiteLLM_ObjectPermissionTable
from litellm.models.organization_membership import LiteLLM_OrganizationMembershipTable

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class BudgetInput(TypedDict):
    operation: Literal["budget"]
    row: dict[str, Json]


class FullInput(TypedDict):
    operation: Literal["full"]
    row: dict[str, Json]


class PermissionInput(TypedDict):
    operation: Literal["permission"]
    row: dict[str, Json]


class MembershipInput(TypedDict):
    operation: Literal["membership"]
    row: dict[str, Json]
    attribute_user: NotRequired[bool]


class EffectiveInput(TypedDict):
    operation: Literal["effective"]
    row: dict[str, Json]
    now: str


Input = BudgetInput | FullInput | PermissionInput | MembershipInput | EffectiveInput


class AttributeUser:
    def __init__(self, user_email: object) -> None:
        self.user_email = user_email


def run(input_value: Input) -> Json:
    if input_value["operation"] == "budget":
        return LiteLLM_BudgetTable.model_validate(input_value["row"]).model_dump(mode="json")
    if input_value["operation"] == "full":
        return LiteLLM_BudgetTableFull.model_validate(input_value["row"]).model_dump(mode="json")
    if input_value["operation"] == "permission":
        return LiteLLM_ObjectPermissionTable.model_validate(input_value["row"]).model_dump(
            mode="json"
        )
    if input_value["operation"] == "membership":
        row: dict[str, object] = dict(input_value["row"])
        if input_value.get("attribute_user"):
            row["user"] = AttributeUser("nested")
        membership = LiteLLM_OrganizationMembershipTable.model_validate(row)
        if input_value.get("attribute_user"):
            membership.user = {"user_email": membership.user_email}
        return membership.model_dump(mode="json")
    if input_value["operation"] != "effective":
        raise ValueError("unsupported operation")
    effective_input: EffectiveInput = input_value
    budget = LiteLLM_BudgetTable.model_validate(effective_input["row"])
    now = datetime.fromisoformat(effective_input["now"])
    return {
        "active_temp_budget_increase": budget.active_temp_budget_increase(now),
        "effective_max_budget": budget.effective_max_budget(now),
    }
