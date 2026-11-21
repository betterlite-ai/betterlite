import asyncio
import logging
import sys
from types import ModuleType
from typing import Final, Literal, TypedDict

import httpx
import litellm
from fastapi import HTTPException
from litellm.proxy._types import ModelAccessDeniedProxyException as DeniedError
from litellm.proxy._types import ProxyErrorTypes, ProxyException, UserAPIKeyAuth
from litellm.proxy.auth import user_api_key_auth
from litellm.proxy.auth.auth_exception_handler import UserAPIKeyAuthExceptionHandler
from litellm.proxy.auth.auth_utils import mark_invalid_virtual_key_error
from litellm.proxy.auth.model_access_denied import ModelAccessDeniedHTTPException
from litellm.types.services import ServiceTypes
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Scope
from typing_extensions import override

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
CLIENT_PORT: Final = 1234
IDENTITY_FIELDS: Final = {"user_id", "key_name", "user_role", "request_route"}
_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class Failure(TypedDict):
    kind: Literal["http", "proxy", "model_denial", "connect"]
    message: str
    type: str
    param: str | None
    status_code: int
    marked: bool
    internal_message: str


class Row(TypedDict):
    step: Literal["handle", "unparsable"]
    failure: Failure
    reply: Literal["original", "replace", "raise"]
    replacement: Failure | None
    switches: list[
        Literal[
            "allow_requests_on_db_unavailable",
            "log_client_error_tracebacks",
            "log_auth_failure_key_identity",
        ]
    ]
    identity: dict[str, str] | None
    request_data: dict[str, JsonValue]
    route: str
    user_agent: str | None
    client_host: str


class Input(TypedDict):
    rows: list[Row]


class CallbackFailedError(Exception):
    pass


# stands in for proxy_logging_obj, its service logger and the proxy's log handler
class Recorder(logging.Handler):
    def __init__(self, row: Row) -> None:
        super().__init__(logging.DEBUG)
        self.row: Final = row
        self.events: Final[list[JsonValue]] = []
        self.service_logging_obj: Final = self

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.events.append(
            {
                "event": "log",
                "logger": record.name,
                "level": record.levelname,
                "traceback": record.exc_info is not None,
                "message": record.getMessage(),
            }
        )

    def service_failure_hook(
        self, service: ServiceTypes, call_type: str, error: Exception, duration: float
    ) -> None:
        del service, error
        self.events.append({"event": "service", "call_type": call_type, "duration": duration})

    async def post_call_failure_hook(
        self,
        request_data: dict[str, JsonValue],
        user_api_key_dict: UserAPIKeyAuth,
        error_type: ProxyErrorTypes,
        route: str,
        **_original_exception: Exception,
    ) -> Exception | None:
        self.events.append(
            {
                "event": "hook",
                "error_type": error_type.value,
                "route": route,
                "metadata": request_data.get("metadata"),
                "user_id": user_api_key_dict.user_id,
                "request_route": user_api_key_dict.request_route,
            }
        )
        match self.row["reply"]:
            case "original":
                return None
            case "replace":
                replacement = self.row["replacement"]
                return None if replacement is None else build(replacement)
            case "raise":
                raise CallbackFailedError("a post-call failure callback raised")


class ProxyServer(ModuleType):
    general_settings: dict[str, JsonValue]
    proxy_logging_obj: Recorder


def run(input_value: Input) -> JsonValue:
    proxy_logger = logging.getLogger("LiteLLM Proxy")
    answers: list[JsonValue] = []
    for row in input_value["rows"]:
        recorder = Recorder(row)
        # the proxy's handler redacts secrets, so the capture applies its filters too
        for installed in proxy_logger.handlers:
            for check in installed.filters:
                recorder.addFilter(check)
        proxy_logger.addHandler(recorder)
        try:
            outcome = answer(row, recorder)
        finally:
            proxy_logger.removeHandler(recorder)
        answers.append({"outcome": outcome, "events": recorder.events})
    return answers


def build(failure: Failure) -> Exception:
    message = failure["message"]
    status_code = failure["status_code"]
    match failure["kind"]:
        case "http":
            return HTTPException(status_code=status_code, detail=message)
        case "proxy":
            raised = ProxyException(message, failure["type"], failure["param"], status_code)
            return mark_invalid_virtual_key_error(raised, is_invalid_virtual_key=failure["marked"])
        case "model_denial":
            internal = failure["internal_message"]
            return ModelAccessDeniedHTTPException(internal, status_code, message)
        case "connect":
            return httpx.ConnectError(message)


def answer(row: Row, recorder: Recorder) -> JsonValue:
    # dispatch runs one call per process, so the fake module and the switches need no restore
    switches = row["switches"]
    proxy_server = ProxyServer(PROXY_MODULE)
    outage = "allow_requests_on_db_unavailable" in switches
    proxy_server.general_settings = {"allow_requests_on_db_unavailable": outage}
    proxy_server.proxy_logging_obj = recorder
    sys.modules[PROXY_MODULE] = proxy_server
    litellm.log_client_error_tracebacks = "log_client_error_tracebacks" in switches
    litellm.log_auth_failure_key_identity = "log_auth_failure_key_identity" in switches
    try:
        return unparsable(row) if row["step"] == "unparsable" else handled(row)
    except DeniedError as denied:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4374
        return {"raised": wire(denied, denied.sanitized_internal_message())}
    except ProxyException as raised:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        return {"raised": wire(raised, None)}
    except CallbackFailedError:
        return {"hook_error": True}


def caller(row: Row) -> UserAPIKeyAuth | None:
    identity = row["identity"]
    return None if identity is None else UserAPIKeyAuth.model_validate(identity)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3356


def handled(row: Row) -> JsonValue:
    agent = row["user_agent"]
    headers = [] if agent is None else [(b"user-agent", agent.encode("latin-1"))]
    client = (row["client_host"], CLIENT_PORT)
    scope: Scope = {"type": "http", "path": row["route"], "headers": headers, "client": client}
    granted = asyncio.run(
        UserAPIKeyAuthExceptionHandler._handle_authentication_error(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_exception_handler.py#L128-L260
            e=build(row["failure"]),
            request=Request(scope=scope),
            request_data=dict(row["request_data"]),
            route=row["route"],
            parent_otel_span=None,
            api_key="sk-presented-key",
            resolved_identity=caller(row),
        )
    )
    return {"identity": _JSON.validate_json(granted.model_dump_json(include=IDENTITY_FIELDS))}


def unparsable(row: Row) -> JsonValue:
    failure = row["failure"]
    exception = ProxyException(
        failure["message"], failure["type"], failure["param"], failure["status_code"]
    )
    asyncio.run(
        user_api_key_auth._record_unparsable_body_failure(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L1384-L1406
            user_api_key_dict=caller(row) or UserAPIKeyAuth(),
            body_parse_exception=exception,
            route=row["route"],
        )
    )
    return {"recorded": True}


def wire(raised: ProxyException, internal_message: str | None) -> JsonValue:
    return {
        "message": raised.message,
        "type": raised.type,
        "param": raised.param,
        "code": raised.code,
        "internal_message": internal_message,
    }
