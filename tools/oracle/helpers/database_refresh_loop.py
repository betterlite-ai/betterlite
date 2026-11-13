import logging
import os
from collections.abc import Generator
from contextlib import contextmanager
from datetime import datetime
from types import TracebackType
from typing import ClassVar, Literal, Self, TypedDict

from litellm.proxy.db import prisma_client
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.db.token_auth import AzureEntraTokenAuth, DatabaseTokenAuth, RdsIamTokenAuth
from pydantic import JsonValue as Json
from typing_extensions import override

AuthKind = Literal["disabled", "rds", "azure"]
ENV_VAR = "DATABASE_URL"
LOGGER_NAME = "LiteLLM Proxy"


class FrozenDateTime(datetime):
    current: ClassVar["FrozenDateTime"]

    @classmethod
    def set_current(cls, text: str) -> None:
        cls.current = cls.fromisoformat(text)

    @classmethod
    @override
    def utcnow(cls) -> "FrozenDateTime":
        return cls.current


# LiteLLM does not export prisma_client.datetime, so object.__setattr__ writes the module attribute
class FrozenClock:
    def __init__(self, now: str) -> None:
        self.now = now

    def __enter__(self) -> Self:
        FrozenDateTime.set_current(self.now)
        object.__setattr__(prisma_client, "datetime", FrozenDateTime)
        return self

    def __exit__(
        self,
        kind: type[BaseException] | None,
        error: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        object.__setattr__(prisma_client, "datetime", datetime)


class LevelCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.levels: list[Json] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.levels.append(record.levelname.lower())


@contextmanager
def captured() -> Generator[LevelCapture, None, None]:
    capture = LevelCapture()
    logger = logging.getLogger(LOGGER_NAME)
    previous_handlers = list(logger.handlers)
    previous_level = logger.level
    for handler in previous_handlers:
        logger.removeHandler(handler)
    logger.addHandler(capture)
    logger.setLevel(logging.DEBUG)
    try:
        yield capture
    finally:
        logger.removeHandler(capture)
        for handler in previous_handlers:
            logger.addHandler(handler)
        logger.setLevel(previous_level)


class ExtractPasswordCall(TypedDict):
    function: Literal["extract_password"]
    url: str | None


class ParseExpirationCall(TypedDict):
    function: Literal["parse_expiration"]
    auth: AuthKind
    token: str | None


class ClockCall(TypedDict):
    auth: AuthKind
    url: str | None
    now: str


class SecondsUntilRefreshCall(ClockCall):
    function: Literal["seconds_until_refresh"]


class IsTokenExpiredCall(ClockCall):
    function: Literal["is_token_expired"]


class TokenRefreshNotNeededCall(ClockCall):
    function: Literal["token_refresh_not_needed"]


Call = (
    ExtractPasswordCall
    | ParseExpirationCall
    | SecondsUntilRefreshCall
    | IsTokenExpiredCall
    | TokenRefreshNotNeededCall
)


class BatchInput(TypedDict):
    function: Literal["batch"]
    calls: list[Call]


Input = Call | BatchInput


class UnusedTokenProvider:
    def __call__(self) -> str:
        raise RuntimeError("the expiry parser never mints a token")


def authentication(kind: AuthKind) -> DatabaseTokenAuth | None:
    if kind == "rds":
        return RdsIamTokenAuth()
    if kind == "azure":
        return AzureEntraTokenAuth(token_provider=UnusedTokenProvider())
    return None


# Each helper's private call sits in a method body with real work, not a one-line forwarder
class Wrapper(PrismaWrapper):
    def __init__(self, kind: AuthKind) -> None:
        self._token_auth = authentication(kind)
        self._db_url_env_var = ENV_VAR

    def extract_password(self, db_url: str | None) -> Json:
        with captured() as capture:
            password = self._extract_token_from_db_url(db_url)
        return {"password": password, "log_levels": capture.levels}

    def expiration(self, token: str | None) -> Json:
        with captured() as capture:
            parsed = self._parse_token_expiration(token)
        value = None if parsed is None else parsed.isoformat()
        return {"expiration": value, "log_levels": capture.levels}

    def scheduled(self, now: str) -> Json:
        with FrozenClock(now), captured() as capture:
            seconds = self._calculate_seconds_until_refresh()
        return {"seconds": float(seconds), "log_levels": capture.levels}

    def expired(self, token_url: str | None, now: str) -> Json:
        with FrozenClock(now), captured() as capture:
            expired = self.is_token_expired(token_url)
        return {"expired": expired, "log_levels": capture.levels}

    def refresh_not_needed(self, token_url: str | None, now: str) -> Json:
        with FrozenClock(now), captured() as capture:
            not_needed = self._token_refresh_not_needed(token_url)
        return {"not_needed": not_needed, "log_levels": capture.levels}


def set_environment(url: str | None) -> None:
    if url is None:
        os.environ.pop(ENV_VAR, None)
    else:
        os.environ[ENV_VAR] = url


def evaluate(call: Call) -> Json:
    if call["function"] == "extract_password":
        return Wrapper("disabled").extract_password(call["url"])
    if call["function"] == "parse_expiration":
        return Wrapper(call["auth"]).expiration(call["token"])
    if call["function"] == "seconds_until_refresh":
        set_environment(call["url"])
        return Wrapper(call["auth"]).scheduled(call["now"])
    if call["function"] == "is_token_expired":
        return Wrapper(call["auth"]).expired(call["url"], call["now"])
    return Wrapper(call["auth"]).refresh_not_needed(call["url"], call["now"])


def scenario(call: Call) -> Json:
    try:
        value = evaluate(call)
    except (ValueError, RuntimeError, OverflowError, OSError):
        return {"ok": False}
    return {"ok": True, "value": value}


def run(input_value: Input) -> Json:
    if input_value["function"] == "batch":
        return [scenario(call) for call in input_value["calls"]]
    return evaluate(input_value)
