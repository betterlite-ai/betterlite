import asyncio
from typing import Final, Literal, TypedDict

from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.memory import memory_endpoints
from litellm.types.memory_management import (
    LiteLLM_MemoryRow,
    MemoryCreateRequest,
    MemoryDeleteResponse,
    MemoryListResponse,
    MemoryUpdateRequest,
)
from pydantic import BaseModel, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

UserRole = Literal[
    "proxy_admin",
    "proxy_admin_viewer",
    "org_admin",
    "internal_user",
    "internal_user_viewer",
    "team",
    "customer",
]
Function = Literal[
    "visibility",
    "is_admin",
    "key_filter",
    "scope",
    "write_access",
    "unique_violation",
    "internal_error",
    "request",
    "model",
]
Answer = JsonValue
JSON: Final = TypeAdapter(JsonValue)


class CallerMetadata(TypedDict):
    service_account_id: str | None


class CallerInput(TypedDict):
    user_id: str | None
    user_role: UserRole | None
    team_id: str | None
    metadata: CallerMetadata


class RowInput(TypedDict):
    user_id: str | None
    team_id: str | None


class Step(TypedDict):
    function: Function
    caller: CallerInput
    row: RowInput
    request: Literal["create", "update"] | None
    model: Literal["row", "list", "delete"] | None
    body: JsonValue
    search: str | None
    key_prefix: str | None
    key: str | None
    requested_user_id: str | None
    requested_team_id: str | None
    code: str | None
    message: str
    log_message: str
    detail: str


class Input(TypedDict):
    steps: list[Step]


class ErrorOut(TypedDict):
    status: int | None
    detail: JsonValue
    type: str | None
    param: str | None
    code: str | None


class StepResult(TypedDict):
    value: Answer
    error: ErrorOut | None


class StoredRow(BaseModel):
    user_id: str | None
    team_id: str | None


# the HTTPException fields read by attribute, so the detail stays typed JSON instead of Any
class RaisedHttpError(BaseModel):
    status_code: int
    detail: JsonValue


class CodedError(Exception):
    code: str | None

    def __init__(self, code: str | None, message: str) -> None:
        super().__init__(message)
        self.code = code


def run(input_value: Input) -> list[StepResult]:
    results: list[StepResult] = []
    for step in input_value["steps"]:
        outcome = asyncio.run(run_step(step))
        results.append(outcome)
    return results


async def run_step(step: Step) -> StepResult:
    caller = build_caller(step["caller"])
    (outcome,) = await asyncio.gather(answer(step, caller), return_exceptions=True)
    value: Answer = None
    error: ErrorOut | None = None
    match outcome:
        case HTTPException():
            seen = RaisedHttpError.model_validate(outcome, from_attributes=True)
            error = {
                "status": seen.status_code,
                "detail": seen.detail,
                "type": None,
                "param": None,
                "code": None,
            }
        case BaseException():
            error = {"status": None, "detail": None, "type": None, "param": None, "code": None}
        case bool() | str() | int() | float() | list() | dict() | None:
            value = outcome
    return {"value": value, "error": error}


def build_caller(caller: CallerInput) -> UserAPIKeyAuth:
    role = caller["user_role"]
    account = caller["metadata"]["service_account_id"]
    metadata: dict[str, str | None] = {"service_account_id": account}
    return UserAPIKeyAuth(
        user_id=caller["user_id"],
        user_role=None if role is None else LitellmUserRoles(role),
        team_id=caller["team_id"],
        metadata=metadata,
    )


def stored_row(step: Step) -> StoredRow:
    row = step["row"]
    return StoredRow(user_id=row["user_id"], team_id=row["team_id"])


async def answer(step: Step, caller: UserAPIKeyAuth) -> Answer:
    value: Answer = None
    match step["function"]:
        case "visibility":
            value = JSON.validate_python(memory_endpoints._visibility_filter(caller))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L79-L94
        case "is_admin":
            value = memory_endpoints._is_admin(caller)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L75-L76
        case "key_filter":
            value = JSON.validate_python(memory_endpoints._key_filter(step["search"], step["key_prefix"], step["key"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L113-L124
        case "scope":
            user_id, team_id = memory_endpoints._resolve_scope(caller, step["requested_user_id"], step["requested_team_id"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L238-L282
            value = JSON.validate_python({"user_id": user_id, "team_id": team_id})
        case "write_access":
            await memory_endpoints._assert_write_access(None, stored_row(step), caller)  # noqa: SLF001  # pyright: ignore[reportArgumentType, reportPrivateUsage]  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L160-L199
        case "unique_violation":
            value = memory_endpoints._is_unique_violation(CodedError(step["code"], step["message"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L223-L235
        case "internal_error":
            raised = memory_endpoints._internal_error(step["log_message"], RuntimeError(step["message"]), step["detail"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L150-L157
            seen = RaisedHttpError.model_validate(raised, from_attributes=True)
            value = JSON.validate_python({"status": seen.status_code, "detail": seen.detail})
        case "request":
            value = request_value(step)
        case "model":
            value = model_value(step)
    return value


def request_value(step: Step) -> Answer:
    kind = step["request"]
    value: Answer = None
    if kind == "create":
        value = create_value(step)
    elif kind == "update":
        value = update_value(step)
    return value


def model_value(step: Step) -> Answer:
    kind = step["model"]
    value: Answer = None
    if kind == "row":
        value = JSON.validate_json(LiteLLM_MemoryRow.model_validate(step["body"]).model_dump_json())
    elif kind == "list":
        listing = MemoryListResponse.model_validate(step["body"])
        value = JSON.validate_json(listing.model_dump_json())
    elif kind == "delete":
        deleted = MemoryDeleteResponse.model_validate(step["body"])
        value = JSON.validate_json(deleted.model_dump_json())
    return value


def create_value(step: Step) -> Answer:
    create = MemoryCreateRequest.model_validate(step["body"])
    column: JsonValue = None
    if create.metadata is not None:
        column = JSON.validate_json(memory_endpoints._serialize_metadata_for_prisma(create.metadata))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L60-L72
    return JSON.validate_python(
        {
            "key": create.key,
            "value": create.value,
            "metadata": create.metadata,
            "user_id": create.user_id,
            "team_id": create.team_id,
            "metadata_column": column,
        }
    )


def update_value(step: Step) -> Answer:
    update = MemoryUpdateRequest.model_validate(step["body"])
    sent = sorted(update.model_fields_set)
    written = "metadata" in sent
    column: JsonValue = None
    if written:
        column = JSON.validate_json(memory_endpoints._serialize_metadata_for_prisma(update.metadata))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L60-L72
    return JSON.validate_python(
        {
            "fields_sent": sent,
            "value": update.value,
            "metadata_written": written,
            "metadata": update.metadata,
            "metadata_column": column,
        }
    )
