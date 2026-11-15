import asyncio
import base64
import sys
from collections.abc import Awaitable
from datetime import UTC, datetime
from types import ModuleType
from typing import Required, TypedDict

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.litellm_proxy.skills.transformation import (
    LiteLLMSkillsTransformationHandler,
)
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from pydantic import BaseModel, JsonValue, TypeAdapter

PROXY_MODULE = "litellm.proxy.proxy_server"
PRISMA_MODULE = "prisma"
FIELDS_MODULE = "prisma.fields"
TWO = 2
NO_DATABASE = "Prisma client is not initialized. Database connection required for LiteLLM skills."
MISSING_IDENTITY = "Unable to record skill ownership: caller has no identity scope."
NOT_FOUND = "Skill not found: "
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
TEXT: TypeAdapter[str] = TypeAdapter(str)
OPTIONAL_TEXT: TypeAdapter[str | None] = TypeAdapter(str | None)


class PrincipalFields(TypedDict, total=False):
    user_id: str
    team_id: str
    org_id: str
    api_key: str
    token: str
    admin: bool


class Payload(TypedDict, total=False):
    display_title: str | None
    description: str | None
    instructions: str | None
    file_content: str | None
    file_name: str | None
    file_type: str | None
    metadata: dict[str, JsonValue] | None


class Row(Payload, total=False):
    skill_id: Required[str]
    source: str
    latest_version: str | None
    created_at: str | None
    created_by: str | None
    updated_at: str | None
    updated_by: str | None


class Step(Payload, total=False):
    action: Required[str]
    skill_id: str
    files: list[list[JsonValue]]
    principal: PrincipalFields
    user_id: str | None
    limit: int
    offset: int
    litellm_call_id: str | None


class Input(TypedDict, total=False):
    now: Required[str]
    uuid: Required[str]
    rows: list[Row]
    page: list[Row]
    steps: Required[list[Step]]


class LoggingDetails(TypedDict, total=False):
    model: str | None
    optional_params: dict[str, JsonValue] | None
    litellm_params: dict[str, JsonValue] | None
    custom_llm_provider: str | None
    litellm_call_id: str | None


LOGGING: TypeAdapter[LoggingDetails] = TypeAdapter(LoggingDetails)


class FakeRow(dict[str, JsonValue]):
    @property
    def created_by(self) -> JsonValue:
        return self.get("created_by")

    def model_dump(self) -> dict[str, JsonValue]:
        return dict(self)


class FakeBase64:
    @staticmethod
    def encode(data: bytes) -> str:
        return base64.b64encode(data).decode("ascii")


class FakeClient:
    def __init__(self, table: "TableState") -> None:
        self.db: FakeClient = self
        self.litellm_skillstable = table


class FakeModule(ModuleType):
    Base64 = FakeBase64
    fields: ModuleType | None = None
    prisma_client: FakeClient | None = None


def seeded_row(fields: Row) -> FakeRow:
    return FakeRow(OBJECT.validate_python(fields))


class TableState:
    def __init__(self, input_value: Input) -> None:
        self.now = input_value["now"]
        self.fixed_id = f"litellm_skill_{input_value['uuid']}"
        self.effects: list[JsonValue] = []
        self.rows: dict[str, FakeRow] = {}
        self.page: list[FakeRow] = []
        seeded = input_value.get("rows")
        if seeded is not None:
            self.rows = {fields["skill_id"]: seeded_row(fields) for fields in seeded}
        listed = input_value.get("page")
        if listed is not None:
            self.page = [seeded_row(fields) for fields in listed]

    async def create(self, data: dict[str, JsonValue]) -> FakeRow:
        stored = {**data, "skill_id": self.fixed_id}
        metadata = stored.get("metadata")
        if metadata is not None:
            stored["metadata"] = OBJECT.validate_json(TEXT.validate_python(metadata))
        self.effects.append(JSON.validate_python({"kind": "create", "data": stored}))
        row = FakeRow({**stored, "created_at": self.now, "updated_at": self.now})
        self.rows[self.fixed_id] = row
        return row

    async def find_unique(self, where: dict[str, str]) -> FakeRow | None:
        self.effects.append(JSON.validate_python({"kind": "find_unique", "where": where}))
        return self.rows.get(where["skill_id"])

    async def find_many(self, take: int, skip: int, order: JsonValue, where: JsonValue) -> list[FakeRow]:
        effect = {"kind": "find_many", "take": take, "skip": skip, "order": order, "where": where}
        self.effects.append(JSON.validate_python(effect))
        return self.page

    async def delete(self, where: dict[str, str]) -> FakeRow | None:
        self.effects.append(JSON.validate_python({"kind": "delete", "where": where}))
        return self.rows.pop(where["skill_id"], None)


def principal_of(fields: PrincipalFields | None) -> UserAPIKeyAuth | None:
    if fields is None:
        return None
    principal = UserAPIKeyAuth(
        user_id=fields.get("user_id"), team_id=fields.get("team_id"), org_id=fields.get("org_id"),
        user_role=LitellmUserRoles.PROXY_ADMIN if fields.get("admin") else None,
    )
    principal.api_key = fields.get("api_key")
    principal.token = fields.get("token")
    return principal


async def awaited(pending: Awaitable[BaseModel]) -> BaseModel:
    return await pending


def settled_json(pending: BaseModel | Awaitable[BaseModel]) -> JsonValue:
    match pending:
        case Awaitable():
            return JSON.validate_json(asyncio.run(awaited(pending)).model_dump_json())
        case BaseModel():
            return JSON.validate_json(pending.model_dump_json())


def logging_view(logging_obj: Logging) -> JsonValue:
    details = LOGGING.validate_python(logging_obj.model_call_details)
    return {
        "model": details["model"],
        "optional_params": details["optional_params"],
        "litellm_params": details["litellm_params"],
        "custom_llm_provider": details["custom_llm_provider"],
        "litellm_call_id": details["litellm_call_id"],
    }


def upload_of(entry: list[JsonValue]) -> tuple[str, bytes] | tuple[str, bytes, str | None]:
    file_name = TEXT.validate_python(entry[0])
    file_content = base64.b64decode(TEXT.validate_python(entry[1]))
    if len(entry) == TWO:
        return (file_name, file_content)
    return (file_name, file_content, OPTIONAL_TEXT.validate_python(entry[2]))


def error_token(error: ValueError) -> str:
    message = str(error)
    for prefix, code in (
        (NO_DATABASE, "no_database"),
        (MISSING_IDENTITY, "missing_identity"),
        (NOT_FOUND, "not_found"),
    ):
        if message.startswith(prefix):
            return code
    return "exception"


def create_answer(step: Step, principal: UserAPIKeyAuth | None, logging_obj: Logging) -> dict[str, JsonValue]:
    entries = step.get("files")
    encoded = step.get("file_content")
    pending = LiteLLMSkillsTransformationHandler().create_skill_handler(
        display_title=step.get("display_title"), description=step.get("description"),
        instructions=step.get("instructions"), metadata=step.get("metadata"),
        files=None if entries is None else [upload_of(entry) for entry in entries],
        file_content=None if encoded is None else base64.b64decode(encoded),
        file_name=step.get("file_name"), file_type=step.get("file_type"),
        user_id=step.get("user_id"), user_api_key_dict=principal, _is_async=True,
        logging_obj=logging_obj, litellm_call_id=step.get("litellm_call_id"),
    )
    return {"action": "create", "record": settled_json(pending)}


def list_answer(step: Step, principal: UserAPIKeyAuth | None, logging_obj: Logging) -> dict[str, JsonValue]:
    limit = step.get("limit")
    offset = step.get("offset")
    pending = LiteLLMSkillsTransformationHandler().list_skills_handler(
        limit=20 if limit is None else limit, offset=0 if offset is None else offset,
        _is_async=True, logging_obj=logging_obj, litellm_call_id=step.get("litellm_call_id"),
        user_api_key_dict=principal,
    )
    return {"action": "list", **OBJECT.validate_python(settled_json(pending))}


def skill_answer(
    step: Step, action: str, principal: UserAPIKeyAuth | None, logging_obj: Logging
) -> dict[str, JsonValue]:
    handler = LiteLLMSkillsTransformationHandler()
    skill_id = step.get("skill_id")
    if skill_id is None:
        raise ValueError("a get or delete step names a skill id")
    if action == "get":
        pending = handler.get_skill_handler(
            skill_id=skill_id, _is_async=True, logging_obj=logging_obj,
            litellm_call_id=step.get("litellm_call_id"), user_api_key_dict=principal,
        )
    else:
        pending = handler.delete_skill_handler(
            skill_id=skill_id, _is_async=True, logging_obj=logging_obj,
            litellm_call_id=step.get("litellm_call_id"), user_api_key_dict=principal,
        )
    return {"action": action, "record": settled_json(pending)}


def run_step(step: Step) -> JsonValue:
    action = step["action"]
    if action not in ("create", "list", "get", "delete"):
        return {"action": action, "error": "invalid_input"}
    if action in ("get", "delete") and step.get("skill_id") is None:
        return {"action": action, "error": "invalid_input"}
    logging_obj = Logging(
        model="fixture-model", messages=None, stream=False, call_type=action,
        start_time=datetime(2026, 1, 1, tzinfo=UTC), litellm_call_id="call-1", function_id="fn-1",
    )
    principal = principal_of(step.get("principal"))
    try:
        if action == "create":
            answer = create_answer(step, principal, logging_obj)
        elif action == "list":
            answer = list_answer(step, principal, logging_obj)
        else:
            answer = skill_answer(step, action, principal, logging_obj)
    except ValueError as error:
        answer = {"action": action, "error": error_token(error)}
    return {**answer, "logging": logging_view(logging_obj)}


def run(input_value: Input) -> JsonValue:
    state = TableState(input_value)
    proxy = FakeModule(PROXY_MODULE)
    proxy.prisma_client = FakeClient(state)
    fields = FakeModule(FIELDS_MODULE)
    prisma = FakeModule(PRISMA_MODULE)
    prisma.fields = fields
    sys.modules[PROXY_MODULE] = proxy
    sys.modules[PRISMA_MODULE] = prisma
    sys.modules[FIELDS_MODULE] = fields
    return {
        "steps": JSON.validate_python([run_step(step) for step in input_value["steps"]]),
        "effects": JSON.validate_python(state.effects),
    }
