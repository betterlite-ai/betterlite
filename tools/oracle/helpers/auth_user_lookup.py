import time
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.caching.dual_cache import LimitedSizeOrderedDict
from litellm.proxy._types import RoleBasedPermissions
from litellm.proxy.auth.auth_checks import (
    _should_check_db,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L2473-L2484
    _update_last_db_access_time,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L2487-L2488
    _user_read_failure,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L2763-L2768
    get_role_based_models,
    get_role_based_routes,
)
from pydantic import JsonValue, TypeAdapter, ValidationError

RoleName: TypeAlias = Literal["proxy_admin", "team", "internal_user"]

ROLE_PERMISSIONS: Final[TypeAdapter[list[RoleBasedPermissions]]] = TypeAdapter(
    list[RoleBasedPermissions]
)

JSON_VALUE: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class ShouldCheck(TypedDict):
    op: Literal["should_check"]
    key: str
    expiry: int


class UpdateEntry(TypedDict):
    op: Literal["update"]
    key: str
    value: JsonValue


CadenceOperation: TypeAlias = ShouldCheck | UpdateEntry


class CadenceCall(TypedDict):
    kind: Literal["cadence"]
    max_size: int
    times_ms: list[int]
    operations: list[CadenceOperation]


class RoleSelectionCall(TypedDict):
    kind: Literal["role_selection"]
    role: RoleName
    role_permissions: NotRequired[JsonValue]


class RoleValidationCall(TypedDict):
    kind: Literal["role_validation"]
    role_permissions: JsonValue


class MessageFailure(TypedDict):
    kind: Literal["message"]
    message: str


class IoFailure(TypedDict):
    kind: Literal["io"]
    message: str


ErrorCase: TypeAlias = MessageFailure | IoFailure


class ReadErrorCall(TypedDict):
    kind: Literal["read_error_classification"]
    errors: list[ErrorCase]


Input: TypeAlias = CadenceCall | RoleSelectionCall | RoleValidationCall | ReadErrorCall


class ScriptedClock:
    def __init__(self, times_ms: list[int]) -> None:
        self.remaining = list(times_ms)
        self.last = 0.0

    def __call__(self) -> float:
        if self.remaining:
            self.last = self.remaining.pop(0) / 1000.0
        return self.last


def run(input_value: Input) -> JsonValue:
    if input_value["kind"] == "cadence":
        return cadence_report(input_value)
    if input_value["kind"] == "role_selection":
        return role_report(input_value)
    if input_value["kind"] == "role_validation":
        return validation_report(input_value)
    return classification_report(input_value)


def cadence_report(call: CadenceCall) -> JsonValue:
    tracker: LimitedSizeOrderedDict[str, tuple[JsonValue | None, float]] = (
        LimitedSizeOrderedDict(max_size=call["max_size"])
    )
    saved = time.time
    time.time = ScriptedClock(call["times_ms"])
    decisions: list[JsonValue] = []
    try:
        for operation in call["operations"]:
            if operation["op"] == "should_check":
                decisions.append(_should_check_db(operation["key"], tracker, operation["expiry"]))
            else:
                _update_last_db_access_time(operation["key"], operation["value"], tracker)
    finally:
        time.time = saved
    return {"decisions": decisions}


def role_report(call: RoleSelectionCall) -> JsonValue:
    settings: dict[str, JsonValue] = {}
    if "role_permissions" in call:
        settings["role_permissions"] = call["role_permissions"]
    role = RoleBasedPermissions.model_validate({"role": call["role"]}).role
    try:
        models = get_role_based_models(role, settings)
        routes = get_role_based_routes(role, settings)
    except ValidationError:
        return {"valid": False}
    return {
        "valid": True,
        "models": JSON_VALUE.validate_python(models),
        "routes": JSON_VALUE.validate_python(routes),
    }


def validation_report(call: RoleValidationCall) -> JsonValue:
    try:
        entries = ROLE_PERMISSIONS.validate_python(call["role_permissions"])
    except ValidationError:
        return {"valid": False}
    dumped: list[JsonValue] = [entry.model_dump(mode="json") for entry in entries]
    return {"valid": True, "entries": dumped}


def classification_report(call: ReadErrorCall) -> JsonValue:
    reports: list[JsonValue] = [classification(error) for error in call["errors"]]
    return {"errors": reports}


def classification(error: ErrorCase) -> JsonValue:
    constructed = error_value(error)
    failure = _user_read_failure("u1", constructed)
    if failure is constructed:
        return {"kind": "unavailable", "message": str(failure)}
    return {"kind": "missing", "message": str(failure)}


def error_value(error: ErrorCase) -> Exception:
    if error["kind"] == "message":
        return Exception(error["message"])
    return OSError(error["message"])
