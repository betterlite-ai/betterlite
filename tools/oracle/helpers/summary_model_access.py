import asyncio
import logging
import sys
from collections.abc import Mapping
from types import ModuleType
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict, TypeVar

from litellm.llms.anthropic.pass_through.context_management.editors import compact
from litellm.models.project import LiteLLM_ProjectTable
from litellm.models.team_membership import LiteLLM_TeamMembership
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._types import ProxyException, UserAPIKeyAuth
from litellm.proxy.auth import auth_checks
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
CHECKS_MODULE: Final = "litellm.proxy.auth.auth_checks"
CHECK: Final = compact._check_summary_model_access  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L197
READ_MODEL: Final = compact._read_summary_model_setting  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L170
READ_TOKENS: Final = compact._read_summary_max_tokens_setting  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L180
OBJECT_CHECK: Final = auth_checks._can_object_call_model  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L4423
PROJECT_CHECK: Final = auth_checks.can_project_access_model  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5249
USER_CHECK: Final = auth_checks.can_user_call_model  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5269


class UserRowJson(TypedDict):
    models: NotRequired[list[str]]


PRINCIPALS: Final = TypeAdapter(UserAPIKeyAuth)
USER_ROWS: Final = TypeAdapter(LiteLLM_UserTable)
PROJECT_ROWS: Final = TypeAdapter(LiteLLM_ProjectTable)
MEMBERSHIPS: Final = TypeAdapter(LiteLLM_TeamMembership)
USER_MODELS: Final = TypeAdapter(UserRowJson)
JSON: Final = TypeAdapter(JsonValue)

Row = TypeVar("Row")


class LookupScript(TypedDict):
    kind: Literal["missing", "failed", "found"]
    row: NotRequired[dict[str, JsonValue]]


class SettingsScenario(TypedDict):
    general_settings: dict[str, JsonValue] | None


class AccessScenario(TypedDict):
    principal: dict[str, JsonValue] | None
    proxy: bool
    database: bool
    checks: Literal["scripted", "real"]
    outcomes: list[Literal["allow", "deny", "broken"]]
    user: LookupScript
    project: LookupScript
    membership: LookupScript


class SettingsCall(TypedDict):
    action: Literal["settings"]
    scenarios: list[SettingsScenario]


class AccessCall(TypedDict):
    action: Literal["access"]
    summary_model: str
    scenarios: list[AccessScenario]


Input: TypeAlias = SettingsCall | AccessCall


class CallJson(TypedDict):
    call: str
    object_type: str | None
    model: str | None
    models: list[str]
    team_model_aliases: dict[str, str]
    team_id: str | None
    user_id: str | None
    user_id_upsert: bool | None
    project_id: str | None


class ScriptedError(Exception):
    pass


class ProxyModule(ModuleType):
    def __init__(self, *, database: bool, general_settings: dict[str, JsonValue] | None) -> None:
        super().__init__(PROXY_MODULE)
        if general_settings is not None:
            self.general_settings = general_settings
        # the check only tests prisma_client for None, so any object stands in for the client
        self.prisma_client = object() if database else None
        self.proxy_logging_obj = None
        self.user_api_key_cache = None


class Diagnostics(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.DEBUG)
        self.lines: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        recorded = record.levelno == logging.DEBUG or record.levelno >= logging.WARNING
        if recorded and message.startswith("compact_20260112:"):
            level = "debug" if record.levelno == logging.DEBUG else "warning"
            entry: dict[str, JsonValue] = {"level": level, "message": message}
            self.lines.append(JSON.validate_python(entry))


class ChecksModule(ModuleType):
    def __init__(self, scenario: AccessScenario, journal: list[CallJson]) -> None:
        super().__init__(CHECKS_MODULE)
        self.real = scenario["checks"] == "real"
        self.outcomes = list(scenario["outcomes"])
        self.journal = journal
        self.user = scenario["user"]
        self.project = scenario["project"]
        self.membership = scenario["membership"]

    def verdict(self) -> Literal[True]:
        outcome = self.outcomes.pop(0) if self.outcomes else "allow"
        if outcome == "deny":
            raise ProxyException("summary model denied", "model_access_denied", "model")
        if outcome == "broken":
            raise ScriptedError("broken")
        return True

    def _can_object_call_model(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L4423
        self,
        model: str,
        llm_router: None,
        models: list[str],
        team_model_aliases: dict[str, str] | None = None,
        team_id: str | None = None,
        key_model_aliases: Mapping[str, str] | None = None,
        object_type: Literal["user", "team", "key", "org", "project", "agent"] = "user",
        fallback_depth: int = 0,
    ) -> Literal[True]:
        record = blank("object", model, list(models))
        record["object_type"] = object_type
        aliases: dict[str, str] = {}
        if team_model_aliases is not None:
            aliases = dict(team_model_aliases)
        record["team_model_aliases"] = aliases
        record["team_id"] = team_id
        self.journal.append(record)
        if self.real:
            return OBJECT_CHECK(
                model=model, llm_router=llm_router, models=models,
                team_model_aliases=team_model_aliases, team_id=team_id,
                key_model_aliases=key_model_aliases, object_type=object_type,
                fallback_depth=fallback_depth,
            )
        return self.verdict()

    async def can_user_call_model(
        self,
        model: str,
        llm_router: None,
        user_object: LiteLLM_UserTable | None,
        key_model_aliases: Mapping[str, str] | None = None,
    ) -> Literal[True]:
        self.journal.append(blank("user", model, user_models(self.user)))
        if self.real:
            return await USER_CHECK(
                model=model, llm_router=llm_router, user_object=user_object,
                key_model_aliases=key_model_aliases,
            )
        return self.verdict()

    def can_project_access_model(
        self,
        model: str,
        project_object: LiteLLM_ProjectTable,
        llm_router: None,
        key_model_aliases: Mapping[str, str] | None = None,
    ) -> Literal[True]:
        self.journal.append(blank("project", model, list(project_object.models)))
        if self.real:
            return PROJECT_CHECK(
                model=model, project_object=project_object, llm_router=llm_router,
                key_model_aliases=key_model_aliases,
            )
        return self.verdict()

    async def get_user_object(
        self, user_id: str, *, user_id_upsert: bool, **_context: JsonValue
    ) -> LiteLLM_UserTable | None:
        record = blank("lookup_user")
        record["user_id"] = user_id
        record["user_id_upsert"] = user_id_upsert
        self.journal.append(record)
        return resolve_row(self.user, USER_ROWS)

    async def get_project_object(
        self, project_id: str, **_context: JsonValue
    ) -> LiteLLM_ProjectTable | None:
        record = blank("lookup_project")
        record["project_id"] = project_id
        self.journal.append(record)
        return resolve_row(self.project, PROJECT_ROWS)

    async def get_team_membership(
        self, user_id: str, team_id: str, **_context: JsonValue
    ) -> LiteLLM_TeamMembership | None:
        record = blank("lookup_membership", team_id=team_id)
        record["user_id"] = user_id
        self.journal.append(record)
        return resolve_row(self.membership, MEMBERSHIPS)


def blank(
    call: str, model: str | None = None, models: list[str] | None = None, team_id: str | None = None,
) -> CallJson:
    return CallJson(
        call=call, object_type=None, model=model, models=[] if models is None else models,
        team_model_aliases={}, team_id=team_id, user_id=None, user_id_upsert=None, project_id=None,
    )


def user_models(script: LookupScript) -> list[str]:
    if script["kind"] != "found":
        return []
    row = USER_MODELS.validate_python(script.get("row"))
    if "models" not in row:
        return []
    return row["models"]


def resolve_row(script: LookupScript, rows: TypeAdapter[Row]) -> Row | None:
    if script["kind"] == "failed":
        raise ScriptedError("offline")
    if script["kind"] == "missing":
        return None
    return rows.validate_python(script.get("row"))


def install_proxy(
    *, present: bool, database: bool, general_settings: dict[str, JsonValue] | None
) -> None:
    if not present:
        sys.modules[PROXY_MODULE] = ModuleType(PROXY_MODULE)
        return
    sys.modules[PROXY_MODULE] = ProxyModule(database=database, general_settings=general_settings)


def settings_answer(input_value: SettingsCall) -> list[JsonValue]:
    outcomes: list[JsonValue] = []
    for scenario in input_value["scenarios"]:
        general_settings = scenario["general_settings"]
        install_proxy(present=general_settings is not None, database=False, general_settings=general_settings)
        outcome: dict[str, JsonValue] = {"model": READ_MODEL(), "max_tokens": READ_TOKENS()}
        outcomes.append(outcome)
    return outcomes


def access_answer(input_value: AccessCall) -> list[JsonValue]:
    logger = logging.getLogger("LiteLLM")
    # LiteLLM's own handler prints debug records to stdout, so it is detached for the run
    detached, level = logger.handlers, logger.level
    logger.handlers = []
    logger.setLevel(logging.DEBUG)
    diagnostics = Diagnostics()
    logger.addHandler(diagnostics)
    outcomes: list[JsonValue] = []
    try:
        for scenario in input_value["scenarios"]:
            fields = scenario["principal"]
            principal = None if fields is None else PRINCIPALS.validate_python(fields)
            install_proxy(present=scenario["proxy"], database=scenario["database"], general_settings=None)
            journal: list[CallJson] = []
            sys.modules[CHECKS_MODULE] = ChecksModule(scenario, journal)
            diagnostics.lines = []
            allowed = asyncio.run(CHECK(principal, input_value["summary_model"], None))
            outcome: dict[str, JsonValue] = {
                "allowed": allowed,
                "calls": JSON.validate_python(journal),
                "diagnostics": JSON.validate_python(diagnostics.lines),
            }
            outcomes.append(JSON.validate_python(outcome))
    finally:
        logger.setLevel(level)
        logger.handlers = detached
    return outcomes


def run(input_value: Input) -> JsonValue:
    if input_value["action"] == "settings":
        answer: dict[str, JsonValue] = {"outcomes": settings_answer(input_value)}
    else:
        answer = {"outcomes": access_answer(input_value)}
    return JSON.validate_python(answer)
