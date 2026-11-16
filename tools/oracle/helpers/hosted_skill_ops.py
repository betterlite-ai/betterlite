import asyncio
import base64
import sys
from collections.abc import Coroutine, Sequence
from datetime import UTC, datetime
from types import ModuleType
from typing import Protocol, Required, TypedDict, runtime_checkable

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.litellm_proxy.skills.transformation import (
    LiteLLMSkillsTransformationHandler,
)
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

PROXY_MODULE = "litellm.proxy.proxy_server"
PRISMA_MODULE = "prisma"
FIELDS_MODULE = "prisma.fields"
NO_DATABASE = "Prisma client is not initialized. Database connection required for LiteLLM skills."
MISSING_IDENTITY = "Unable to record skill ownership: caller has no identity scope."
NOT_FOUND = "Skill not found: "
DEFAULT_LIMIT = 20
DEFAULT_OFFSET = 0
TWO = 2
OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
TEXT: TypeAdapter[str] = TypeAdapter(str)
OPTIONAL_TEXT: TypeAdapter[str | None] = TypeAdapter(str | None)
Upload = tuple[str, bytes] | tuple[str, bytes, str | None]


@runtime_checkable
class Settled(Protocol):
    def model_dump_json(self) -> str: ...


Pending = Settled | Coroutine[object, object, Settled]


class HostedHandler(Protocol):
    def create_skill_handler(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/litellm_proxy/skills/transformation.py#L40-L56
        self, *, display_title: str | None, description: str | None, instructions: str | None,
        files: Sequence[Upload] | None, file_content: bytes | None, file_name: str | None,
        file_type: str | None, metadata: dict[str, JsonValue] | None, user_id: str | None,
        user_api_key_dict: UserAPIKeyAuth | None, _is_async: bool, logging_obj: Logging,
        litellm_call_id: str | None,
    ) -> Pending: ...

    def list_skills_handler(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/litellm_proxy/skills/transformation.py#L159-L167
        self, *, limit: int, offset: int, _is_async: bool, logging_obj: Logging,
        litellm_call_id: str | None, user_api_key_dict: UserAPIKeyAuth | None,
    ) -> Pending: ...

    def get_skill_handler(
        self, *, skill_id: str, _is_async: bool, logging_obj: Logging,
        litellm_call_id: str | None, user_api_key_dict: UserAPIKeyAuth | None,
    ) -> Pending: ...

    def delete_skill_handler(
        self, *, skill_id: str, _is_async: bool, logging_obj: Logging,
        litellm_call_id: str | None, user_api_key_dict: UserAPIKeyAuth | None,
    ) -> Pending: ...


class PrincipalFields(TypedDict, total=False):
    user_id: str
    team_id: str
    org_id: str
    api_key: str
    token: str
    admin: bool


class Step(TypedDict, total=False):
    action: Required[str]
    display_title: str | None
    description: str | None
    instructions: str | None
    file_content: str | None
    file_name: str | None
    file_type: str | None
    metadata: dict[str, JsonValue] | None
    files: list[list[JsonValue]]
    principal: PrincipalFields
    skill_id: str
    user_id: str | None
    limit: int
    offset: int
    litellm_call_id: str | None


class Input(TypedDict, total=False):
    now: Required[str]
    uuid: Required[str]
    rows: list[dict[str, JsonValue]]
    page: list[dict[str, JsonValue]]
    steps: Required[list[Step]]


class LoggingDetails(BaseModel):
    model: str | None = None
    optional_params: dict[str, JsonValue] | None = None
    litellm_params: dict[str, JsonValue] | None = None
    custom_llm_provider: str | None = None
    litellm_call_id: str | None = None


class RecordedLogging(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: LoggingDetails


class FakeRow(dict[str, JsonValue]):
    @property
    def created_by(self) -> JsonValue:
        return self.get("created_by")

    def model_dump(self) -> dict[str, JsonValue]:
        return dict(self)


class FakeBase64:
    @staticmethod
    def encode(data: bytes) -> str:
        encoded = base64.b64encode(data)
        return encoded.decode("ascii")


class FakeClient:
    def __init__(self, table: "TableState") -> None:
        self.db: FakeClient = self
        self.litellm_skillstable = table


class FakeModule(ModuleType):
    Base64 = FakeBase64
    fields: ModuleType | None = None
    prisma_client: FakeClient | None = None


class TableState:
    def __init__(self, input_value: Input) -> None:
        self.now = input_value["now"]
        self.fixed_id = f"litellm_skill_{input_value['uuid']}"
        self.effects: list[JsonValue] = []
        self.rows: dict[str, FakeRow] = {}
        self.page: list[FakeRow] = []
        seeded = input_value.get("rows")
        if seeded is not None:
            self.rows = {TEXT.validate_python(row["skill_id"]): FakeRow(row) for row in seeded}
        listed = input_value.get("page")
        if listed is not None:
            self.page = [FakeRow(row) for row in listed]

    async def create(self, data: dict[str, JsonValue]) -> FakeRow:
        stored = {**data, "skill_id": self.fixed_id}
        metadata = stored.get("metadata")
        if metadata is not None:
            stored["metadata"] = OBJECT.validate_json(TEXT.validate_python(metadata))
        self.effects.append({"kind": "create", "data": stored})
        row = FakeRow({**stored, "created_at": self.now, "updated_at": self.now})
        self.rows[self.fixed_id] = row
        return row

    async def find_unique(self, where: dict[str, JsonValue]) -> FakeRow | None:
        self.effects.append({"kind": "find_unique", "where": where})
        return self.rows.get(TEXT.validate_python(where["skill_id"]))

    async def find_many(
        self, take: int, skip: int, order: JsonValue, where: JsonValue,
    ) -> list[FakeRow]:
        self.effects.append({
            "kind": "find_many", "take": take, "skip": skip, "order": order, "where": where,
        })
        return self.page

    async def delete(self, where: dict[str, JsonValue]) -> FakeRow | None:
        self.effects.append({"kind": "delete", "where": where})
        return self.rows.pop(TEXT.validate_python(where["skill_id"]), None)


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


def upload_of(entry: list[JsonValue]) -> Upload:
    file_name = TEXT.validate_python(entry[0])
    file_content = base64.b64decode(TEXT.validate_python(entry[1]))
    if len(entry) == TWO:
        return (file_name, file_content)
    return (file_name, file_content, OPTIONAL_TEXT.validate_python(entry[2]))


def error_token(error: ValueError) -> str:
    message = str(error)
    for prefix, code in (
        (NO_DATABASE, "no_database"), (MISSING_IDENTITY, "missing_identity"),
        (NOT_FOUND, "not_found"),
    ):
        if message.startswith(prefix):
            return code
    return "exception"


def settled_json(pending: Pending) -> dict[str, JsonValue]:
    match pending:
        case Coroutine():
            return OBJECT.validate_json(asyncio.run(pending).model_dump_json())
        case Settled():
            return OBJECT.validate_json(pending.model_dump_json())


def answer_of(action: str, pending: Pending) -> dict[str, JsonValue]:
    record = settled_json(pending)
    if action == "list":
        return {"action": action, **record}
    return {"action": action, "record": record}


def run_step(step: Step, handler: HostedHandler) -> JsonValue:
    action = step["action"]
    if action not in ("create", "list", "get", "delete"):
        return {"action": action, "error": "invalid_input"}
    skill_id = step.get("skill_id")
    logging_obj = Logging(
        model="fixture-model", messages=None, stream=False, call_type=action,
        start_time=datetime(2026, 1, 1, tzinfo=UTC), litellm_call_id="call-1", function_id="fn-1",
    )
    principal = principal_of(step.get("principal"))
    files = step.get("files")
    encoded = step.get("file_content")
    limit = step.get("limit")
    offset = step.get("offset")
    try:
        if action == "create":
            answer = answer_of(action, handler.create_skill_handler(
                display_title=step.get("display_title"), description=step.get("description"),
                instructions=step.get("instructions"), metadata=step.get("metadata"),
                files=None if files is None else [upload_of(entry) for entry in files],
                file_content=None if encoded is None else base64.b64decode(encoded),
                file_name=step.get("file_name"), file_type=step.get("file_type"),
                user_id=step.get("user_id"), user_api_key_dict=principal,
                _is_async=True, logging_obj=logging_obj,
                litellm_call_id=step.get("litellm_call_id"),
            ))
        elif action == "list":
            answer = answer_of(action, handler.list_skills_handler(
                limit=DEFAULT_LIMIT if limit is None else limit,
                offset=DEFAULT_OFFSET if offset is None else offset,
                _is_async=True, logging_obj=logging_obj,
                litellm_call_id=step.get("litellm_call_id"), user_api_key_dict=principal,
            ))
        elif skill_id is None:
            return {"action": action, "error": "invalid_input"}
        elif action == "get":
            answer = answer_of(action, handler.get_skill_handler(
                skill_id=skill_id, _is_async=True, logging_obj=logging_obj,
                litellm_call_id=step.get("litellm_call_id"), user_api_key_dict=principal,
            ))
        else:
            answer = answer_of(action, handler.delete_skill_handler(
                skill_id=skill_id, _is_async=True, logging_obj=logging_obj,
                litellm_call_id=step.get("litellm_call_id"), user_api_key_dict=principal,
            ))
    except ValueError as error:
        answer = {"action": action, "error": error_token(error)}
    details = RecordedLogging.model_validate(logging_obj).model_call_details
    return {**answer, "logging": {
        "model": details.model, "optional_params": details.optional_params,
        "litellm_params": details.litellm_params,
        "custom_llm_provider": details.custom_llm_provider,
        "litellm_call_id": details.litellm_call_id,
    }}


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
    handler: HostedHandler = LiteLLMSkillsTransformationHandler()
    steps: list[JsonValue] = [run_step(step, handler) for step in input_value["steps"]]
    return {"steps": steps, "effects": state.effects}
