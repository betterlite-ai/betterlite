import asyncio
import logging
from collections.abc import Mapping
from types import SimpleNamespace
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm._logging import verbose_proxy_logger
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy._types import ProxyErrorTypes, ProxyException
from litellm.proxy.auth import password_policy as policy
from litellm.proxy.utils import (
    hash_password,
    migrate_passwords_to_scrypt_async,  # pyright: ignore[reportUnknownVariableType]
    verify_password,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

# Pinning os.urandom gives both sides the same salt, so the scrypt strings compare byte for byte.
SALT = bytes(range(16))


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


class Users:
    def __init__(self, rows: list[tuple[str, str]]) -> None:
        self.rows = rows

    async def find_many(self, where: dict[str, Json]) -> list[SimpleNamespace]:
        del where
        return [SimpleNamespace(user_id=user_id, password=stored) for user_id, stored in self.rows]

    async def update(self, where: dict[str, Json], data: dict[str, Json]) -> None:
        user_id = where["user_id"]
        stored = cast(str, data["password"])
        self.rows = [(row_id, stored if row_id == user_id else old) for row_id, old in self.rows]


def verdict(error: ProxyException | None) -> Json:
    if error is None:
        return None
    return {
        "status": int(error.code),
        "type": ProxyErrorTypes(error.type).value,
        "param": error.param,
        "code": error.code,
        "message": error.message,
    }


def text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def general_settings(value: Json) -> Mapping[str, object]:
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise ValueError("general_settings must be an object")
    return cast(Mapping[str, object], value)


def hibp_client(spec: Json, seen: list[Json]) -> AsyncHTTPHandler:
    options = cast(dict[str, Json], spec) if isinstance(spec, dict) else {}
    replies = options.get("replies")
    by_prefix = cast(dict[str, dict[str, Json]], replies) if isinstance(replies, dict) else {}
    unreachable = options.get("unreachable") is True

    def handle(request: httpx.Request) -> httpx.Response:
        path = request.url.path
        seen.append(
            {
                "path": path,
                "add_padding": request.headers.get("Add-Padding"),
                "user_agent": request.headers.get("User-Agent"),
            }
        )
        if unreachable:
            raise httpx.ConnectError("the range API is unreachable", request=request)
        reply = by_prefix.get(path.rsplit("/", 1)[1], {})
        if reply.get("timeout") is True:
            raise httpx.ReadTimeout("the range API did not answer", request=request)
        status = reply.get("status", 200)
        body = reply.get("body", "")
        return httpx.Response(cast(int, status), content=text(body, "body").encode())

    client = AsyncHTTPHandler(timeout=policy.HIBP_TIMEOUT_SECONDS)
    client.client = httpx.AsyncClient(transport=httpx.MockTransport(handle))
    return client


def observed(seen: list[Json], recorded: Warnings, extra: dict[str, Json]) -> Json:
    split = [message.partition(": ") for message in recorded.messages]
    result: dict[str, Json] = {
        "requests": sorted(seen, key=lambda request: str(cast(dict[str, Json], request)["path"])),
        "warnings": [fixed for fixed, _, _ in split],
        "warning_details": [detail for _, _, detail in split],
    }
    result.update(extra)
    return result


async def breach(password: str, settings: Mapping[str, object], client: AsyncHTTPHandler) -> Json:
    try:
        await policy.validate_password_not_breached(password, settings, client)
    except ProxyException as error:
        return {"breached": True, "error": verdict(error)}
    return {"breached": False, "error": None}


async def bulk(
    passwords: list[str], settings: Mapping[str, object], client: AsyncHTTPHandler
) -> Json:
    verdicts = await policy.validate_passwords_bulk(passwords, settings, client)
    return {
        "verdicts": [
            {"password": password, "error": verdict(error)} for password, error in verdicts.items()
        ]
    }


async def migrate(users: list[dict[str, Json]]) -> Json:
    rows = [
        (text(user["user_id"], "user_id"), text(user["password"], "password")) for user in users
    ]
    table = Users(rows)
    client = SimpleNamespace(db=SimpleNamespace(litellm_usertable=table))
    message = await migrate_passwords_to_scrypt_async(client)
    return {
        "message": message,
        "rows": [{"user_id": user_id, "password": stored} for user_id, stored in table.rows],
    }


def with_hibp(value: dict[str, Json], work: str) -> Json:
    settings = general_settings(value.get("general_settings"))
    seen: list[Json] = []
    recorded = Warnings()
    previous = verbose_proxy_logger.level
    verbose_proxy_logger.setLevel(logging.WARNING)
    verbose_proxy_logger.addHandler(recorded)
    try:
        client = hibp_client(value.get("hibp"), seen)
        if work == "breach":
            outcome = asyncio.run(breach(text(value.get("password"), "password"), settings, client))
        else:
            passwords = value.get("passwords")
            if not isinstance(passwords, list):
                raise ValueError("passwords must be an array")
            outcome = asyncio.run(
                bulk([text(item, "password") for item in passwords], settings, client)
            )
    finally:
        verbose_proxy_logger.removeHandler(recorded)
        verbose_proxy_logger.setLevel(previous)
    return observed(seen, recorded, cast(dict[str, Json], outcome))


def run(input_value: dict[str, Json]) -> Json:
    function = text(input_value.get("function"), "function")
    if function == "hash":
        with patch("os.urandom", return_value=SALT):
            return {"hash": hash_password(text(input_value.get("password"), "password"))}
    if function == "verify":
        verified = verify_password(
            text(input_value.get("password"), "password"), text(input_value.get("stored"), "stored")
        )
        return {"verified": verified}
    if function == "strength":
        try:
            policy.validate_password_policy(
                text(input_value.get("password"), "password"),
                general_settings(input_value.get("general_settings")),
            )
        except ProxyException as error:
            return {"error": verdict(error)}
        return {"error": None}
    if function in ("breach", "bulk"):
        return with_hibp(input_value, function)
    if function == "migrate":
        users = input_value.get("users")
        if not isinstance(users, list):
            raise ValueError("users must be an array")
        with patch("os.urandom", return_value=SALT):
            return asyncio.run(migrate(cast(list[dict[str, Json]], users)))
    raise ValueError(f"unknown function {function}")
