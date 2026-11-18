import logging
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.models.team_membership import LiteLLM_TeamMembership
from litellm.proxy.auth.auth_checks import (
    _log_budget_lookup_failure,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L415-L435
    _member_allowed_models,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L366-L369
    _membership_from_shared_load,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L411-L412
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


class BudgetFields(TypedDict):
    allowed_models: NotRequired[list[str] | None]
    budget_id: NotRequired[str | None]
    created_at: NotRequired[str]
    max_budget: NotRequired[float | None]


class MembershipFields(TypedDict):
    user_id: str
    team_id: str
    budget_id: NotRequired[str | None]
    litellm_budget_table: NotRequired[BudgetFields | None]
    spend: NotRequired[float | None]
    total_spend: NotRequired[float | None]


class MemberModelScopeCall(TypedDict):
    kind: Literal["member_model_scope"]
    membership: MembershipFields


class SharedTypedCall(TypedDict):
    kind: Literal["shared_typed"]
    membership: MembershipFields


class SharedSubtypeCall(TypedDict):
    kind: Literal["shared_subtype"]
    membership: MembershipFields


class SharedForeignCall(TypedDict):
    kind: Literal["shared_foreign"]
    payload: JsonValue


class BudgetFailureCall(TypedDict):
    kind: Literal["budget_failure_log_policy"]
    exact_exception: bool
    message: str


Input: TypeAlias = (
    MemberModelScopeCall
    | SharedTypedCall
    | SharedSubtypeCall
    | SharedForeignCall
    | BudgetFailureCall
)

MEMBERSHIP: Final[TypeAdapter[LiteLLM_TeamMembership]] = TypeAdapter(LiteLLM_TeamMembership)


class DerivedMembership(LiteLLM_TeamMembership): ...


DERIVED: Final[TypeAdapter[DerivedMembership]] = TypeAdapter(DerivedMembership)


class Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.records: list[logging.LogRecord] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.records.append(record)


def run(input_value: Input) -> JsonValue:
    if input_value["kind"] == "member_model_scope":
        return member_models(input_value["membership"])
    if input_value["kind"] == "shared_typed":
        return shared_load(MEMBERSHIP.validate_python(input_value["membership"]))
    if input_value["kind"] == "shared_subtype":
        return shared_load(DERIVED.validate_python(input_value["membership"]))
    if input_value["kind"] == "shared_foreign":
        return shared_load(input_value["payload"])
    return failure_report(input_value)


def member_models(fields: MembershipFields) -> JsonValue:
    membership = MEMBERSHIP.validate_python(fields)
    return {"allowed_models": list(_member_allowed_models(membership))}


def shared_load(result: LiteLLM_TeamMembership | JsonValue) -> JsonValue:
    membership = _membership_from_shared_load(result)
    if membership is None:
        return {"accepted": False, "user_id": None}
    return {"accepted": True, "user_id": membership.user_id}


def failure_report(call: BudgetFailureCall) -> JsonValue:
    capture = Capture()
    logger = logging.getLogger("LiteLLM Proxy")
    logger.addHandler(capture)
    try:
        _log_budget_lookup_failure(
            "user",
            error_for(exact_exception=call["exact_exception"], message=call["message"]),
        )
    finally:
        logger.removeHandler(capture)
    first = capture.records[0] if capture.records else None
    return {
        "records": len(capture.records),
        "level": None if first is None else first.levelname,
        "hint": any("schema mismatches" in record.getMessage() for record in capture.records),
    }


def error_for(*, exact_exception: bool, message: str) -> Exception:
    if exact_exception:
        return Exception(message)
    return ValueError(message)
