import asyncio
import base64
import sys
from types import ModuleType
from typing import Required, TypedDict

from litellm.llms.litellm_proxy.skills.handler import LiteLLMSkillsHandler
from litellm.models.skills import LiteLLM_SkillsTable
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from pydantic import JsonValue, TypeAdapter

PROXY_MODULE = "litellm.proxy.proxy_server"
NO_DATABASE = "Prisma client is not initialized. Database connection required for LiteLLM skills."
NOT_FOUND = "Skill not found: "
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class PrincipalFields(TypedDict, total=False):
    user_id: str
    team_id: str
    org_id: str
    api_key: str
    token: str
    admin: bool


class RowFields(TypedDict, total=False):
    skill_id: Required[str]
    display_title: str | None
    description: str | None
    instructions: str | None
    source: str | None
    latest_version: str | None
    file_content: str | None
    file_name: str | None
    file_type: str | None
    metadata: dict[str, JsonValue] | None
    created_at: str | None
    created_by: str | None
    updated_at: str | None
    updated_by: str | None


class Step(TypedDict, total=False):
    action: Required[str]
    principal: PrincipalFields
    skill_id: str
    limit: int
    offset: int
    available: bool


class Input(TypedDict, total=False):
    now: Required[str]
    rows: list[RowFields]
    page: list[RowFields]
    steps: Required[list[Step]]


class FakeRow(dict[str, JsonValue]):
    @property
    def created_by(self) -> JsonValue:
        return self.get("created_by")

    def model_dump(self) -> dict[str, JsonValue]:
        return dict(self)


class ReadsTable:
    def __init__(self, input_value: Input) -> None:
        self.now = input_value["now"]
        self.effects: list[JsonValue] = []
        self.rows: dict[str, FakeRow] = {}
        self.page: list[FakeRow] = []
        seeded = input_value.get("rows")
        if seeded is not None:
            self.rows = {fields["skill_id"]: self.make(fields) for fields in seeded}
        listed = input_value.get("page")
        if listed is not None:
            self.page = [self.make(fields) for fields in listed]

    def make(self, fields: RowFields) -> FakeRow:
        row = FakeRow(OBJECT.validate_python(fields))
        if row.get("source") is None:
            row["source"] = "custom"
        if row.get("created_at") is None:
            row["created_at"] = self.now
        if row.get("updated_at") is None:
            row["updated_at"] = self.now
        return row

    async def find_unique(self, where: dict[str, str]) -> FakeRow | None:
        effect = {"kind": "find_unique", "where": where}
        self.effects.append(JSON.validate_python(effect))
        return self.rows.get(where["skill_id"])

    async def find_many(
        self, take: int, skip: int, order: JsonValue, where: JsonValue
    ) -> list[FakeRow]:
        effect = {"kind": "find_many", "take": take, "skip": skip, "order": order, "where": where}
        self.effects.append(JSON.validate_python(effect))
        return self.page


class FakeClient:
    def __init__(self, table: ReadsTable) -> None:
        self.db: FakeClient = self
        self.litellm_skillstable = table


class FakeModule(ModuleType):
    prisma_client: FakeClient | None = None


def principal_of(fields: PrincipalFields) -> UserAPIKeyAuth:
    return UserAPIKeyAuth.model_construct(
        user_id=fields.get("user_id"),
        team_id=fields.get("team_id"),
        org_id=fields.get("org_id"),
        api_key=fields.get("api_key"),
        token=fields.get("token"),
        user_role=LitellmUserRoles.PROXY_ADMIN if fields.get("admin") else None,
    )


def row_value(row: LiteLLM_SkillsTable) -> JsonValue:
    value = OBJECT.validate_json(row.model_dump_json(exclude={"file_content"}))
    content = row.file_content
    value["file_content"] = None if content is None else base64.b64encode(content).decode("ascii")
    return value


def error_token(error: ValueError) -> str:
    message = str(error)
    if message.startswith(NO_DATABASE):
        return "no_database"
    if message.startswith(NOT_FOUND):
        return "not_found"
    return "exception"


async def list_answer(step: Step, principal: UserAPIKeyAuth | None) -> JsonValue:
    limit = step.get("limit")
    offset = step.get("offset")
    found = await LiteLLMSkillsHandler.list_skills(
        limit=20 if limit is None else limit,
        offset=0 if offset is None else offset,
        user_api_key_dict=principal,
    )
    return {"action": "list", "rows": [row_value(row) for row in found]}


async def get_answer(step: Step, principal: UserAPIKeyAuth | None) -> JsonValue:
    skill_id = step.get("skill_id")
    if skill_id is None:
        return {"action": "get", "error": "invalid_input"}
    skill = await LiteLLMSkillsHandler.get_skill(skill_id, user_api_key_dict=principal)
    return {"action": "get", "row": row_value(skill)}


async def run_step(step: Step, table: ReadsTable, proxy: FakeModule) -> JsonValue:
    available = step.get("available")
    proxy.prisma_client = None if available is False else FakeClient(table)
    fields = step.get("principal")
    principal = None if fields is None else principal_of(fields)
    action = step["action"]
    try:
        if action == "list":
            return await list_answer(step, principal)
        if action == "get":
            return await get_answer(step, principal)
    except ValueError as error:
        return {"action": action, "error": error_token(error)}
    return {"action": action, "error": "invalid_input"}


async def run_steps(steps: list[Step], table: ReadsTable, proxy: FakeModule) -> list[JsonValue]:
    results: list[JsonValue] = [await run_step(step, table, proxy) for step in steps]
    return results


def run(input_value: Input) -> JsonValue:
    table = ReadsTable(input_value)
    proxy = FakeModule(PROXY_MODULE)
    proxy.prisma_client = FakeClient(table)
    sys.modules[PROXY_MODULE] = proxy
    steps = asyncio.run(run_steps(input_value["steps"], table, proxy))
    return {
        "steps": JSON.validate_python(steps),
        "effects": JSON.validate_python(table.effects),
    }
