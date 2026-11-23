import asyncio
import logging
from collections.abc import Mapping
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import httpx
from fastapi import HTTPException
from litellm.proxy._experimental.mcp_server.idp_token_exchange import (
    TokenExchangePrerequisites,
    identity_from_subject_token,
)
from litellm.proxy._types import (
    LiteLLM_EndUserTable,
    LiteLLM_OrganizationTable,
    LiteLLM_TeamMembership,
    LiteLLM_TeamTable,
    LiteLLM_UserTable,
    ProxyException,
)
from litellm.proxy.auth.handle_jwt import JWTHandler
from prisma.errors import PrismaError
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

PROXY_LOGGER: Final = "LiteLLM Proxy"
LOG_PREFIX: Final = "token exchange "


class Prerequisites(TypedDict):
    jwt_auth_enabled: bool
    has_database: bool
    maps_jwts_to_virtual_keys: bool


class ResultScript(TypedDict):
    kind: Literal["result"]
    user_id: NotRequired[str | None]
    team_id: NotRequired[str | None]
    agent_id: NotRequired[str | None]


class HttpScript(TypedDict):
    kind: Literal["http"]
    status: int
    detail: str


class ProxyScript(TypedDict):
    kind: Literal["proxy"]
    code: str
    message: str


class PlainScript(TypedDict):
    kind: Literal["plain"]
    message: str


AuthorizerScript: TypeAlias = ResultScript | HttpScript | ProxyScript | PlainScript


class Cause(TypedDict):
    kind: Literal["transient", "faulted"]
    message: str


class Input(TypedDict):
    prerequisites: Prerequisites
    subject_token: str
    authorizer: AuthorizerScript
    cause: Cause | None


class IdentityAnswer(TypedDict):
    user_id: str
    team_id: str | None


class RefusalAnswer(TypedDict):
    error: str
    description: str


AnswerBody: TypeAlias = IdentityAnswer | RefusalAnswer


class LogEntry(TypedDict):
    level: str
    message: str


class Answer(TypedDict):
    answer: AnswerBody
    logs: list[LogEntry]


# The fields of JWTAuthBuilderResult this double fills, typed so no expression of the pinned Any-carrying dict is built.
class JwtAuthResult(TypedDict):
    is_proxy_admin: bool
    team_object: LiteLLM_TeamTable | None
    user_object: LiteLLM_UserTable | None
    end_user_object: LiteLLM_EndUserTable | None
    org_object: LiteLLM_OrganizationTable | None
    token: str
    team_id: str | None
    user_id: str | None
    user_email: str | None
    end_user_id: str | None
    org_id: str | None
    team_membership: LiteLLM_TeamMembership | None
    jwt_claims: dict[str, JsonValue]
    agent_id: str | None


ANSWER_ADAPTER: Final[TypeAdapter[AnswerBody]] = TypeAdapter(AnswerBody)


def refusal_exception(script: HttpScript | ProxyScript | PlainScript) -> Exception:
    if script["kind"] == "http":
        return HTTPException(status_code=script["status"], detail=script["detail"])
    if script["kind"] == "proxy":
        return ProxyException(
            message=script["message"],
            type="invalid_request_error",
            param="None",
            code=script["code"],
        )
    return ValueError(script["message"])


def database_cause(cause: Cause) -> Exception:
    if cause["kind"] == "transient":
        return httpx.ConnectError(cause["message"])
    return PrismaError(cause["message"])


class ScriptedAuthorizer:
    def __init__(self, script: AuthorizerScript, cause: Cause | None) -> None:
        self.script = script
        self.cause = cause

    async def __call__(
        self,
        subject_token: str,
        _request_headers: Mapping[str, str],
        /,
    ) -> JwtAuthResult:
        script = self.script
        if script["kind"] == "result":
            return JwtAuthResult(
                is_proxy_admin=False,
                team_object=None,
                user_object=None,
                end_user_object=None,
                org_object=None,
                token=subject_token,
                team_id=script.get("team_id"),
                user_id=script.get("user_id"),
                user_email=None,
                end_user_id=None,
                org_id=None,
                team_membership=None,
                jwt_claims={},
                agent_id=script.get("agent_id"),
            )
        failure = refusal_exception(script)
        cause = self.cause
        if cause is None:
            raise failure
        raise failure from database_cause(cause)


class TokenExchangeLog(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.entries: list[LogEntry] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if not message.startswith(LOG_PREFIX):
            return
        self.entries.append(LogEntry(level=record.levelname, message=message))


def attach_handler(capture: TokenExchangeLog) -> logging.Logger:
    logger = logging.getLogger(PROXY_LOGGER)
    for existing in list(logger.handlers):
        logger.removeHandler(existing)
    logger.propagate = False
    logger.setLevel(logging.WARNING)
    logger.addHandler(capture)
    return logger


async def answer(input_value: Input, capture: TokenExchangeLog) -> Answer:
    prerequisites = input_value["prerequisites"]
    outcome = await identity_from_subject_token(
        input_value["subject_token"],
        request_headers={},
        prerequisites=TokenExchangePrerequisites(
            jwt_auth_enabled=prerequisites["jwt_auth_enabled"],
            has_database=prerequisites["has_database"],
            licensed=True,
            maps_jwts_to_virtual_keys=prerequisites["maps_jwts_to_virtual_keys"],
        ),
        is_jwt=JWTHandler.is_jwt,
        authorize=ScriptedAuthorizer(input_value["authorizer"], input_value["cause"]),
    )
    return Answer(
        answer=ANSWER_ADAPTER.validate_json(outcome.model_dump_json()),
        logs=capture.entries,
    )


def run(input_value: Input) -> Answer:
    capture = TokenExchangeLog()
    logger = attach_handler(capture)
    try:
        return asyncio.run(answer(input_value, capture))
    finally:
        logger.removeHandler(capture)
