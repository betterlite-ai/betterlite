import asyncio
import math
from collections.abc import Awaitable, Callable, Mapping
from contextlib import AbstractAsyncContextManager, suppress
from dataclasses import dataclass
from types import TracebackType
from typing import ClassVar, Literal, Protocol, Self, TypeAlias, TypeVar, cast
from unittest.mock import patch

import anyio
import httpx2
import litellm
from litellm._logging import verbose_logger
from litellm.constants import MCP_NPM_CACHE_DIR
from litellm.experimental_mcp_client import client as mcp_client
from litellm.types.mcp import MCPAuthType, MCPStdioConfig, MCPTransportType, MCPUpstreamProtocol
from mcp import MCPError
from mcp.types import InitializeResult
from pydantic import BaseModel, ConfigDict, Field

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Result = TypeVar("Result")
AuthType = Literal[
    "none",
    "api_key",
    "bearer_token",
    "basic",
    "authorization",
    "oauth2",
    "token",
    "oauth2_token_exchange",
]
Step = Literal["ok", "fail", "cancel", "timeout", "recorded_failure"]
Operation = Literal["ok", "protocol", "application", "cancel", "read_timeout", "relayed_timeout"]
StreamKind = Literal["value", "http", "io"]
Streams = tuple[object, object]
Transport = AbstractAsyncContextManager[Streams]


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class AuthSettings(Strict):
    auth_type: AuthType = "none"
    auth_value: str | dict[str, str] | None = None
    auth_header_name: str | None = None
    extra_headers: dict[str, str] = Field(default_factory=dict)
    resolved_auth: bool = False
    aws_auth: bool = False
    resolved_header_name: str | None = None


class StdioSettings(Strict):
    command: str = ""
    args: list[str] = Field(default_factory=list)
    env: dict[str, str] | None = None


class TlsSettings(Strict):
    verification: Literal["enabled", "disabled", "ca_file"] = "enabled"
    ca_file: str | None = None


class Settings(Strict):
    server_url: str = "https://mcp.example.com/mcp"
    transport_type: Literal["sse", "http", "stdio"] = "http"
    timeout_ms: int = 60_000
    stdio: StdioSettings | None = None
    auth: AuthSettings = Field(default_factory=AuthSettings)
    environment: dict[str, str] = Field(default_factory=dict)
    npm_cache_dir: str = MCP_NPM_CACHE_DIR
    tls: TlsSettings = Field(default_factory=TlsSettings)
    protocol_version: str = "auto"
    client_version: str = "1.105.1"


class TransportInput(Settings):
    operation: Literal["transport"]


class StdioInput(Strict):
    operation: Literal["stdio_env"]
    provided: dict[str, str] | None = None
    environment: dict[str, str] = Field(default_factory=dict)
    npm_cache_dir: str = MCP_NPM_CACHE_DIR


class InitializeInput(Strict):
    operation: Literal["initialize"]
    protocol_version: str = "auto"
    client_version: str = "1.105.1"
    callbacks: list[Literal["sampling", "elicitation", "logging"]] = Field(default_factory=list)
    server_version: str = "2025-06-18"


class RedirectCheck(Strict):
    url: str
    headers: dict[str, str] = Field(default_factory=dict)


class AuthVariant(Strict):
    sources: list[Literal["resolved", "aws", "explicit"]] = Field(default_factory=list)


class FactoryInput(Settings):
    operation: Literal["factory"]
    explicit_auth: bool = False
    redirect_checks: list[RedirectCheck] = Field(default_factory=list)
    auth_variants: list[AuthVariant] = Field(default_factory=list)


class Plan(Strict):
    transport_enter: Step = "ok"
    transport_exit: Step = "ok"
    session_enter: Step = "ok"
    session_exit: Step = "ok"
    initialize: Literal["ok", "fail"] = "ok"
    http_close: Step = "ok"
    server_version: str = "2025-06-18"
    instructions: str | None = None


class Scenario(Strict):
    mode: Literal["execute", "run"] = "execute"
    quiet: bool = False
    plan: Plan = Field(default_factory=Plan)
    http: bool = False
    operation: Operation = "ok"
    stream: StreamKind | None = None


class ExecuteInput(Strict):
    operation: Literal["execute"]
    settings: Settings
    scenarios: list[Scenario]


class ClientInfo(Protocol):
    name: str
    version: str


class InitializeParams(Protocol):
    protocol_version: str
    capabilities: object | None
    client_info: ClientInfo


class FakeAuth:
    def __init__(self, header_name: str | None) -> None:
        self.header_name = header_name


class FakeError(Exception):
    pass


class StreamError(FakeError):
    def __init__(self, kind: StreamKind) -> None:
        super().__init__(kind)
        self.kind = kind


class FakeHttpClient:
    def __init__(self, **kwargs: object) -> None:
        self.kwargs: dict[str, object] = kwargs
        self.cleanup_scope: object | None = None
        self.cleanup_errors: tuple[Exception, ...] = ()
        self.closed = False

    async def aclose(self) -> None:
        self.closed = True


REAL_SCOPE = anyio.CancelScope


class RecordingScope:
    created: ClassVar[list["RecordingScope"]] = []

    def __init__(self, deadline: float = math.inf, *, shield: bool = False) -> None:
        self._scope = REAL_SCOPE(deadline=deadline, shield=shield)
        RecordingScope.created.append(self)

    def __enter__(self) -> Self:
        self._scope.__enter__()
        return self

    def __exit__(
        self,
        kind: type[BaseException] | None,
        value: BaseException | None,
        trace: TracebackType | None,
    ) -> bool | None:
        return self._scope.__exit__(kind, value, trace)

    @property
    def shield(self) -> bool:
        return self._scope.shield

    @shield.setter
    def shield(self, value: bool) -> None:
        self._scope.shield = value

    @property
    def deadline(self) -> float:
        return self._scope.deadline

    @deadline.setter
    def deadline(self, value: float) -> None:
        self._scope.deadline = value

    @property
    def cancel_called(self) -> bool:
        return self._scope.cancel_called


def make_client(settings: Settings) -> mcp_client.MCPClient:
    resolved: httpx2.Auth | None = None
    if settings.auth.resolved_auth:
        resolved = cast(httpx2.Auth, FakeAuth(settings.auth.resolved_header_name))
    aws: httpx2.Auth | None = None
    if settings.auth.aws_auth:
        aws = cast(httpx2.Auth, FakeAuth(None))
    callbacks = no_callback
    return mcp_client.MCPClient(
        server_url=settings.server_url,
        transport_type=cast(MCPTransportType, settings.transport_type),
        auth_type=cast(
            MCPAuthType,
            None if settings.auth.auth_type == "none" else settings.auth.auth_type,
        ),
        auth_value=settings.auth.auth_value,
        auth_header_name=settings.auth.auth_header_name,
        timeout=settings.timeout_ms / 1000,
        stdio_config=(
            cast(MCPStdioConfig, settings.stdio.model_dump())
            if settings.stdio is not None
            else None
        ),
        extra_headers=settings.auth.extra_headers,
        resolved_auth=resolved,
        aws_auth=aws,
        sampling_callback=callbacks,
        elicitation_callback=callbacks,
        logging_callback=callbacks,
        protocol_version=cast(MCPUpstreamProtocol, settings.protocol_version),
    )


def private(client: object, name: str) -> object:
    return cast(object, getattr(client, "_" + name))


def no_callback(*_args: object, **_kwargs: object) -> None:
    return None


def auth_kind(value: object, resolved: object, aws: object, explicit: object) -> str:
    if value is explicit:
        return "explicit"
    if value is resolved:
        return "resolved"
    if value is aws:
        return "aws"
    return "none"


def hooks_of(value: object) -> Mapping[str, list[Callable[[httpx2.Request], Awaitable[None]]]]:
    if value is None:
        return {}
    return cast(Mapping[str, list[Callable[[httpx2.Request], Awaitable[None]]]], value)


@dataclass
class Handles:
    url: str
    slot: str | None
    resolved: object
    aws: object
    explicit: object


def config_json(recorder: FakeHttpClient, handles: Handles) -> Json:
    kwargs = recorder.kwargs
    timeout = cast(httpx2.Timeout, kwargs.get("timeout"))
    hooks = hooks_of(kwargs.get("event_hooks"))
    guard = hooks.get("request") or []
    return cast(
        Json,
        {
            "headers": cast(dict[str, str], kwargs.get("headers") or {}),
            "timeout_ms": round((timeout.read or 0.0) * 1000),
            "tls": cast(Json, kwargs.get("verify")),
            "auth": auth_kind(
                kwargs.get("auth"), handles.resolved, handles.aws, handles.explicit
            ),
            "follow_redirects": bool(kwargs.get("follow_redirects")),
            "redirect_guard": (
                {"configured_url": handles.url, "slot": handles.slot} if guard else None
            ),
            "capture_error_responses": bool(hooks.get("response")),
        },
    )


def transport_operation(input_value: TransportInput) -> Json:
    client = make_client(input_value)
    handles = Handles(
        url=client.server_url,
        slot=cast(str | None, private(client, "credential_slot")),
        resolved=private(client, "resolved_auth"),
        aws=private(client, "aws_auth"),
        explicit=object(),
    )
    record: dict[str, Json] = {
        "kind": input_value.transport_type,
        "http_client": False,
        "stdio": None,
        "sse": None,
        "http": None,
        "error": None,
    }

    def resolved_ssl(_verify: object) -> dict[str, Json]:
        return cast(dict[str, Json], input_value.tls.model_dump())

    def stdio_client(parameters: object) -> object:
        record["kind"] = "stdio"
        record["stdio"] = cast(
            Json,
            {
                "command": getattr(parameters, "command", ""),
                "args": list(cast(list[str], getattr(parameters, "args", []))),
                "env": cast(Json, getattr(parameters, "env", None)),
            },
        )
        return object()

    def sse_client(**kwargs: object) -> object:
        headers = cast(dict[str, str], kwargs.get("headers") or {})
        timeout = cast(float, kwargs.get("timeout"))
        factory = cast(Callable[..., FakeHttpClient], kwargs.get("httpx_client_factory"))
        produced = factory(headers=headers, timeout=httpx2.Timeout(timeout))
        record["kind"] = "sse"
        record["sse"] = cast(
            Json,
            {
                "url": cast(str, kwargs.get("url")),
                "timeout_ms": round(timeout * 1000),
                "headers": headers,
                "client": config_json(produced, handles),
            },
        )
        return object()

    def streamable_http_client(**kwargs: object) -> object:
        produced = cast(FakeHttpClient, kwargs.get("http_client"))
        record["kind"] = "http"
        record["http_client"] = True
        record["http"] = config_json(produced, handles)
        return object()

    with (
        patch.object(mcp_client, "stdio_client", stdio_client),
        patch.object(mcp_client, "sse_client", sse_client),
        patch.object(mcp_client, "streamable_http_client", streamable_http_client),
        patch.object(mcp_client, "_MCPHTTPClient", FakeHttpClient),
        patch.object(mcp_client, "get_ssl_configuration", resolved_ssl),
    ):
        try:
            create = cast(
                Callable[[], tuple[Transport, FakeHttpClient | None]],
                private(client, "create_transport_context"),
            )
            _context, _client = create()
        except ValueError:
            record["error"] = "missing_stdio_config"
    return record


def stdio_operation(input_value: StdioInput) -> Json:
    client = make_client(Settings(server_url=""))
    with patch.dict("os.environ", input_value.environment, clear=True), patch.object(
        mcp_client, "MCP_NPM_CACHE_DIR", input_value.npm_cache_dir
    ):
        safe_env = cast(
            Callable[[dict[str, str] | None], dict[str, str] | None],
            private(client, "get_safe_stdio_env"),
        )
        return {"env": cast(Json, safe_env(input_value.provided))}


class FakeInitResult:
    def __init__(self, version: str) -> None:
        self.protocol_version = version
        self.instructions: str | None = None


class FakeInitializeSession:
    def __init__(self, server_version: str, calls: list[str], requests: list[Json]) -> None:
        self.server_version = server_version
        self.calls = calls
        self.requests = requests

    def result(self) -> InitializeResult:
        return cast(InitializeResult, FakeInitResult(self.server_version))

    async def initialize(self) -> InitializeResult:
        self.calls.append("initialize")
        return self.result()

    async def send_request(self, request: object, _result_type: object) -> InitializeResult:
        self.calls.append("send_request")
        params = cast(InitializeParams, getattr(request, "param" + "s"))
        capabilities = params.capabilities
        self.requests.append(
            {
                "protocol_version": params.protocol_version,
                "client_info": {
                    "name": params.client_info.name,
                    "version": params.client_info.version,
                },
                "capabilities": {
                    "sampling": capabilities is not None
                    and getattr(capabilities, "sampling", None) is not None,
                    "elicitation": capabilities is not None
                    and getattr(capabilities, "elicitation", None) is not None,
                },
            }
        )
        return self.result()

    def adopt(self, _result: object) -> None:
        self.calls.append("adopt")

    async def send_notification(self, _notification: object) -> None:
        self.calls.append("send_notification")


def initialize_operation(input_value: InitializeInput) -> Json:
    callback = no_callback
    client = mcp_client.MCPClient(
        protocol_version=cast(MCPUpstreamProtocol, input_value.protocol_version),
        sampling_callback=callback if "sampling" in input_value.callbacks else None,
        elicitation_callback=callback if "elicitation" in input_value.callbacks else None,
    )
    calls: list[str] = []
    requests: list[Json] = []
    session = FakeInitializeSession(input_value.server_version, calls, requests)
    error_code: int | None = None
    version: str | None = None

    async def run_initialize() -> InitializeResult:
        initialize = cast(
            Callable[[object], Awaitable[InitializeResult]],
            private(client, "initialize_session"),
        )
        return await initialize(session)

    try:
        result = asyncio.run(run_initialize())
        version = result.protocol_version
    except MCPError as error:
        error_code = error.code
    return {
        "calls": cast(Json, calls),
        "request": requests[0] if requests else None,
        "version": version,
        "error_code": error_code,
    }


def resolved_ssl(input_value: FactoryInput) -> Callable[[object], dict[str, Json]]:
    def resolve(_verify: object) -> dict[str, Json]:
        return cast(dict[str, Json], input_value.tls.model_dump())

    return resolve


def build_factory(
    input_value: FactoryInput, *, explicit_auth: bool
) -> tuple[Json, FakeHttpClient]:
    client = make_client(input_value)
    explicit = cast(httpx2.Auth, FakeAuth(None))
    handles = Handles(
        url=client.server_url,
        slot=cast(str | None, private(client, "credential_slot")),
        resolved=private(client, "resolved_auth"),
        aws=private(client, "aws_auth"),
        explicit=explicit,
    )
    with (
        patch.object(mcp_client, "_MCPHTTPClient", FakeHttpClient),
        patch.object(mcp_client, "get_ssl_configuration", resolved_ssl(input_value)),
    ):
        creator = cast(
            Callable[[], Callable[..., FakeHttpClient]],
            private(client, "create_httpx_client_factory"),
        )
        factory = creator()
        produced = factory(
            headers={},
            timeout=httpx2.Timeout(input_value.timeout_ms / 1000),
            auth=explicit if explicit_auth else None,
        )
    return config_json(produced, handles), produced


def factory_operation(input_value: FactoryInput) -> Json:
    config, produced = build_factory(input_value, explicit_auth=input_value.explicit_auth)
    variants: list[Json] = []
    for variant in input_value.auth_variants:
        settings = input_value.model_copy(
            update={
                "auth": input_value.auth.model_copy(
                    update={
                        "resolved_auth": "resolved" in variant.sources,
                        "aws_auth": "aws" in variant.sources,
                    }
                )
            }
        )
        variant_config, _variant_http = build_factory(
            settings, explicit_auth="explicit" in variant.sources
        )
        variants.append(variant_config)
    hooks = hooks_of(produced.kwargs.get("event_hooks"))
    guards = hooks.get("request") or []

    async def apply() -> list[Json]:
        results: list[Json] = []
        for check in input_value.redirect_checks:
            request = httpx2.Request("GET", check.url, headers=check.headers)
            for guard in guards:
                await guard(request)
            results.append(cast(Json, dict(request.headers)))
        return results

    return {"config": config, "variants": variants, "redirects": asyncio.run(apply())}


class FakeOperationSession:
    message_handler: Callable[[object], Awaitable[None]] | None = None


class FakeSessionContext:
    def __init__(self, plan: Plan, calls: list[str], observed: dict[str, Json]) -> None:
        self.plan = plan
        self.calls = calls
        self.observed = observed
        self.handler: Callable[[object], Awaitable[None]] | None = None
        self.scope: RecordingScope | None = None

    async def __aenter__(self) -> FakeOperationSession:
        self.calls.append("session.enter")
        self.scope = RecordingScope.created[0] if RecordingScope.created else None
        if self.plan.session_enter == "fail":
            raise FakeError("session enter")
        session = FakeOperationSession()
        session.message_handler = self.handler
        return session

    async def __aexit__(
        self,
        _kind: type[BaseException] | None,
        _value: BaseException | None,
        _trace: object,
    ) -> bool | None:
        self.calls.append("session.exit")
        if self.scope is not None:
            self.observed["session_bound"] = round(
                (self.scope.deadline - anyio.current_time()) * 1000
            )
        if self.plan.session_exit == "fail":
            raise FakeError("session exit")
        if self.plan.session_exit == "cancel":
            raise asyncio.CancelledError
        if self.plan.session_exit == "timeout":
            await anyio.sleep(6)
        return False


class FakeTransportContext:
    def __init__(
        self,
        scenario: Scenario,
        calls: list[str],
        observed: dict[str, Json],
        http: FakeHttpClient | None,
    ) -> None:
        self.scenario = scenario
        self.calls = calls
        self.observed = observed
        self.http = http
        self.scope: RecordingScope | None = None

    async def __aenter__(self) -> Streams:
        self.calls.append("transport.enter")
        self.scope = RecordingScope.created[0] if RecordingScope.created else None
        if self.scenario.plan.transport_enter == "fail":
            raise FakeError("transport enter")
        if self.scenario.plan.transport_enter == "cancel":
            await anyio.sleep_forever()
        return (object(), object())

    async def __aexit__(
        self,
        _kind: type[BaseException] | None,
        _value: BaseException | None,
        _trace: object,
    ) -> bool | None:
        self.calls.append("transport.exit")
        if self.scope is not None:
            self.observed["transport_bound"] = round(
                (self.scope.deadline - anyio.current_time()) * 1000
            )
        if self.scenario.plan.transport_exit == "fail":
            raise FakeError("transport exit")
        if self.scenario.plan.transport_exit == "cancel":
            raise asyncio.CancelledError
        if self.scenario.plan.transport_exit == "recorded_failure":
            error = FakeError("recorded cleanup error")
            if self.http is not None:
                self.http.cleanup_errors = (error,)
            raise ExceptionGroup("cleanup", [error])
        return False


class Counter:
    def __init__(self) -> None:
        self.count = 0

    def __call__(self, *_args: object, **_kwargs: object) -> None:
        self.count += 1


def stream_exception(kind: StreamKind) -> Exception:
    if kind == "value":
        return ValueError("stream failed")
    if kind == "http":
        return httpx2.HTTPError("stream failed")
    return OSError("stream failed")


def timeout_error(code: int, *, context: bool) -> MCPError:
    if not context:
        return MCPError(code=code, message="upstream timeout")
    try:
        raise TimeoutError("elapsed")
    except TimeoutError:
        try:
            raise MCPError(code=code, message="SDK read timeout")
        except MCPError as error:
            return error


def outcome_of(error: BaseException, scopes: list["RecordingScope"]) -> str:
    if isinstance(error, StreamError):
        return f"stream:{error.kind}"
    if isinstance(error, MCPError):
        return f"protocol:{error.code}"
    if isinstance(error, TimeoutError):
        return "read_timeout"
    if isinstance(error, FakeError):
        return f"runtime:{error}"
    if isinstance(error, asyncio.CancelledError):
        timed_out = bool(scopes) and scopes[0].cancel_called
        return "timeout" if timed_out else "cancelled"
    return f"other:{type(error).__name__}"


async def run_scenario(  # noqa: C901 - the plan drives every lifecycle branch
    scenario: Scenario,
    settings: Settings,
    observed: dict[str, Json],
    calls: list[str],
) -> tuple[str | None, str | None]:
    client = make_client(settings)
    setattr(client, "_" + "last_initialize_instructions", "stale")
    fake_http = FakeHttpClient()
    transport = FakeTransportContext(
        scenario, calls, observed, fake_http if scenario.http else None
    )
    context = FakeSessionContext(scenario.plan, calls, observed)

    def build_session(*_args: object, **kwargs: object) -> FakeSessionContext:
        context.handler = cast(
            Callable[[object], Awaitable[None]] | None, kwargs.get("message_handler")
        )
        return context

    async def operation(session: object) -> str:
        calls.append("operation")
        handler = cast(
            Callable[[object], Awaitable[None]] | None,
            getattr(session, "message_handler", None),
        )
        if scenario.stream is not None and handler is not None:
            with suppress(RuntimeError):
                await handler(stream_exception(scenario.stream))
        match scenario.operation:
            case "ok":
                return "done"
            case "protocol":
                raise MCPError(code=-32000, message="upstream error")
            case "application":
                raise FakeError("application")
            case "cancel":
                raise asyncio.CancelledError
            case "read_timeout":
                raise timeout_error(-32001, context=True)
            case "relayed_timeout":
                raise timeout_error(-32001, context=False)

    async def recording_cleanup(
        pending: Callable[[], Awaitable[Result]], deadline: float
    ) -> Result:
        observed["http_bound"] = round((deadline - anyio.current_time()) * 1000)
        return await pending()

    failure: str | None = None
    try:
        with (
            patch.object(anyio, "CancelScope", RecordingScope),
            patch.object(mcp_client, "ClientSession", build_session),
            patch.object(mcp_client, "_MCPHTTPClient", FakeHttpClient),
            patch.object(mcp_client, "_run_bounded_cleanup", recording_cleanup),
        ):
            if scenario.mode == "execute":
                runner = cast(
                    Callable[..., Awaitable[str]],
                    private(client, "execute_session_operation"),
                )
                await runner(
                    transport, operation, http_client=fake_http if scenario.http else None
                )
            else:
                with patch.object(
                    mcp_client,
                    "_create_transport_context",
                    lambda: (transport, fake_http if scenario.http else None),
                ):
                    await client.run_with_session(operation, quiet_on_error=scenario.quiet)
    except BaseException as error:  # noqa: BLE001 - every helper exception maps to an outcome
        failure = outcome_of(error, RecordingScope.created)
    instructions = cast(
        str | None, private(client, "last_initialize_instructions")
    )
    return failure, instructions


def execute_operation(input_value: ExecuteInput) -> Json:
    results: list[Json] = []
    for scenario in input_value.scenarios:
        calls: list[str] = []
        observed: dict[str, Json] = {}
        RecordingScope.created = []
        warnings = Counter()
        debugs = Counter()

        with (
            patch.object(verbose_logger, "warning", warnings),
            patch.object(verbose_logger, "debug", debugs),
        ):
            failure, instructions = asyncio.run(
                run_scenario(scenario, input_value.settings, observed, calls)
            )
        results.append(
            {
                "calls": cast(Json, calls),
                "warnings": warnings.count,
                "debugs": debugs.count,
                "bounds_ms": {
                    "session": observed.get("session_bound"),
                    "transport": observed.get("transport_bound"),
                    "http": observed.get("http_bound"),
                },
                "instructions": instructions,
                "outcome": failure if failure is not None else "ok",
                "value": "done" if failure is None else None,
            }
        )
    return {"results": cast(Json, results)}


def run(input_value: dict[str, Json]) -> Json:
    _ = litellm
    operation = input_value.get("operation")
    match operation:
        case "transport":
            return transport_operation(TransportInput.model_validate(input_value))
        case "stdio_env":
            return stdio_operation(StdioInput.model_validate(input_value))
        case "initialize":
            return initialize_operation(InitializeInput.model_validate(input_value))
        case "factory":
            return factory_operation(FactoryInput.model_validate(input_value))
        case "execute":
            return execute_operation(ExecuteInput.model_validate(input_value))
        case _:
            raise ValueError(f"unknown b4744 operation: {operation}")
