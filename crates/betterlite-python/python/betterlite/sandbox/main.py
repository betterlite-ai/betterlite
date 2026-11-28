from collections.abc import Mapping, Sequence
from typing import ClassVar, Final

from pydantic import ConfigDict, Field, JsonValue, PrivateAttr, TypeAdapter

from betterlite._endpoints import CallPayload, CallValue, to_call_payload
from betterlite._native import NativeDecisionError, NativeDecisionSession
from betterlite.decisions.main import JSON, parse_response, raise_decision_error
from betterlite.types.llms.base import LiteLLMPydanticObjectBase

CREATE: Final = "create_sandbox"
RUN: Final = "run_code"
DELETE: Final = "delete_sandbox"
CODE_INTERPRETER: Final = "code_interpreter"
INTERNAL: Final[tuple[str, ...]] = (
    "litellm_call_id",
    "litellm_logging_obj",
    "litellm_metadata",
    "litellm_trace_id",
)
BOOKKEEPING: Final[tuple[str, ...]] = ("client", "logger_fn", "shared_session")
CREATE_KEYS: Final[tuple[str, ...]] = (
    "entrypoint",
    "env_vars",
    "metadata",
    "network_policy",
    "poll_interval",
    "ready_timeout",
    "resource_limits",
    "resource_requests",
    "secure_access",
    "use_server_proxy",
)
RUN_KEYS: Final[tuple[str, ...]] = (
    "api_base",
    "env_vars",
    "language",
    "poll_interval",
    "ready_timeout",
    "use_server_proxy",
)
INTERPRETER_KEYS: Final[tuple[str, ...]] = ("allow_internet_access", *CREATE_KEYS, "language")


class ContainerHandle(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", protected_namespaces=())

    id: str
    provider: str
    domain: str | None = None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    @property
    def hidden_params(self) -> dict[str, JsonValue]:
        return self._hidden_params

    def set_hidden_params(self, params: Mapping[str, JsonValue]) -> None:
        self._hidden_params.update(params)


class CodeExecutionResult(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", protected_namespaces=())

    stdout: str = ""
    stderr: str = ""
    results: list[dict[str, JsonValue]] = Field(default_factory=list)
    error: dict[str, JsonValue] | None = None
    execution_count: int | None = None
    object: str = "code_execution"
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    @property
    def hidden_params(self) -> dict[str, JsonValue]:
        return self._hidden_params

    def set_hidden_params(self, params: Mapping[str, JsonValue]) -> None:
        self._hidden_params.update(params)


CONTAINER: Final[TypeAdapter[ContainerHandle]] = TypeAdapter(ContainerHandle)
RESULT: Final[TypeAdapter[CodeExecutionResult]] = TypeAdapter(CodeExecutionResult)
BOOL: Final[TypeAdapter[bool]] = TypeAdapter(bool)


def container_value(container: ContainerHandle | str) -> JsonValue:
    match container:
        case ContainerHandle():
            fields: dict[str, JsonValue] = {
                "id": container.id,
                "provider": container.provider,
                "domain": container.domain,
                "hidden_params": dict(container.hidden_params),
            }
            return JSON.validate_python(fields)
        case str() as sandbox_id:
            return JSON.validate_python(sandbox_id)


def sandbox_values(
    provider: str,
    named: Mapping[str, CallValue],
    kwargs: Mapping[str, CallValue],
    known: Sequence[str],
) -> dict[str, CallValue]:
    lifted = {name: kwargs[name] for name in known if name in kwargs}
    params = {
        name: value
        for name, value in kwargs.items()
        if name not in INTERNAL and name not in known and name not in BOOKKEEPING
    }
    values: dict[str, CallValue] = {"provider": provider, **named, **lifted, "params": params}
    values |= {name: kwargs[name] for name in BOOKKEEPING if name in kwargs}
    return values


def sandbox_payload(operation: str, values: Mapping[str, CallValue]) -> CallPayload:
    arguments = {name: value for name, value in values.items() if name not in BOOKKEEPING}
    payload: dict[str, CallValue] = {
        "arguments": arguments,
        "client": values.get("client"),
        "logger_fn": values.get("logger_fn"),
        "shared_session": values.get("shared_session"),
        "operation": operation,
    }
    return to_call_payload(payload)


async def acall_sandbox(
    session: NativeDecisionSession, operation: str, values: Mapping[str, CallValue]
) -> JsonValue:
    payload = sandbox_payload(operation, values)
    try:
        answer = await session.acall(payload.text, payload.blobs)
    except NativeDecisionError as error:
        raise_decision_error(error)
    return JSON.validate_json(answer)


async def acreate_sandbox(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/sandbox/main.py#L65
    provider: str,
    template: str | None = None,
    timeout: int | None = None,
    allow_internet_access: bool | None = None,  # noqa: FBT001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/sandbox/main.py#L69
    api_key: str | None = None,
    api_base: str | None = None,
    **kwargs: CallValue,
) -> ContainerHandle:
    named: dict[str, CallValue] = {
        "allow_internet_access": allow_internet_access,
        "api_base": api_base,
        "api_key": api_key,
        "create_timeout": timeout,
        "template": template,
    }
    answer = await acall_sandbox(
        NativeDecisionSession("{}"),
        CREATE,
        sandbox_values(provider, named, kwargs, CREATE_KEYS),
    )
    return parse_response(answer, CONTAINER)


async def arun_code(
    provider: str,
    container: ContainerHandle | str,
    code: str,
    api_key: str | None = None,
    **kwargs: CallValue,
) -> CodeExecutionResult:
    named: dict[str, CallValue] = {
        "api_key": api_key,
        "code": code,
        "container": container_value(container),
    }
    session = NativeDecisionSession("{}")
    values = sandbox_values(provider, named, kwargs, RUN_KEYS)
    return parse_response(await acall_sandbox(session, RUN, values), RESULT)


async def adelete_sandbox(
    provider: str,
    container: ContainerHandle | str,
    api_key: str | None = None,
    api_base: str | None = None,
    **kwargs: CallValue,
) -> bool:
    named: dict[str, CallValue] = {
        "api_base": api_base,
        "api_key": api_key,
        "container": container_value(container),
    }
    values = sandbox_values(provider, named, kwargs, ())
    return BOOL.validate_python(await acall_sandbox(NativeDecisionSession("{}"), DELETE, values))


async def acode_interpreter_tool(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/sandbox/main.py#L120
    provider: str,
    code: str,
    template: str | None = None,
    timeout: int | None = None,
    api_key: str | None = None,
    api_base: str | None = None,
    **kwargs: CallValue,
) -> CodeExecutionResult:
    named: dict[str, CallValue] = {
        "api_base": api_base,
        "api_key": api_key,
        "code": code,
        "create_timeout": timeout,
        "template": template,
    }
    values = sandbox_values(provider, named, kwargs, INTERPRETER_KEYS)
    session = NativeDecisionSession("{}")
    return parse_response(await acall_sandbox(session, CODE_INTERPRETER, values), RESULT)


__all__ = ["acode_interpreter_tool", "acreate_sandbox", "adelete_sandbox", "arun_code"]
