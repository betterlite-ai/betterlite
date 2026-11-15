import asyncio
import base64
import logging
import sys
from types import ModuleType
from typing import Required, TypedDict

from litellm.llms.litellm_proxy.skills.handler import LiteLLMSkillsHandler
from litellm.models.skills import LiteLLM_SkillsTable
from litellm.proxy._types import LitellmUserRoles, NewSkillRequest, UserAPIKeyAuth
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

PROXY_MODULE = "litellm.proxy.proxy_server"
PRISMA_MODULE = "prisma"
FIELDS_MODULE = "prisma.fields"
SKILL_PREFIX = "litellm_skill_"
LOGGER_NAME = "LiteLLM"
NO_DATABASE = "Prisma client is not initialized. Database connection required for LiteLLM skills."
MISSING_IDENTITY = "Unable to record skill ownership: caller has no identity scope."
NOT_FOUND = "Skill not found: "
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
TEXT: TypeAdapter[str] = TypeAdapter(str)


class PrincipalFields(TypedDict, total=False):
    user_id: str
    team_id: str
    org_id: str
    api_key: str
    token: str
    admin: bool


class CreateData(TypedDict, total=False):
    display_title: str | None
    description: str | None
    instructions: str | None
    metadata: dict[str, JsonValue] | None
    file_content: str | None
    file_name: str | None
    file_type: str | None


class RowFields(CreateData, total=False):
    skill_id: Required[str]
    source: str | None
    latest_version: str | None
    created_at: str | None
    created_by: str | None
    updated_at: str | None
    updated_by: str | None


class Step(TypedDict, total=False):
    action: Required[str]
    data: CreateData
    user_id: str
    principal: PrincipalFields
    skill_id: str
    limit: int
    offset: int


class Input(TypedDict, total=False):
    now: Required[str]
    uuid: Required[str]
    rows: list[RowFields]
    page: list[RowFields]
    faults: dict[str, bool]
    steps: Required[list[Step]]


class FakeRow(dict[str, JsonValue]):
    @property
    def created_by(self) -> JsonValue:
        return self.get("created_by")

    def model_dump(self) -> dict[str, JsonValue]:
        return dict(self)


def make_row(fields: RowFields | dict[str, JsonValue], now: str) -> FakeRow:
    row = FakeRow(OBJECT.validate_python(fields))
    for key, value in (("source", "custom"), ("created_at", now), ("updated_at", now)):
        if row.get(key) is None:
            row[key] = value
    return row


class TableState:
    def __init__(self, input_value: Input) -> None:
        self.now = input_value["now"]
        self.fixed_id = f"{SKILL_PREFIX}{input_value['uuid']}"
        self.effects: list[JsonValue] = []
        self.rows: dict[str, FakeRow] = {}
        self.page: list[FakeRow] = []
        seeded = input_value.get("rows")
        if seeded is not None:
            self.rows = {fields["skill_id"]: make_row(fields, self.now) for fields in seeded}
        listed = input_value.get("page")
        if listed is not None:
            self.page = [make_row(fields, self.now) for fields in listed]
        faults = input_value.get("faults")
        self.faults: dict[str, bool] = {} if faults is None else faults

    def fail(self, kind: str) -> None:
        if self.faults.get(kind):
            raise RuntimeError(f"the scripted repository failed the {kind}")

    async def create(self, data: dict[str, JsonValue]) -> FakeRow:
        stored = {**data, "skill_id": self.fixed_id}
        if "metadata" in stored:
            stored["metadata"] = OBJECT.validate_json(TEXT.validate_python(stored["metadata"]))
        self.effects.append(JSON.validate_python({"kind": "create", "data": stored}))
        self.fail("create")
        row = make_row(stored, self.now)
        if row.get("metadata") is None:
            row["metadata"] = {}
        self.rows[self.fixed_id] = row
        return row

    async def find_unique(self, where: dict[str, str]) -> FakeRow | None:
        self.effects.append(JSON.validate_python({"kind": "find_unique", "where": where}))
        self.fail("find_unique")
        return self.rows.get(where["skill_id"])

    async def find_many(
        self, take: int, skip: int, order: JsonValue, where: JsonValue
    ) -> list[FakeRow]:
        effect = {"kind": "find_many", "take": take, "skip": skip, "order": order, "where": where}
        self.effects.append(JSON.validate_python(effect))
        self.fail("find_many")
        return self.page

    async def delete(self, where: dict[str, str]) -> FakeRow | None:
        self.effects.append(JSON.validate_python({"kind": "delete", "where": where}))
        self.fail("delete")
        return self.rows.pop(where["skill_id"], None)


class FakeClient:
    def __init__(self, table: TableState) -> None:
        self.db: FakeClient = self
        self.litellm_skillstable = table


class FakeBase64:
    @staticmethod
    def encode(data: bytes) -> str:
        encoded = base64.b64encode(data)
        return encoded.decode("ascii")


class FakeModule(ModuleType):
    Base64 = FakeBase64
    fields: ModuleType | None = None
    prisma_client: FakeClient | None = None


class WarningRecorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[JsonValue] = []

    @override
    def emit(self, _record: logging.LogRecord) -> None:
        self.messages.append({"level": "warn"})


def principal_of(fields: PrincipalFields) -> UserAPIKeyAuth:
    # The handler receives resolved auth fields; construction would replace token with the api key.
    principal = UserAPIKeyAuth(
        user_id=fields.get("user_id"),
        team_id=fields.get("team_id"),
        org_id=fields.get("org_id"),
        user_role=LitellmUserRoles.PROXY_ADMIN if fields.get("admin") else None,
    )
    principal.api_key = fields.get("api_key")
    principal.token = fields.get("token")
    return principal


def new_request(fields: CreateData) -> NewSkillRequest:
    content = fields.get("file_content")
    return NewSkillRequest(
        display_title=fields.get("display_title"),
        description=fields.get("description"),
        instructions=fields.get("instructions"),
        metadata=fields.get("metadata"),
        file_content=None if content is None else base64.b64decode(content),
        file_name=fields.get("file_name"),
        file_type=fields.get("file_type"),
    )


def row_value(row: LiteLLM_SkillsTable) -> JsonValue:
    value = OBJECT.validate_json(row.model_dump_json(exclude={"file_content"}))
    content = row.file_content
    value["file_content"] = None if content is None else base64.b64encode(content).decode("ascii")
    return value


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


async def run_step(step: Step) -> JsonValue:
    action = step["action"]
    fields = step.get("principal")
    principal = None if fields is None else principal_of(fields)
    try:
        if action == "create":
            return await create_answer(step, principal)
        if action in {"list", "search"}:
            return await list_answer(step, principal)
        if action in {"get", "delete", "fetch"}:
            return await skill_answer(step, principal)
    except ValueError as error:
        return {"action": action, "error": error_token(error)}
    except RuntimeError:
        return {"action": action, "error": "exception"}
    return {"action": action, "error": "invalid_input"}


async def create_answer(step: Step, principal: UserAPIKeyAuth | None) -> JsonValue:
    data = step.get("data")
    if data is None:
        return {"action": step["action"], "error": "invalid_input"}
    row = await LiteLLMSkillsHandler.create_skill(
        new_request(data), user_id=step.get("user_id"), user_api_key_dict=principal
    )
    return {"action": step["action"], "row": row_value(row)}


async def list_answer(step: Step, principal: UserAPIKeyAuth | None) -> JsonValue:
    if step["action"] == "search":
        found = await LiteLLMSkillsHandler.list_skills_for_search(user_api_key_dict=principal)
    else:
        limit = step.get("limit")
        offset = step.get("offset")
        page = (20 if limit is None else limit, 0 if offset is None else offset)
        found = await LiteLLMSkillsHandler.list_skills(
            limit=page[0], offset=page[1], user_api_key_dict=principal
        )
    return {"action": step["action"], "rows": [row_value(row) for row in found]}


async def skill_answer(step: Step, principal: UserAPIKeyAuth | None) -> JsonValue:
    action = step["action"]
    skill_id = step.get("skill_id")
    if skill_id is None:
        return {"action": action, "error": "invalid_input"}
    if action == "get":
        skill = await LiteLLMSkillsHandler.get_skill(skill_id, user_api_key_dict=principal)
        return {"action": action, "row": row_value(skill)}
    if action == "delete":
        deleted = await LiteLLMSkillsHandler.delete_skill(skill_id, user_api_key_dict=principal)
        return {"action": action, "result": {"id": skill_id, "type": deleted["type"]}}
    fetched = await LiteLLMSkillsHandler.fetch_skill_from_db(
        skill_id, user_api_key_dict=principal
    )
    return {"action": action, "row": None if fetched is None else row_value(fetched)}


async def run_steps(steps: list[Step]) -> list[JsonValue]:
    results: list[JsonValue] = [await run_step(step) for step in steps]
    return results


def run(input_value: Input) -> JsonValue:
    state = TableState(input_value)
    proxy = FakeModule(PROXY_MODULE)
    proxy.prisma_client = FakeClient(state)
    fields = FakeModule(FIELDS_MODULE)
    prisma = FakeModule(PRISMA_MODULE)
    prisma.fields = fields
    recorder = WarningRecorder()
    logging.getLogger(LOGGER_NAME).handlers = [recorder]
    logging.getLogger(LOGGER_NAME).setLevel(logging.WARNING)
    sys.modules[PROXY_MODULE] = proxy
    sys.modules[PRISMA_MODULE] = prisma
    sys.modules[FIELDS_MODULE] = fields
    return {
        "steps": JSON.validate_python(asyncio.run(run_steps(input_value["steps"]))),
        "effects": JSON.validate_python(state.effects),
        "warnings": JSON.validate_python(recorder.messages),
    }
