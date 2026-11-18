import asyncio
import logging
from typing import Final, NotRequired, Protocol, TypedDict

import litellm
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.common_utils.discoverable_model_filter import (
    discoverable_rows,
    undiscoverable_model_names,
)
from litellm.proxy.common_utils.healthy_model_filter import get_hidden_unhealthy_model_names
from litellm.router import Router
from litellm.types.router import RouterModelGroupAliasItem
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

LOGGER_NAME: Final = "LiteLLM Proxy"
DEBUG_PREFIX: Final = "healthy-only model listing is enabled"
HEALTH_FAILURE: Final = "the health cache is unreachable"
BROKEN_PATTERN: Final = "broken-("
ROW_NAME: Final[TypeAdapter[str]] = TypeAdapter(str)
ALIASES: Final[TypeAdapter[dict[str, str | RouterModelGroupAliasItem]]] = TypeAdapter(
    dict[str, str | RouterModelGroupAliasItem]
)


class Health(TypedDict, total=False):
    unhealthy_names: list[str]
    fails: bool


class Arguments(TypedDict, total=False):
    model_names: list[str]
    user_role: str | None
    team_id: str | None
    healthy_only: bool | None
    model_list_healthy_only: bool | None
    with_router: bool | None
    health: Health
    broken_pattern: bool


class Input(TypedDict):
    function: str
    model_list: NotRequired[list[dict[str, JsonValue]]]
    model_group_alias: NotRequired[dict[str, JsonValue]]
    arguments: Arguments


class PatternRouterView(Protocol):
    patterns: dict[str, list[dict[str, JsonValue]]]


class DebugNotes(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.notes: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if message.startswith(DEBUG_PREFIX):
            self.notes.append(message)


class ReadyHealthRouter(Router):
    def __init__(self, *, unhealthy: set[str], fails: bool) -> None:
        super().__init__()
        self.unhealthy = unhealthy
        self.report_failure = fails
        self.health_reads = 0

    @override
    async def async_get_fully_unhealthy_model_names(  # ast-grep-ignore: no-long-names
        self,
    ) -> set[str]:
        self.health_reads += 1
        if self.report_failure:
            raise RuntimeError(HEALTH_FAILURE)
        return self.unhealthy


def rows_of(call: Input) -> list[dict[str, JsonValue]]:
    rows = call.get("model_list")
    return [] if rows is None else rows


def aliases_of(call: Input) -> dict[str, str | RouterModelGroupAliasItem]:
    declared = call.get("model_group_alias")
    return ALIASES.validate_python({} if declared is None else declared)


def caller(role: str | None) -> UserAPIKeyAuth:
    if role is None:
        return UserAPIKeyAuth()
    return UserAPIKeyAuth(user_role=LitellmUserRoles(role))


def broken_pattern_record() -> dict[str, JsonValue]:
    return {
        "model_name": "broken-*",
        "litellm_params": {"model": "openai/gpt-4o"},
        "model_info": {"discoverable": False},
    }


def snapshot_router(call: Input) -> Router:
    router = Router(model_list=rows_of(call), model_group_alias=aliases_of(call))
    if call["arguments"].get("broken_pattern") is True:
        view: PatternRouterView = router.pattern_router
        view.patterns[BROKEN_PATTERN] = [broken_pattern_record()]
    return router


def router_of(call: Input) -> Router | None:
    disabled = call["arguments"].get("with_router")
    if disabled is False:
        return None
    return snapshot_router(call)


def settings_of(arguments: Arguments) -> dict[str, JsonValue]:
    enabled = arguments.get("model_list_healthy_only")
    return {"model_list_healthy_only": False if enabled is None else enabled}


def health_router(arguments: Arguments) -> ReadyHealthRouter | None:
    disabled = arguments.get("with_router")
    if disabled is False:
        return None
    health = arguments.get("health")
    if health is None:
        return ReadyHealthRouter(unhealthy=set(), fails=False)
    unhealthy = health.get("unhealthy_names")
    fails = health.get("fails")
    return ReadyHealthRouter(
        unhealthy=set() if unhealthy is None else set(unhealthy),
        fails=False if fails is None else fails,
    )


def undiscoverable(call: Input) -> JsonValue:
    arguments = call["arguments"]
    asked = arguments.get("model_names")
    model_names: list[str] = [] if asked is None else asked
    hidden = undiscoverable_model_names(
        model_names,
        router_of(call),
        caller(arguments.get("user_role")),
        arguments.get("team_id"),
    )
    names: list[JsonValue] = []
    names.extend(sorted(hidden))
    return {"hidden": names}


def discoverable(call: Input) -> JsonValue:
    rows = rows_of(call)
    kept = discoverable_rows(rows, caller(call["arguments"].get("user_role")))
    names: list[JsonValue] = []
    names.extend(ROW_NAME.validate_python(row["model_name"]) for row in kept)
    return {"rows": names}


def healthy(call: Input) -> JsonValue:
    arguments = call["arguments"]
    router = health_router(arguments)
    logger = logging.getLogger(LOGGER_NAME)
    notes = DebugNotes()
    saved_level = logger.level
    saved_handlers = logger.handlers[:]
    active: list[logging.Handler] = [notes]
    logger.handlers = active
    logger.setLevel(logging.DEBUG)
    try:
        hidden = asyncio.run(
            get_hidden_unhealthy_model_names(
                arguments.get("healthy_only"), settings_of(arguments), router
            )
        )
    finally:
        logger.handlers = saved_handlers
        logger.setLevel(saved_level)
    names: list[JsonValue] = []
    names.extend(sorted(hidden))
    debug: list[JsonValue] = []
    debug.extend(notes.notes)
    reads = 0 if router is None else router.health_reads
    return {"hidden": names, "health_reads": reads, "debug": debug}


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    function = input_value["function"]
    if function == "undiscoverable_model_names":
        return undiscoverable(input_value)
    if function == "discoverable_rows":
        return discoverable(input_value)
    if function == "hidden_unhealthy_model_names":
        return healthy(input_value)
    raise ValueError(f"unsupported model_visibility function {function}")
