import asyncio
import logging
from typing import Final, Literal, TypeAlias, TypedDict
from unittest import mock

import httpx
import litellm
from fastapi import HTTPException
from litellm.constants import INVALID_VIRTUAL_KEY_ERROR_MARKER
from litellm.proxy import proxy_server
from litellm.proxy._types import (
    ModelAccessDeniedProxyException,
    ProxyErrorTypes,
    ProxyException,
    UserAPIKeyAuth,
)
from litellm.proxy.auth import auth_exception_handler, user_api_key_auth
from litellm.proxy.auth.auth_exception_handler import UserAPIKeyAuthExceptionHandler
from litellm.proxy.auth.model_access_denied import ModelAccessDeniedHTTPException
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Scope

_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
CLIENT_PORT: Final = 1234
OUTAGE: Final = "allow_requests_on_db_unavailable"
TRACEBACKS: Final = "log_client_error_tracebacks"
KEY_IDENTITY: Final = "log_auth_failure_key_identity"
Switch: TypeAlias = Literal[
    "allow_requests_on_db_unavailable", "log_client_error_tracebacks", "log_auth_failure_key_identity"
]


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
    switches: list[Switch]
    identity: dict[str, str] | None
    request_data: dict[str, JsonValue]
    route: str
    user_agent: str | None
    client_host: str


class Input(TypedDict):
    rows: list[Row]


class CallbackFailedError(Exception):
    pass


class Recorder(logging.Handler):
    """Stands in for proxy_logging_obj and the proxy logger, recording each call in order."""

    def __init__(self, row: Row) -> None:
        super().__init__(logging.DEBUG)
        self.row: Final = row
        self.events: Final[list[JsonValue]] = []
        self.service_logging_obj: Final = self

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

    def service_failure_hook(self, call_type: str, duration: float, **_details: object) -> None:
        self.events.append({"event": "service", "call_type": call_type, "duration": duration})

    def seed(self, user_api_key_dict: UserAPIKeyAuth, model: object = None) -> None:
        self.events.append(
            {
                "event": "seed",
                "request_route": user_api_key_dict.request_route,
                "model": _JSON.validate_python(model),
            }
        )

    async def post_call_failure_hook(
        self,
        request_data: dict[str, object],
        user_api_key_dict: UserAPIKeyAuth,
        error_type: ProxyErrorTypes,
        route: str,
        **_details: object,
    ) -> Exception | None:
        self.events.append(
            {
                "event": "hook",
                "error_type": error_type.value,
                "route": route,
                "metadata": _JSON.validate_python(request_data.get("metadata")),
                "user_id": user_api_key_dict.user_id,
            }
        )
        replacement = self.row["replacement"]
        match self.row["reply"]:
            case "original":
                return None
            case "replace":
                return None if replacement is None else build(replacement)
            case "raise":
                raise CallbackFailedError("a post-call failure callback raised")


def run(input_value: Input) -> JsonValue:
    return [answer(row) for row in input_value["rows"]]


def build(failure: Failure) -> Exception:
    error: Exception
    message = failure["message"]
    status_code = failure["status_code"]
    match failure["kind"]:
        case "http":
            error = HTTPException(status_code=status_code, detail=message)
        case "proxy":
            error = ProxyException(message, failure["type"], failure["param"], status_code)
        case "model_denial":
            error = ModelAccessDeniedHTTPException(failure["internal_message"], status_code, message)
        case "connect":
            error = httpx.ConnectError(message)
    if failure["marked"]:
        setattr(error, INVALID_VIRTUAL_KEY_ERROR_MARKER, True)
    return error


def answer(row: Row) -> JsonValue:
    recorder = Recorder(row)
    proxy_logger = logging.getLogger("LiteLLM Proxy")
    # the proxy's own handler redacts secrets before writing, so the capture applies its filters too
    for installed in proxy_logger.handlers:
        for check in installed.filters:
            recorder.addFilter(check)
    switches = row["switches"]
    proxy_logger.addHandler(recorder)
    try:
        with (
            mock.patch.object(proxy_server, "general_settings", {OUTAGE: OUTAGE in switches}),
            mock.patch.object(proxy_server, "proxy_logging_obj", recorder),
            mock.patch.object(auth_exception_handler, "seed_request_identity", recorder.seed),
            mock.patch.object(litellm, TRACEBACKS, TRACEBACKS in switches),
            mock.patch.object(litellm, KEY_IDENTITY, KEY_IDENTITY in switches),
        ):
            outcome = unparsable(row) if row["step"] == "unparsable" else handled(row)
    except ProxyException as raised:
        outcome = {"raised": wire(raised)}
    except CallbackFailedError:
        outcome = {"hook_error": True}
    finally:
        proxy_logger.removeHandler(recorder)
    return {"outcome": outcome, "events": recorder.events}


def caller(row: Row) -> UserAPIKeyAuth | None:
    identity = row["identity"]
    return None if identity is None else UserAPIKeyAuth.model_validate(identity)


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
    role = granted.user_role
    return {
        "identity": {
            "user_id": granted.user_id,
            "key_name": granted.key_name,
            "user_role": None if role is None else role.value,
            "request_route": granted.request_route,
        }
    }


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


def wire(raised: ProxyException) -> JsonValue:
    error_type = raised.type
    return {
        "message": raised.message,
        "type": error_type.value if isinstance(error_type, ProxyErrorTypes) else error_type,
        "param": raised.param,
        "code": str(raised.code),
        "internal_message": raised.internal_message
        if isinstance(raised, ModelAccessDeniedProxyException)
        else None,
    }
