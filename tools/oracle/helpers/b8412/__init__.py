from typing import Literal, TypedDict

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

from .records import RecordDump

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


class Call(TypedDict):
    record: Record
    value: dict[str, JsonValue]


class Recorded(TypedDict):
    outcome: Literal["record"]
    dump: RecordDump


class Patched(Recorded):
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
