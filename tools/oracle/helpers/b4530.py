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
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


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
    return {"value": json_value(outcome), "error": error_out(outcome)}


def json_value(outcome: Answer | BaseException) -> Answer:
    match outcome:
        case BaseException():
            return None
        case bool() | str() | int() | float() | list() | dict() | None:
            return outcome


def error_out(outcome: Answer | BaseException) -> ErrorOut | None:
    match outcome:
        case HTTPException():
            seen = RaisedHttpError.model_validate(outcome, from_attributes=True)
            return {
                "status": seen.status_code,
                "detail": seen.detail,
                "type": None,
                "param": None,
                "code": None,
            }
        case BaseException():
            return {"status": None, "detail": None, "type": None, "param": None, "code": None}
        case bool() | str() | int() | float() | list() | dict() | None:
            return None


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
    function = step["function"]
    if function == "visibility":
        value = JSON.validate_python(memory_endpoints._visibility_filter(caller))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L79-L94
    elif function == "is_admin":
        value = memory_endpoints._is_admin(caller)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L75-L76
    elif function == "key_filter":
        matched = memory_endpoints._key_filter(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L113-L124
            step["search"], step["key_prefix"], step["key"]
        )
        value = JSON.validate_python(matched)
    elif function == "scope":
        user_id, team_id = memory_endpoints._resolve_scope(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L238-L282
            caller, step["requested_user_id"], step["requested_team_id"]
        )
        value = JSON.validate_python({"user_id": user_id, "team_id": team_id})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L238-L282
    elif function == "write_access":
        await memory_endpoints._assert_write_access(None, stored_row(step), caller)  # type: ignore[arg-type]  # noqa: SLF001  # pyright: ignore[reportArgumentType, reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L160-L199
    elif function == "unique_violation":
        value = memory_endpoints._is_unique_violation(CodedError(step["code"], step["message"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L223-L235
    elif function == "internal_error":
        raised = memory_endpoints._internal_error(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L150-L157
            step["log_message"], RuntimeError(step["message"]), step["detail"]
        )
        seen = RaisedHttpError.model_validate(raised, from_attributes=True)
        value = JSON.validate_python({"status": seen.status_code, "detail": seen.detail})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L150-L157
    elif function == "request":
        value = request_value(step)
    elif function == "model":
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
    match step["model"]:
        case "row":
            row = LiteLLM_MemoryRow.model_validate(step["body"])
            return JSON.validate_json(row.model_dump_json())
        case "list":
            listing = MemoryListResponse.model_validate(step["body"])
            return JSON.validate_json(listing.model_dump_json())
        case "delete":
            deleted = MemoryDeleteResponse.model_validate(step["body"])
            return JSON.validate_json(deleted.model_dump_json())
        case None:
            return None


def create_value(step: Step) -> Answer:
    create = MemoryCreateRequest.model_validate(step["body"])
    column: JsonValue = None
    if create.metadata is not None:
        column = JSON.validate_json(
            memory_endpoints._serialize_metadata_for_prisma(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L60-L72
                create.metadata
            )
        )
    return JSON.validate_python(
        {  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L60-L72
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
        column = JSON.validate_json(
            memory_endpoints._serialize_metadata_for_prisma(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L60-L72
                update.metadata
            )
        )
    return JSON.validate_python(
        {  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/memory/memory_endpoints.py#L60-L72
            "fields_sent": sent,
            "value": update.value,
            "metadata_written": written,
            "metadata": update.metadata,
            "metadata_column": column,
        }
    )
