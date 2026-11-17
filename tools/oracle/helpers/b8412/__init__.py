import re
from typing import Literal, TypedDict

from litellm.proxy._types import (
    AddTeamCallback,
    DeleteTeamRequest,
    LiteLLM_OrganizationTableUpdate,
    LiteLLM_OrganizationTableWithMembers,
    MemberAddRequest,
    OrganizationMemberAddRequest,
    OrganizationMemberDeleteRequest,
    OrganizationMemberUpdateRequest,
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
    "MemberAddRequest": MemberAddRequest,
    "TeamMemberAddRequest": TeamMemberAddRequest,
    "OrgMemberAddRequest": OrgMemberAddRequest,
    "OrganizationMemberAddRequest": OrganizationMemberAddRequest,
    "TeamMemberDeleteRequest": TeamMemberDeleteRequest,
    # ast-grep-ignore: no-long-names
    "OrganizationMemberDeleteRequest": OrganizationMemberDeleteRequest,
    "TeamMemberUpdateRequest": TeamMemberUpdateRequest,
    # ast-grep-ignore: no-long-names
    "OrganizationMemberUpdateRequest": OrganizationMemberUpdateRequest,
    # ast-grep-ignore: no-long-names
    "OrganizationTableUpdate": LiteLLM_OrganizationTableUpdate,
    # ast-grep-ignore: no-long-names
    "OrganizationTableWithMembers": LiteLLM_OrganizationTableWithMembers,
}

DUMP: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])

VALUE_ERROR_PREFIX = "Value error, "
CALLBACK_NAMES_MARKER = "Must be one of "
ENUM_MEMBER = re.compile(r"<LitellmUserRoles\.[A-Z_]+: '([^']+)'>")


def run(input_value: Call) -> Outcome:
    if input_value["record"] == "OrganizationUpdateRequestV2":
        return patch_outcome(input_value["value"])
    return record_outcome(input_value["record"], input_value["value"])


def record_outcome(record: Record, value: dict[str, JsonValue]) -> Outcome:
    try:
        model = RECORDS[record].model_validate(value)
    except ValidationError as error:
        return rejected(error)
    return {
        "outcome": "record",
        "dump": DUMP.validate_json(model.model_dump_json()),
    }


def patch_outcome(value: dict[str, JsonValue]) -> Outcome:
    try:
        request = OrganizationUpdateRequestV2.model_validate(value)
    except ValidationError as error:
        return rejected(error)
    return {
        "outcome": "record",
        "dump": DUMP.validate_json(request.model_dump_json()),
        "fields_set": sorted(request.model_fields_set),
    }


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
    text = error.errors()[0]["msg"].removeprefix(VALUE_ERROR_PREFIX)
    if CALLBACK_NAMES_MARKER in text:
        head, _, tail = text.partition(CALLBACK_NAMES_MARKER)
        return f"{head}{CALLBACK_NAMES_MARKER}[{set_names(tail)}]"
    return ENUM_MEMBER.sub(r"'\1'", text)
