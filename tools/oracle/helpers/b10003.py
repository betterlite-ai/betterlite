from collections.abc import Generator
from contextlib import contextmanager
from datetime import datetime
from logging import WARNING, Handler, Logger, LogRecord, getLogger
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.proxy._experimental.mcp_server.outbound_credentials.sso_assertion_store import (
    SSOIdentityAssertion,
    assertion_expired,
    assertion_from_sso_login,
)
from pydantic import JsonValue, SecretStr
from typing_extensions import override

# LiteLLM's verbose_proxy_logger writes to the logger named "LiteLLM Proxy"
PROXY_LOGGER: Final = "LiteLLM Proxy"


class LoginCall(TypedDict):
    function: Literal["login"]
    id_token: NotRequired[JsonValue]
    refresh_token: NotRequired[JsonValue]


class ExpiryCall(TypedDict):
    function: Literal["expiry"]
    expires_at: NotRequired[str | None]
    now: str


Input: TypeAlias = LoginCall | ExpiryCall


class Capture(Handler):
    def __init__(self) -> None:
        super().__init__(level=WARNING)
        self.messages: list[JsonValue] = []

    @override
    def emit(self, record: LogRecord) -> None:
        self.messages.append(record.getMessage())


@contextmanager
def capturing(logger: Logger) -> Generator[Capture, None, None]:
    capture = Capture()
    previous_level = logger.level
    logger.addHandler(capture)
    logger.setLevel(WARNING)
    try:
        yield capture
    finally:
        logger.removeHandler(capture)
        logger.setLevel(previous_level)


def assertion_value(assertion: SSOIdentityAssertion | None) -> dict[str, JsonValue]:
    if assertion is None:
        return {"id_token": None, "refresh_token": None, "issuer": None, "expires_at": None}
    refresh_token = assertion.refresh_token
    expires_at = assertion.expires_at
    return {
        "id_token": assertion.id_token.get_secret_value(),
        "refresh_token": None if refresh_token is None else refresh_token.get_secret_value(),
        "issuer": assertion.issuer,
        "expires_at": None if expires_at is None else expires_at.isoformat(),
    }


def login_value(call: LoginCall) -> dict[str, JsonValue]:
    assertion = assertion_from_sso_login(call.get("id_token"), call.get("refresh_token"))
    return assertion_value(assertion)


def expiry_value(call: ExpiryCall) -> dict[str, JsonValue]:
    expires_at = call.get("expires_at")
    assertion = SSOIdentityAssertion(
        id_token=SecretStr("id-token"),
        expires_at=None if expires_at is None else datetime.fromisoformat(expires_at),
    )
    expired = assertion_expired(assertion, datetime.fromisoformat(call["now"]))
    return {"expired": expired}


def answer(call: Input) -> dict[str, JsonValue]:
    if call["function"] == "login":
        return login_value(call)
    return expiry_value(call)


def run(input_value: Input) -> dict[str, JsonValue]:
    logger = getLogger(PROXY_LOGGER)
    with capturing(logger) as capture:
        result = answer(input_value)
    return {"result": result, "warnings": capture.messages}
