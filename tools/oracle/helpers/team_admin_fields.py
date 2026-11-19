import logging
from typing import Literal, TypeAlias, TypedDict

from fastapi import HTTPException
from litellm.models.team import LiteLLM_TeamTable
from litellm.models.verification_token import LiteLLM_VerificationToken
from litellm.proxy._types import UpdateKeyRequest, UpdateTeamRequest
from litellm.proxy.management_endpoints import team_admin_field_permissions as policy
from litellm.proxy.management_endpoints.team_admin_field_permissions import (
    TeamAdminEditAllowed,
    TeamAdminEditingDisabled,
    TeamAdminFieldNotPermitted,
    TeamAdminKeyEditAllowed,
    TeamAdminMemberKeyEditingDisabled as KeyEditingDisabled,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import assert_never, override

_JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ResolveInput(TypedDict):
    function: Literal["resolve"]
    general_settings: dict[str, JsonValue]
    supported: list[str]


class MemberKeyBudgetsInput(TypedDict):
    function: Literal["member_key_budgets"]
    general_settings: dict[str, JsonValue]


class ChangedTeamInput(TypedDict):
    function: Literal["changed_team"]
    request: JsonValue
    existing: JsonValue


class TeamVerdictInput(TypedDict):
    function: Literal["team_verdict"]
    request: JsonValue
    existing: JsonValue
    permitted: list[str]


class TeamRequestInput(TypedDict):
    function: Literal["team_request"]
    request: JsonValue
    existing: JsonValue
    permitted: list[str]


class ChangedKeyInput(TypedDict):
    function: Literal["changed_key"]
    request: JsonValue
    existing: JsonValue


class KeyVerdictInput(TypedDict):
    function: Literal["key_verdict"]
    request: JsonValue
    existing: JsonValue
    enabled: bool


class KeyRequestInput(TypedDict):
    function: Literal["key_request"]
    request: JsonValue
    existing: JsonValue
    enabled: bool


Input: TypeAlias = (
    ResolveInput
    | MemberKeyBudgetsInput
    | ChangedTeamInput
    | TeamVerdictInput
    | TeamRequestInput
    | ChangedKeyInput
    | KeyVerdictInput
    | KeyRequestInput
)


class WarningKind(TypedDict):
    kind: Literal["not_a_list", "unsupported", "other"]


class ResolveAnswer(TypedDict):
    fields: list[str]
    warnings: list[WarningKind]


class MemberKeyBudgetsAnswer(TypedDict):
    allowed: bool


class ChangedAnswer(TypedDict):
    changed: list[str]


class TeamAllowedAnswer(TypedDict):
    kind: Literal["allowed"]
    field: None
    request: JsonValue


class TeamBlockedAnswer(TypedDict):
    kind: Literal["field_not_permitted"]
    field: str
    request: None


class TeamDisabledAnswer(TypedDict):
    kind: Literal["disabled"]
    field: None
    request: None


TeamVerdictAnswer: TypeAlias = TeamAllowedAnswer | TeamBlockedAnswer | TeamDisabledAnswer


class TeamRequestAnswer(TypedDict):
    status: int | None
    detail: str | None
    request: JsonValue | None


class KeyAllowedAnswer(TypedDict):
    kind: Literal["allowed"]
    field: None
    changed: list[str]


class KeyBlockedAnswer(TypedDict):
    kind: Literal["field_not_permitted"]
    field: str
    changed: list[str]


class KeyDisabledAnswer(TypedDict):
    kind: Literal["disabled"]
    field: None
    changed: list[str]


KeyVerdictAnswer: TypeAlias = KeyAllowedAnswer | KeyBlockedAnswer | KeyDisabledAnswer


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


class WarningKinds(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.kinds: list[WarningKind] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if "must be a list of field names" in message:
            self.kinds.append({"kind": "not_a_list"})
        elif "ignores unsupported field(s)" in message:
            self.kinds.append({"kind": "unsupported"})
        else:
            self.kinds.append({"kind": "other"})


def team_request_of(document: JsonValue) -> UpdateTeamRequest:
    request = UpdateTeamRequest.model_validate(document)
    return request


def team_row_of(document: JsonValue) -> LiteLLM_TeamTable:
    row = LiteLLM_TeamTable.model_validate(document)
    return row


def key_request_of(document: JsonValue) -> UpdateKeyRequest:
    request = UpdateKeyRequest.model_validate(document)
    return request


def key_row_of(document: JsonValue) -> LiteLLM_VerificationToken:
    row = LiteLLM_VerificationToken.model_validate(document)
    return row


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
    answer: ResolveAnswer = {"fields": sorted(fields), "warnings": handler.kinds}
    return answer


def member_key_budgets_answer(input_value: MemberKeyBudgetsInput) -> MemberKeyBudgetsAnswer:
    allowed = policy.team_admin_may_edit_member_key_budgets(input_value["general_settings"])
    answer: MemberKeyBudgetsAnswer = {"allowed": allowed}
    return answer


def changed_team_answer(input_value: ChangedTeamInput) -> ChangedAnswer:
    changed = policy.changed_team_fields(
        team_request_of(input_value["request"]), team_row_of(input_value["existing"])
    )
    answer: ChangedAnswer = {"changed": sorted(changed)}
    return answer


def team_verdict_view(verdict: policy.TeamAdminEditVerdict) -> TeamVerdictAnswer:
    match verdict:
        case TeamAdminEditAllowed():
            dumped = _JSON.validate_json(verdict.request.model_dump_json(exclude_unset=True))
            allowed: TeamAllowedAnswer = {"kind": "allowed", "field": None, "request": dumped}
            return allowed
        case TeamAdminFieldNotPermitted(field=field):
            blocked: TeamBlockedAnswer = {"kind": "field_not_permitted", "field": field, "request": None}
            return blocked
        case TeamAdminEditingDisabled():
            disabled: TeamDisabledAnswer = {"kind": "disabled", "field": None, "request": None}
            return disabled
    assert_never(verdict)


def team_verdict_answer(input_value: TeamVerdictInput) -> TeamVerdictAnswer:
    verdict = policy.team_admin_edit_verdict(
        team_request_of(input_value["request"]),
        team_row_of(input_value["existing"]),
        frozenset(input_value["permitted"]),
    )
    return team_verdict_view(verdict)


def team_request_answer(input_value: TeamRequestInput) -> TeamRequestAnswer:
    verdict = policy.team_admin_edit_verdict(
        team_request_of(input_value["request"]),
        team_row_of(input_value["existing"]),
        frozenset(input_value["permitted"]),
    )
    try:
        allowed = policy.team_admin_request_or_raise(verdict)
    except HTTPException as error:
        refused: TeamRequestAnswer = {"status": error.status_code, "detail": error.detail, "request": None}
        return refused
    dumped = _JSON.validate_json(allowed.model_dump_json(exclude_unset=True))
    answer: TeamRequestAnswer = {"status": None, "detail": None, "request": dumped}
    return answer


def changed_key_answer(input_value: ChangedKeyInput) -> ChangedAnswer:
    changed = policy.changed_key_fields(
        key_request_of(input_value["request"]), key_row_of(input_value["existing"])
    )
    answer: ChangedAnswer = {"changed": sorted(changed)}
    return answer


def key_verdict_view(verdict: policy.TeamAdminKeyEditVerdict) -> KeyVerdictAnswer:
    match verdict:
        case TeamAdminKeyEditAllowed():
            allowed: KeyAllowedAnswer = {
                "kind": "allowed",
                "field": None,
                "changed": sorted(verdict.changed),
            }
            return allowed
        case TeamAdminFieldNotPermitted(field=field):
            blocked: KeyBlockedAnswer = {
                "kind": "field_not_permitted",
                "field": field,
                "changed": [],
            }
            return blocked
        case KeyEditingDisabled():
            disabled: KeyDisabledAnswer = {"kind": "disabled", "field": None, "changed": []}
            return disabled
    assert_never(verdict)


def key_verdict_answer(input_value: KeyVerdictInput) -> KeyVerdictAnswer:
    verdict = policy.team_admin_key_edit_verdict(
        key_request_of(input_value["request"]),
        key_row_of(input_value["existing"]),
        input_value["enabled"],
    )
    return key_verdict_view(verdict)


def key_request_answer(input_value: KeyRequestInput) -> KeyRequestAnswer:
    verdict = policy.team_admin_key_edit_verdict(
        key_request_of(input_value["request"]),
        key_row_of(input_value["existing"]),
        input_value["enabled"],
    )
    try:
        policy.team_admin_key_request_or_raise(verdict)
    except HTTPException as error:
        refused: KeyRequestAnswer = {"status": error.status_code, "detail": error.detail}
        return refused
    answer: KeyRequestAnswer = {"status": None, "detail": None}
    return answer


def run(input_value: Input) -> Answer:
    result: Answer
    if input_value["function"] == "resolve":
        result = resolve_answer(input_value)
    elif input_value["function"] == "member_key_budgets":
        result = member_key_budgets_answer(input_value)
    elif input_value["function"] == "changed_team":
        result = changed_team_answer(input_value)
    elif input_value["function"] == "team_verdict":
        result = team_verdict_answer(input_value)
    elif input_value["function"] == "team_request":
        result = team_request_answer(input_value)
    elif input_value["function"] == "changed_key":
        result = changed_key_answer(input_value)
    elif input_value["function"] == "key_verdict":
        result = key_verdict_answer(input_value)
    else:
        result = key_request_answer(input_value)
    return result
