import logging
from typing import Final, Literal, TypeAlias, TypedDict, assert_never

from litellm.models.team import LiteLLM_TeamTable
from litellm.models.verification_token import LiteLLM_VerificationToken
from litellm.proxy._types import UpdateKeyRequest, UpdateTeamRequest
from litellm.proxy.management_endpoints import team_admin_field_permissions as policy
from litellm.proxy.management_endpoints.team_admin_field_permissions import (
    TeamAdminMemberKeyEditingDisabled as KeyEditingDisabled,
)
from pydantic import BaseModel, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
TEAM_REQUEST: Final[TypeAdapter[UpdateTeamRequest]] = TypeAdapter(UpdateTeamRequest)
TEAM_ROW: Final[TypeAdapter[LiteLLM_TeamTable]] = TypeAdapter(LiteLLM_TeamTable)
KEY_REQUEST: Final[TypeAdapter[UpdateKeyRequest]] = TypeAdapter(UpdateKeyRequest)
KEY_ROW: Final[TypeAdapter[LiteLLM_VerificationToken]] = TypeAdapter(
    LiteLLM_VerificationToken
)

WarningKindLiteral: TypeAlias = Literal["not_a_list", "unsupported", "other"]
WARNING_FRAGMENTS: Final[tuple[tuple[str, WarningKindLiteral], ...]] = (
    ("must be a list of field names", "not_a_list"),
    ("ignores unsupported field(s)", "unsupported"),
)


class ResolveInput(TypedDict):
    function: Literal["resolve"]
    general_settings: dict[str, JsonValue]
    supported: list[str]


class MemberKeyBudgetsInput(TypedDict):
    function: Literal["member_key_budgets"]
    general_settings: dict[str, JsonValue]


class ChangedInput(TypedDict):
    function: Literal["changed_team", "changed_key"]
    request: JsonValue
    existing: JsonValue


class PermittedInput(TypedDict):
    function: Literal["team_verdict", "team_request"]
    request: JsonValue
    existing: JsonValue
    permitted: list[str]


class EnabledInput(TypedDict):
    function: Literal["key_verdict", "key_request"]
    request: JsonValue
    existing: JsonValue
    enabled: bool


Input: TypeAlias = (
    ResolveInput | MemberKeyBudgetsInput | ChangedInput | PermittedInput | EnabledInput
)


class WarningKind(TypedDict):
    kind: WarningKindLiteral


class ResolveAnswer(TypedDict):
    fields: list[str]
    warnings: list[WarningKind]


class MemberKeyBudgetsAnswer(TypedDict):
    allowed: bool


class ChangedAnswer(TypedDict):
    changed: list[str]


class TeamVerdictAnswer(TypedDict):
    kind: Literal["allowed", "field_not_permitted", "disabled"]
    field: str | None
    request: JsonValue | None


class TeamRequestAnswer(TypedDict):
    status: int | None
    detail: str | None
    request: JsonValue | None


class KeyVerdictAnswer(TypedDict):
    kind: Literal["allowed", "field_not_permitted", "disabled"]
    field: str | None
    changed: list[str]


class KeyRequestAnswer(TypedDict):
    status: int | None
    detail: str | None


Answer: TypeAlias = (
    ResolveAnswer
    | MemberKeyBudgetsAnswer
    | ChangedAnswer
    | TeamVerdictAnswer
    | TeamRequestAnswer
    | KeyVerdictAnswer
    | KeyRequestAnswer
)


class Rejection(BaseModel):
    status_code: int
    detail: str


REJECTION: Final[TypeAdapter[Rejection]] = TypeAdapter(Rejection)


class WarningKinds(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.kinds: list[WarningKind] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.kinds.append({"kind": warning_kind(record.getMessage())})


def warning_kind(message: str) -> WarningKindLiteral:
    for fragment, kind in WARNING_FRAGMENTS:
        if fragment in message:
            return kind
    return "other"


def resolve_answer(input_value: ResolveInput) -> ResolveAnswer:
    handler = WarningKinds()
    logger = logging.getLogger("LiteLLM Proxy")
    logger.addHandler(handler)
    try:
        fields = policy.resolve_team_admin_editable_fields(
            input_value["general_settings"], frozenset(input_value["supported"])
        )
    finally:
        logger.removeHandler(handler)
    return {"fields": sorted(fields), "warnings": handler.kinds}


def member_key_budgets_answer(input_value: MemberKeyBudgetsInput) -> MemberKeyBudgetsAnswer:
    allowed = policy.team_admin_may_edit_member_key_budgets(input_value["general_settings"])
    return {"allowed": allowed}


def changed_team_answer(input_value: ChangedInput) -> ChangedAnswer:
    changed = policy.changed_team_fields(
        TEAM_REQUEST.validate_python(input_value["request"]),
        TEAM_ROW.validate_python(input_value["existing"]),
    )
    return {"changed": sorted(changed)}


def changed_key_answer(input_value: ChangedInput) -> ChangedAnswer:
    changed = policy.changed_key_fields(
        KEY_REQUEST.validate_python(input_value["request"]),
        KEY_ROW.validate_python(input_value["existing"]),
    )
    return {"changed": sorted(changed)}


def team_verdict_view(verdict: policy.TeamAdminEditVerdict) -> TeamVerdictAnswer:
    match verdict:
        case policy.TeamAdminEditAllowed():
            dumped = _JSON.validate_json(verdict.request.model_dump_json(exclude_unset=True))
            return {"kind": "allowed", "field": None, "request": dumped}
        case policy.TeamAdminFieldNotPermitted(field=field):
            return {"kind": "field_not_permitted", "field": field, "request": None}
        case policy.TeamAdminEditingDisabled():
            return {"kind": "disabled", "field": None, "request": None}
    assert_never(verdict)


def key_verdict_view(verdict: policy.TeamAdminKeyEditVerdict) -> KeyVerdictAnswer:
    match verdict:
        case policy.TeamAdminKeyEditAllowed():
            return {"kind": "allowed", "field": None, "changed": sorted(verdict.changed)}
        case policy.TeamAdminFieldNotPermitted(field=field):
            return {"kind": "field_not_permitted", "field": field, "changed": []}
        case KeyEditingDisabled():
            return {"kind": "disabled", "field": None, "changed": []}
    assert_never(verdict)


def team_verdict_answer(input_value: PermittedInput) -> TeamVerdictAnswer:
    verdict = policy.team_admin_edit_verdict(
        TEAM_REQUEST.validate_python(input_value["request"]),
        TEAM_ROW.validate_python(input_value["existing"]),
        frozenset(input_value["permitted"]),
    )
    return team_verdict_view(verdict)


def team_request_answer(input_value: PermittedInput) -> TeamRequestAnswer:
    verdict = policy.team_admin_edit_verdict(
        TEAM_REQUEST.validate_python(input_value["request"]),
        TEAM_ROW.validate_python(input_value["existing"]),
        frozenset(input_value["permitted"]),
    )
    try:
        allowed = policy.team_admin_request_or_raise(verdict)
    except HTTPException as error:
        rejected = REJECTION.validate_python(error, from_attributes=True)
        return {"status": rejected.status_code, "detail": rejected.detail, "request": None}
    dumped = _JSON.validate_json(allowed.model_dump_json(exclude_unset=True))
    return {"status": None, "detail": None, "request": dumped}


def key_verdict_answer(input_value: EnabledInput) -> KeyVerdictAnswer:
    verdict = policy.team_admin_key_edit_verdict(
        KEY_REQUEST.validate_python(input_value["request"]),
        KEY_ROW.validate_python(input_value["existing"]),
        input_value["enabled"],
    )
    return key_verdict_view(verdict)


def key_request_answer(input_value: EnabledInput) -> KeyRequestAnswer:
    verdict = policy.team_admin_key_edit_verdict(
        KEY_REQUEST.validate_python(input_value["request"]),
        KEY_ROW.validate_python(input_value["existing"]),
        input_value["enabled"],
    )
    try:
        policy.team_admin_key_request_or_raise(verdict)
    except HTTPException as error:
        rejected = REJECTION.validate_python(error, from_attributes=True)
        return {"status": rejected.status_code, "detail": rejected.detail}
    return {"status": None, "detail": None}


def team_answer(input_value: PermittedInput) -> TeamVerdictAnswer | TeamRequestAnswer:
    match input_value["function"]:
        case "team_verdict":
            return team_verdict_answer(input_value)
        case "team_request":
            return team_request_answer(input_value)


def key_answer(input_value: EnabledInput) -> KeyVerdictAnswer | KeyRequestAnswer:
    match input_value["function"]:
        case "key_verdict":
            return key_verdict_answer(input_value)
        case "key_request":
            return key_request_answer(input_value)


def run(input_value: Input) -> Answer:
    match input_value["function"]:
        case "resolve":
            return resolve_answer(input_value)
        case "member_key_budgets":
            return member_key_budgets_answer(input_value)
        case "changed_team":
            return changed_team_answer(input_value)
        case "changed_key":
            return changed_key_answer(input_value)
        case "team_verdict" | "team_request":
            return team_answer(input_value)
        case "key_verdict" | "key_request":
            return key_answer(input_value)
