import asyncio
import logging
from typing import Final, NotRequired, TypedDict

import litellm
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.common_utils.discoverable_model_filter import (
    discoverable_rows,
    undiscoverable_model_names,
)
from litellm.proxy.common_utils.healthy_model_filter import get_hidden_unhealthy_model_names
from litellm.router import Router
from litellm.router_utils.pattern_match_deployments import PatternMatchRouter
from litellm.types.router import RouterModelGroupAliasItem
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

LOGGER_NAME: Final = "LiteLLM Proxy"
DEBUG_PREFIX: Final = "healthy-only model listing is enabled"
HEALTH_FAILURE: Final = "the health cache is unreachable"
BROKEN_KEY: Final = "broken-("
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


class InjectedModelInfo(TypedDict):
    discoverable: bool


class InjectedRow(TypedDict):
    model_info: InjectedModelInfo


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


# The raw invalid regex an internal record can hold; LiteLLM compiles it during a listing.
class BrokenPatternRouter(PatternMatchRouter):
    def __init__(self) -> None:
        rows: dict[str, list[InjectedRow]] = {
            BROKEN_KEY: [InjectedRow(model_info=InjectedModelInfo(discoverable=False))]
        }
        self.patterns = rows


def rows_of(call: Input) -> list[dict[str, JsonValue]]:
    rows = call.get("model_list")
    return [] if rows is None else rows


def aliases_of(call: Input) -> dict[str, str | RouterModelGroupAliasItem]:
    declared = call.get("model_group_alias")
    entries: dict[str, JsonValue] = {} if declared is None else declared
    return ALIASES.validate_python(entries)


def caller(role: str | None) -> UserAPIKeyAuth:
    if role is None:
        return UserAPIKeyAuth()
    return UserAPIKeyAuth(user_role=LitellmUserRoles(role))


def router_of(call: Input) -> Router | None:
    arguments = call["arguments"]
    disabled = arguments.get("with_router")
    if disabled is False:
        return None
    router = Router(model_list=rows_of(call), model_group_alias=aliases_of(call))
    if arguments.get("broken_pattern") is not True:
        return router
    router.pattern_router = BrokenPatternRouter()
    return router


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


def asked_hidden(call: Input, model_names: list[str], router: Router | None) -> frozenset[str]:
    arguments = call["arguments"]
    return undiscoverable_model_names(
        model_names,
        router,
        caller(arguments.get("user_role")),
        arguments.get("team_id"),
    )


def undiscoverable(call: Input) -> JsonValue:
    arguments = call["arguments"]
    asked = arguments.get("model_names")
    model_names: list[str] = [] if asked is None else asked
    hidden = asked_hidden(call, model_names, router_of(call))
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
