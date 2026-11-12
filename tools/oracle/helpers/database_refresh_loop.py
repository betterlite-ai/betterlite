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
from typing_extensions import override

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
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


class FrozenClock:
    def __init__(self, now: str) -> None:
        self.now = now
        self.previous: type[datetime] = datetime

    def __enter__(self) -> Self:
        FrozenDateTime.set_current(self.now)
        self.previous = prisma_client.datetime  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/prisma_client.py#L9
        prisma_client.datetime = FrozenDateTime  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/prisma_client.py#L9
        return self

    def __exit__(
        self,
        kind: type[BaseException] | None,
        error: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        prisma_client.datetime = self.previous  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/prisma_client.py#L9


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


def wrapper(auth: AuthKind) -> PrismaWrapper:
    created = object.__new__(PrismaWrapper)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/prisma_client.py#L142
    created._token_auth = authentication(auth)  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/prisma_client.py#L182
    created._db_url_env_var = ENV_VAR  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/prisma_client.py#L190
    return created


def set_environment(url: str | None) -> None:
    if url is None:
        os.environ.pop(ENV_VAR, None)
    else:
        os.environ[ENV_VAR] = url


def extract_password(call: ExtractPasswordCall) -> Json:
    with captured() as capture:
        password = wrapper("disabled")._extract_token_from_db_url(call["url"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/prisma_client.py#L349
    return {"password": password, "log_levels": capture.levels}


def expiration(call: ParseExpirationCall) -> Json:
    with captured() as capture:
        parsed = wrapper(call["auth"])._parse_token_expiration(call["token"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/prisma_client.py#L370
    value = None if parsed is None else parsed.isoformat()
    return {"expiration": value, "log_levels": capture.levels}


def seconds_until_refresh(call: SecondsUntilRefreshCall) -> Json:
    set_environment(call["url"])
    created = wrapper(call["auth"])
    with FrozenClock(call["now"]), captured() as capture:
        seconds = created._calculate_seconds_until_refresh()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/prisma_client.py#L380
    return {"seconds": float(seconds), "log_levels": capture.levels}


def is_token_expired(call: IsTokenExpiredCall) -> Json:
    created = wrapper(call["auth"])
    with FrozenClock(call["now"]), captured() as capture:
        expired = created.is_token_expired(call["url"])
    return {"expired": expired, "log_levels": capture.levels}


def token_refresh_not_needed(call: TokenRefreshNotNeededCall) -> Json:
    created = wrapper(call["auth"])
    with FrozenClock(call["now"]), captured() as capture:
        not_needed = created._token_refresh_not_needed(call["url"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/prisma_client.py#L791
    return {"not_needed": not_needed, "log_levels": capture.levels}


def evaluate(call: Call) -> Json:
    if call["function"] == "extract_password":
        return extract_password(call)
    if call["function"] == "parse_expiration":
        return expiration(call)
    if call["function"] == "seconds_until_refresh":
        return seconds_until_refresh(call)
    if call["function"] == "is_token_expired":
        return is_token_expired(call)
    return token_refresh_not_needed(call)


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
