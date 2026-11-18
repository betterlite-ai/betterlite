from typing import Literal, TypeAlias, TypedDict

from litellm.proxy._types import (
    AddTeamCallback,
    DeleteTeamRequest,
    MemberAddRequest,
    OrganizationMemberAddRequest,
    OrganizationUpdateRequestV2,
    OrgMember,
    OrgMemberAddRequest,
    TeamCallbackMetadata,
    TeamMemberAddRequest,
    TeamMemberDeleteRequest,
    TeamMemberUpdateRequest,
)
from litellm.proxy._types import (
    LiteLLM_OrganizationTableUpdate as OrgTableUpdate,
)
from litellm.proxy._types import (
    LiteLLM_OrganizationTableWithMembers as OrgTableWithMembers,
)
from litellm.proxy._types import (
    OrganizationMemberDeleteRequest as OrgMemberDelete,
)
from litellm.proxy._types import (
    OrganizationMemberUpdateRequest as OrgMemberUpdate,
)
from pydantic import BaseModel, JsonValue, TypeAdapter

Record = Literal[
    "DeleteTeamRequest",
    "AddTeamCallback",
    "TeamCallbackMetadata",
    "OrgMember",
    "MemberAddRequest",
    "TeamMemberAddRequest",
    "OrgMemberAddRequest",
    "OrganizationMemberAddRequest",
    "TeamMemberDeleteRequest",
    "OrganizationMemberDeleteRequest",
    "TeamMemberUpdateRequest",
    "OrganizationMemberUpdateRequest",
    "OrganizationTableUpdate",
    "OrganizationUpdateRequestV2",
    "OrganizationTableWithMembers",
]

Amount: TypeAlias = int | float


class Call(TypedDict):
    record: Record
    value: dict[str, JsonValue]


class MemberDump(TypedDict):
    user_id: str | None
    user_email: str | None
    role: str


class MembershipDump(TypedDict):
    user_id: str
    organization_id: str
    user_role: str | None
    spend: Amount
    budget_id: str | None
    created_at: str
    updated_at: str
    user: dict[str, JsonValue] | None
    litellm_budget_table: dict[str, JsonValue] | None
    user_email: str | None


class DeleteTeamDump(TypedDict):
    team_ids: list[str]


class AddTeamCallbackDump(TypedDict):
    callback_name: str
    callback_type: str | None
    callback_vars: dict[str, str]


class TeamCallbackMetadataDump(TypedDict):
    success_callback: list[str]
    failure_callback: list[str]
    callbacks: list[str]
    callback_vars: dict[str, str]


class MemberAddDump(TypedDict):
    member: MemberDump | list[MemberDump]


class TeamMemberAddDump(TypedDict):
    member: MemberDump | list[MemberDump]
    team_id: str
    max_budget_in_team: Amount | None
    budget_duration: str | None
    allowed_models: list[str] | None


class OrgMemberAddDump(TypedDict):
    member: MemberDump | list[MemberDump]


class OrganizationMemberAddDump(TypedDict):
    member: MemberDump | list[MemberDump]
    organization_id: str
    max_budget_in_organization: Amount | None


class TeamMemberDeleteDump(TypedDict):
    user_id: str | None
    user_email: str | None
    team_id: str


class OrganizationMemberDeleteDump(TypedDict):
    user_id: str | None
    user_email: str | None
    organization_id: str


class TeamMemberUpdateDump(TypedDict):
    user_id: str | None
    user_email: str | None
    team_id: str
    max_budget_in_team: Amount | None
    role: str | None
    tpm_limit: int | None
    rpm_limit: int | None
    budget_duration: str | None
    allowed_models: list[str] | None
    temp_budget_increase: Amount | None
    temp_budget_expiry: str | None


class OrganizationMemberUpdateDump(TypedDict):
    user_id: str | None
    user_email: str | None
    organization_id: str
    max_budget_in_organization: Amount | None
    role: str | None


class OrgTableUpdateDump(TypedDict):
    budget_id: str | None
    soft_budget: Amount | None
    max_budget: Amount | None
    max_parallel_requests: int | None
    tpm_limit: int | None
    rpm_limit: int | None
    tpd_limit: int | None
    model_max_budget: dict[str, JsonValue] | None
    budget_duration: str | None
    allowed_models: list[str] | None
    temp_budget_increase: Amount | None
    temp_budget_expiry: str | None
    organization_id: str | None
    organization_alias: str | None
    spend: Amount | None
    metadata: dict[str, JsonValue] | None
    models: list[str] | None
    updated_by: str | None
    object_permission: dict[str, JsonValue] | None
    model_tpm_limit: dict[str, int] | None
    model_rpm_limit: dict[str, int] | None


class OrgUpdateV2Dump(TypedDict):
    organization_alias: str | None
    models: list[str] | None
    metadata: dict[str, JsonValue] | None
    tpm_limit: int | None
    rpm_limit: int | None
    max_budget: Amount | None
    soft_budget: Amount | None
    max_parallel_requests: int | None
    model_max_budget: dict[str, JsonValue] | None
    budget_duration: str | None
    object_permission: dict[str, JsonValue] | None


class OrgTableWithMembersDump(TypedDict):
    organization_id: str | None
    organization_alias: str | None
    budget_id: str
    spend: Amount
    metadata: dict[str, JsonValue] | None
    models: list[str]
    model_spend: dict[str, JsonValue] | None
    created_by: str
    updated_by: str
    users: list[dict[str, JsonValue]] | None
    litellm_budget_table: dict[str, JsonValue] | None
    object_permission: dict[str, JsonValue] | None
    object_permission_id: str | None
    members: list[MembershipDump]
    teams: list[dict[str, JsonValue]]
    created_at: str
    updated_at: str


# nested rows other tasks own (users, teams, budget tables) and free-form metadata stay open JSON
RecordDump: TypeAlias = (
    DeleteTeamDump
    | AddTeamCallbackDump
    | TeamCallbackMetadataDump
    | MemberDump
    | MemberAddDump
    | TeamMemberAddDump
    | OrgMemberAddDump
    | OrganizationMemberAddDump
    | TeamMemberDeleteDump
    | OrganizationMemberDeleteDump
    | TeamMemberUpdateDump
    | OrganizationMemberUpdateDump
    | OrgTableUpdateDump
    | OrgUpdateV2Dump
    | OrgTableWithMembersDump
)


class Recorded(TypedDict):
    outcome: Literal["record"]
    dump: RecordDump


class Patched(TypedDict):
    outcome: Literal["record"]
    dump: RecordDump
    fields_set: list[str]


RECORDS: dict[Record, type[BaseModel]] = {
    "DeleteTeamRequest": DeleteTeamRequest,
    "AddTeamCallback": AddTeamCallback,
    "TeamCallbackMetadata": TeamCallbackMetadata,
    "OrgMember": OrgMember,
    "MemberAddRequest": MemberAddRequest,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4606
    "TeamMemberAddRequest": TeamMemberAddRequest,
    "OrgMemberAddRequest": OrgMemberAddRequest,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4627
    "OrganizationMemberAddRequest": OrganizationMemberAddRequest,
    "TeamMemberDeleteRequest": TeamMemberDeleteRequest,
    "OrganizationMemberDeleteRequest": OrgMemberDelete,
    "TeamMemberUpdateRequest": TeamMemberUpdateRequest,
    "OrganizationMemberUpdateRequest": OrgMemberUpdate,
    "OrganizationTableUpdate": OrgTableUpdate,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3607
    "OrganizationUpdateRequestV2": OrganizationUpdateRequestV2,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3634
    "OrganizationTableWithMembers": OrgTableWithMembers,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3664
}

DUMP: TypeAdapter[RecordDump] = TypeAdapter(RecordDump)


def run(input_value: Call) -> Recorded | Patched:
    record = input_value["record"]
    model = RECORDS[record].model_validate(input_value["value"])
    dump = DUMP.validate_json(model.model_dump_json())
    if record == "OrganizationUpdateRequestV2":
        patched: Patched = {
            "outcome": "record",
            "dump": dump,
            "fields_set": sorted(model.model_fields_set),
        }
        return patched
    recorded: Recorded = {"outcome": "record", "dump": dump}
    return recorded
