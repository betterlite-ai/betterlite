from typing import TypedDict

from litellm.models.team import LiteLLM_TeamTable, Member, MemberBase
from litellm.models.team_membership import LiteLLM_TeamMembership

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Input(TypedDict):
    operation: str
    row: dict[str, Json]


def run(input_value: Input) -> Json:
    operation = input_value["operation"]
    row = input_value["row"]
    if operation == "member":
        return Member.model_validate(row).model_dump(mode="json")
    if operation == "member_base":
        return MemberBase.model_validate(row).model_dump(mode="json")
    if operation == "team":
        return LiteLLM_TeamTable.model_validate(row).model_dump(mode="json")
    if operation == "membership":
        return LiteLLM_TeamMembership.model_validate(row).model_dump(mode="json")
    raise ValueError("unsupported operation")
