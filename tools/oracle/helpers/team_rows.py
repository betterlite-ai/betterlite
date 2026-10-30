from collections.abc import Callable
from typing import TypedDict, cast

from litellm.models.team import (
    BudgetLimitEntry,
    LiteLLM_DeletedTeamTable,
    LiteLLM_ModelTable,
    LiteLLM_TeamTable,
    LiteLLM_TeamTableCachedObj,
    Member,
    MemberBase,
    TeamBase,
)
from litellm.models.team_membership import LiteLLM_TeamMembership

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Call(TypedDict):
    name: str
    operation: str
    row: Json


class Input(TypedDict):
    calls: list[Call]


def execute(call: Call) -> Json:
    operation, row = call["operation"], call["row"]
    if operation in ("validator", "base_model"):
        source = TeamBase.model_validate(row) if operation == "base_model" else row
        validator = cast(Callable[[object], Json], LiteLLM_TeamTable.set_model_info)
        try:
            result = validator(source)
        except ValueError as error:
            return {"row": row, "error": str(error)}
        else:
            return {"row": result, "error": None}
    models = {
        "member_base": MemberBase,
        "member": Member,
        "budget_limit": BudgetLimitEntry,
        "model": LiteLLM_ModelTable,
        "team_base": TeamBase,
        "team": LiteLLM_TeamTable,
        "cached": LiteLLM_TeamTableCachedObj,
        "deleted": LiteLLM_DeletedTeamTable,
        "membership": LiteLLM_TeamMembership,
    }
    if operation == "limits":
        member = LiteLLM_TeamMembership.model_validate(row)
        return [member.safe_get_team_member_rpm_limit(), member.safe_get_team_member_tpm_limit()]
    result = models[operation].model_validate(row).model_dump(mode="json")
    return {"kind": operation, "row": result}


def run(input_value: Input) -> Json:
    results: list[Json] = []
    for call in input_value["calls"]:
        try:
            results.append(execute(call))
        except ValueError as error:
            message = str(error)
            exact = [
                "input needs to be a dictionary",
                "Either user id or user email must be provided",
            ]
            fields = [
                "metadata",
                "aliases",
                "config",
                "permissions",
                "model_max_budget",
                "model_aliases",
                "router_settings",
                "budget_limits",
            ]
            exact += [f"Field {field} should be a valid dictionary" for field in fields]
            results.append(next((text for text in exact if text in message), "invalid row"))
    return results
