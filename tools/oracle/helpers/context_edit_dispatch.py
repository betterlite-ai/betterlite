import asyncio
import logging
from dataclasses import asdict
from typing import Final, Literal, Protocol, TypedDict, cast

from litellm.llms.anthropic.pass_through.context_management import (
    PolyfillResult,
    apply_context_management,
    dispatcher,
    errors,
)
from litellm.llms.anthropic.pass_through.context_management.constants import (
    CLEAR_TOOL_USES_EDIT_TYPE,
    COMPACT_EDIT_TYPE,
)
from litellm.types.llms.anthropic import AppliedEdit, CompactionBlock, UsageIteration
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

DIAGNOSTIC_PREFIX: Final = "context_management polyfill:"
ANTHROPIC_ERROR: Final = "AnthropicContextManagementError"
JSON_OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class Input(TypedDict):
    scenarios: list[JsonValue]


# one editor's answer, as the case scripts it: either native return shape or the refusal
class Script(BaseModel):
    model_config = ConfigDict(extra="forbid")

    kind: Literal["legacy", "polyfill", "error"]
    messages: list[dict[str, JsonValue]] = Field(default_factory=list)
    system: str | list[dict[str, JsonValue]] | None = None
    applied: AppliedEdit | None = None
    applied_edits: list[AppliedEdit] = Field(default_factory=list)
    compaction_block: CompactionBlock | None = None
    iterations_usage: list[UsageIteration] | None = None
    status_code: int = 0
    message: str = ""


class Scenario(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str = "main-model"
    messages: list[dict[str, JsonValue]]
    system: str | list[dict[str, JsonValue]] | None = None
    tools: list[JsonValue] | None = None
    context_management: dict[str, JsonValue] | list[JsonValue] | None = None
    litellm_metadata: dict[str, JsonValue] | None = None
    router: bool = False
    principal: bool = False
    clear: list[Script] = Field(default_factory=list)
    compact: list[Script] = Field(default_factory=list)


class Call(BaseModel):
    model_config = ConfigDict(extra="forbid")

    scenarios: list[Scenario]


CALL: Final[TypeAdapter[Call]] = TypeAdapter(Call)


# the two request-wide values a case marks present; the dispatcher passes them through
class Router(BaseModel):
    model_config = ConfigDict(extra="forbid")


class Principal(BaseModel):
    model_config = ConfigDict(extra="forbid")


class Dispatch(Protocol):
    async def __call__(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/dispatcher.py#L72-L82
        self,
        *,
        model: str,
        messages: list[dict[str, JsonValue]],
        tools: list[JsonValue] | None,
        system: str | list[dict[str, JsonValue]] | None,
        context_management_spec: dict[str, JsonValue] | list[JsonValue] | None,
        litellm_metadata: dict[str, JsonValue] | None = None,
        llm_router: Router | None = None,
        user_api_key_auth: Principal | None = None,
    ) -> PolyfillResult: ...


DISPATCH = cast(Dispatch, apply_context_management)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/dispatcher.py#L72


def take(
    scripts: list[Script],
) -> PolyfillResult | tuple[list[dict[str, JsonValue]], AppliedEdit | None]:
    if not scripts:
        raise RuntimeError("the case ran out of scripts")
    script = scripts.pop(0)
    if script.kind == "legacy":
        return (script.messages, script.applied)
    if script.kind == "polyfill":
        return PolyfillResult(
            messages=script.messages,
            system=script.system,
            applied_edits=script.applied_edits,
            compaction_block=script.compaction_block,
            iterations_usage=script.iterations_usage,
        )
    raise errors.AnthropicContextManagementError(
        status_code=script.status_code,
        message=script.message,
    )


# the compact editor's three request-wide arguments, marked present so the case can compare them
def marked_request_wide(
    arguments: dict[str, JsonValue],
) -> dict[str, JsonValue]:
    metadata = arguments.pop("litellm_metadata")
    router = arguments.pop("llm_router")
    principal = arguments.pop("user_api_key_auth")
    return {
        "litellm_metadata": metadata,
        "llm_router": None if router is None else "router",
        "user_api_key_auth": None if principal is None else "present",
    }


class ScriptedEditor:
    def __init__(self, clear: list[Script], compact: list[Script]) -> None:
        self.clear = clear
        self.compact = compact
        self.calls: list[dict[str, JsonValue]] = []

    # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/clear_tool_uses.py#L153-L160
    def clear_tool_uses(
        self, **arguments: JsonValue
    ) -> PolyfillResult | tuple[list[dict[str, JsonValue]], AppliedEdit | None]:
        return self.record(CLEAR_TOOL_USES_EDIT_TYPE, arguments, {}, self.clear)

    # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/dispatcher.py#L112-L132
    async def compact_uses(
        self, **arguments: JsonValue
    ) -> PolyfillResult | tuple[list[dict[str, JsonValue]], AppliedEdit | None]:
        request_wide = marked_request_wide(arguments)
        return self.record(COMPACT_EDIT_TYPE, arguments, request_wide, self.compact)

    def record(
        self,
        editor: str,
        arguments: dict[str, JsonValue],
        request_wide: dict[str, JsonValue],
        scripts: list[Script],
    ) -> PolyfillResult | tuple[list[dict[str, JsonValue]], AppliedEdit | None]:
        entry = {**arguments, "editor": editor, **request_wide}
        self.calls.append(JSON_OBJECT.validate_python(entry))
        return take(scripts)


class LogCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.lines: list[dict[str, JsonValue]] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if message.startswith(DIAGNOSTIC_PREFIX):
            entry = {"level": record.levelname.lower(), "message": message}
            self.lines.append(JSON_OBJECT.validate_python(entry))


async def call_dispatch(
    scenario: Scenario, router: Router | None, principal: Principal | None
) -> PolyfillResult:
    return await DISPATCH(
        model=scenario.model,
        messages=scenario.messages,
        tools=scenario.tools,
        system=scenario.system,
        context_management_spec=scenario.context_management,
        litellm_metadata=scenario.litellm_metadata,
        llm_router=router,
        user_api_key_auth=principal,
    )


def run_scenario(scenario: Scenario) -> dict[str, JsonValue]:
    editor = ScriptedEditor(scenario.clear, scenario.compact)
    # the registry the dispatcher consults: https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/dispatcher.py#L23-L26
    object.__setattr__(
        dispatcher,
        "_EDITOR_REGISTRY",
        {
            CLEAR_TOOL_USES_EDIT_TYPE: editor.clear_tool_uses,
            COMPACT_EDIT_TYPE: editor.compact_uses,
        },
    )
    capture = LogCapture()
    logger = logging.getLogger("LiteLLM")
    previous_handlers = logger.handlers
    previous_level = logger.level
    logger.handlers = [capture]
    logger.setLevel(logging.DEBUG)
    router = Router() if scenario.router else None
    principal = Principal() if scenario.principal else None
    result: dict[str, JsonValue] | None = None
    failure: dict[str, JsonValue] | None = None
    try:
        polyfilled = asyncio.run(call_dispatch(scenario, router, principal))
        result = JSON_OBJECT.validate_python(cast(dict[str, JsonValue], asdict(polyfilled)))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/result.py#L21
    except errors.AnthropicContextManagementError as error:
        failure = JSON_OBJECT.validate_python(
            {"class": ANTHROPIC_ERROR, "status_code": error.status_code, "message": error.message}
        )
    finally:
        logger.handlers = previous_handlers
        logger.setLevel(previous_level)
    return JSON_OBJECT.validate_python(
        {"result": result, "failure": failure, "calls": editor.calls, "diagnostics": capture.lines}
    )


def run(input_value: Input) -> JsonValue:
    call = CALL.validate_python(input_value)
    runs = [run_scenario(scenario) for scenario in call.scenarios]
    return JSON_OBJECT.validate_python({"runs": runs})
