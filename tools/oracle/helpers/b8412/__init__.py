import re
from typing import Literal, TypedDict

from litellm.proxy._types import (
    AddTeamCallback,
    DeleteTeamRequest,
    LiteLLM_OrganizationTableUpdate as OrgTableUpdate,
    LiteLLM_OrganizationTableWithMembers as OrgTableWithMembers,
    MemberAddRequest,
    OrganizationMemberAddRequest,
    OrganizationMemberDeleteRequest as OrgMemberDelete,
    OrganizationMemberUpdateRequest as OrgMemberUpdate,
    OrganizationUpdateRequestV2,
    OrgMember,
    OrgMemberAddRequest,
    TeamCallbackMetadata,
    TeamMemberAddRequest,
    TeamMemberDeleteRequest,
    TeamMemberUpdateRequest,
)
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError

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
    dump: dict[str, JsonValue]


class Patched(TypedDict):
    outcome: Literal["record"]
    dump: dict[str, JsonValue]
    fields_set: list[str]


class Rejected(TypedDict):
    outcome: Literal["rejected"]
    message: str


Outcome = Recorded | Patched | Rejected

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

DUMP: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])

VALUE_ERROR_PREFIX = "Value error, "
CALLBACK_NAMES_MARKER = "Must be one of "
ENUM_MEMBER = re.compile(r"<LitellmUserRoles\.[A-Z_]+: '([^']+)'>")


def run(input_value: Call) -> Outcome:
    record = input_value["record"]
    try:
        model = RECORDS[record].model_validate(input_value["value"])
    except ValidationError as error:
        return rejected(error)
    dump = DUMP.validate_json(model.model_dump_json())
    if record == "OrganizationUpdateRequestV2":
        fields_set = sorted(model.model_fields_set)
        return {"outcome": "record", "dump": dump, "fields_set": fields_set}
    return {"outcome": "record", "dump": dump}


def rejected(error: ValidationError) -> Rejected:
    message = stable(error)
    return {"outcome": "rejected", "message": message}


def set_names(set_repr: str) -> str:
    text = set_repr.strip()
    if not (text.startswith("{") and text.endswith("}")):
        return set_repr
    members = text.removeprefix("{").removesuffix("}").split(", ")
    names = sorted(member.removeprefix("'").removesuffix("'") for member in members)
    return ", ".join(f"'{name}'" for name in names)


def stable(error: ValidationError) -> str:
    text = error.errors()[0]["msg"].removeprefix(VALUE_ERROR_PREFIX)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/auto_router_model_naming.py#L366
    if CALLBACK_NAMES_MARKER in text:
        head, _, tail = text.partition(CALLBACK_NAMES_MARKER)
        return f"{head}{CALLBACK_NAMES_MARKER}[{set_names(tail)}]"
    return ENUM_MEMBER.sub(r"'\1'", text)
