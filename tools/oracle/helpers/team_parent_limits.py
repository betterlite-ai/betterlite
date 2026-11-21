from typing import Literal, NotRequired, TypedDict

from fastapi import HTTPException
from litellm.proxy._types import (
    LiteLLM_OrganizationTable,
    LiteLLM_TeamTable,
    NewTeamRequest,
    UpdateTeamRequest,
)
from litellm.proxy.management_endpoints.team_endpoints import (
    check_org_team_model_specific_limits,
    check_org_team_rpm_tpm_limits,
)
from pydantic import BaseModel


class Metadata(TypedDict, total=False):
    model_rpm_limit: dict[str, int | None] | None
    model_tpm_limit: dict[str, int | None] | None
    tpm_limit_type: str
    rpm_limit_type: str


class Team(TypedDict):
    team_id: str
    organization_id: NotRequired[str]
    tpm_limit: NotRequired[int | None]
    rpm_limit: NotRequired[int | None]
    metadata: NotRequired[Metadata | None]


class Budget(TypedDict, total=False):
    max_budget: float | None
    tpm_limit: int | None
    rpm_limit: int | None


class Organization(TypedDict):
    organization_id: str
    budget_id: str
    created_by: str
    updated_by: str
    models: NotRequired[list[str]]
    metadata: NotRequired[Metadata | None]
    litellm_budget_table: NotRequired[Budget | None]


class TeamFields(TypedDict, total=False):
    team_id: str
    tpm_limit: int | None
    rpm_limit: int | None
    model_rpm_limit: dict[str, int] | None
    model_tpm_limit: dict[str, int] | None
    metadata: Metadata | None


class TeamRequest(TypedDict):
    kind: Literal["new", "update"]
    body: TeamFields


class Call(TypedDict):
    check: Literal["model_specific", "rpm_tpm"]
    teams: list[Team]
    organization: Organization
    request: TeamRequest


class Input(TypedDict):
    calls: list[Call]


class Outcome(TypedDict):
    status: int | None
    detail: str | None


# the allocation checks raise HTTPException(400, detail=<message string>)
class Rejected(BaseModel):
    status_code: int
    detail: str


def run(input_value: Input) -> list[Outcome]:
    outcomes: list[Outcome] = []
    for call in input_value["calls"]:
        teams = [LiteLLM_TeamTable.model_validate(team) for team in call["teams"]]
        organization = LiteLLM_OrganizationTable.model_validate(call["organization"])
        body = call["request"]["body"]
        data = (
            NewTeamRequest.model_validate(body)
            if call["request"]["kind"] == "new"
            else UpdateTeamRequest.model_validate(body)
        )
        try:
            if call["check"] == "model_specific":
                check_org_team_model_specific_limits(teams=teams, org_table=organization, data=data)
            else:
                check_org_team_rpm_tpm_limits(teams=teams, org_table=organization, data=data)
        except HTTPException as error:
            rejected = Rejected.model_validate(error, from_attributes=True)
            outcomes.append({"status": rejected.status_code, "detail": rejected.detail})
            continue
        outcomes.append({"status": None, "detail": None})
    return outcomes
