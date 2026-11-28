import asyncio
import json
import logging
import os
from dataclasses import dataclass
from typing import Final, Protocol, cast

import httpx
import litellm
from litellm.proxy._types import JWTIssuerConfig, LiteLLM_JWTAuth
from litellm.proxy.auth import handle_jwt
from litellm.proxy.auth.handle_jwt import JWTHandler
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from pydantic import JsonValue

from .fakes import (
    JSON,
    Answer,
    Cache,
    Failure,
    Input,
    Issuer,
    PublicKey,
    Recorder,
    Scenario,
    ScenarioResult,
    ScriptedKeys,
    Step,
    StepResult,
    UserinfoTransport,
    proxy_failure,
)

litellm.suppress_debug_info = True

AUDIENCE_VARIABLE: Final = "JWT_AUDIENCE"
ISSUER_VARIABLE: Final = "JWT_ISSUER"


# the three handler methods a step runs, typed so the calls carry no bare LiteLLM dict
class AuthCall(Protocol):
    async def auth_jwt(self, token: str) -> JsonValue: ...


class UserinfoCall(Protocol):
    async def get_oidc_userinfo(self, token: str) -> JsonValue: ...


class DecodeCall(Protocol):
    def _decode_jwt_with_public_key(
        self,
        token: str,
        public_key: PublicKey,
        audience: str | None,
        issuer: str | None,
    ) -> JsonValue: ...


def set_variable(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value


def issuer_config(entry: Issuer) -> JWTIssuerConfig:
    return JWTIssuerConfig(
        issuer=entry["issuer"],
        jwks_url=entry.get("jwks_url"),
        audience=entry.get("audience"),
        disable_audience_validation=entry["disable_audience_validation"],
        user_id_jwt_field=entry.get("user_id_jwt_field"),
        user_email_jwt_field=entry.get("user_email_jwt_field"),
        team_id_jwt_field=entry.get("team_id_jwt_field"),
        team_ids_jwt_field=entry.get("team_ids_jwt_field"),
        org_id_jwt_field=entry.get("org_id_jwt_field"),
        end_user_id_jwt_field=entry.get("end_user_id_jwt_field"),
    )


def build_handler(settings: Scenario, cache: Cache, transport: UserinfoTransport) -> JWTHandler:
    issuers: list[JWTIssuerConfig] = [issuer_config(entry) for entry in settings["issuers"]]
    jwtauth = LiteLLM_JWTAuth(
        issuers=issuers,
        oidc_userinfo_endpoint=settings.get("userinfo_endpoint"),
        oidc_userinfo_cache_ttl=settings["userinfo_ttl"],
    )
    handler = JWTHandler()
    handler.update_environment(
        prisma_client=None,
        user_api_key_cache=UserApiKeyCache(in_memory_cache=cache),
        litellm_jwtauth=jwtauth,
    )
    keys = ScriptedKeys(settings["key"])
    object.__setattr__(handler, "get_public_key", keys.get_public_key)
    object.__setattr__(handler, "_get_public_key_from_jwks_url", keys.get_public_key_from_jwks_url)
    handler.http_handler.client = httpx.AsyncClient(
        transport=httpx.MockTransport(transport.handle)
    )
    return handler


def auth_outcome(task: asyncio.Future[JsonValue]) -> tuple[JsonValue, Failure | None]:
    failure = task.exception()
    if failure is not None:
        return (None, proxy_failure(failure))
    return (task.result(), None)


def userinfo_outcome(task: asyncio.Future[JsonValue]) -> tuple[JsonValue, Failure | None]:
    failure = task.exception()
    if failure is not None:
        return (None, Failure(status=None, type=None, param=None, code=None))
    return (task.result(), None)


@dataclass
class Session:
    handler: JWTHandler
    key: PublicKey
    transport: UserinfoTransport
    cache: Cache
    recorder: Recorder
    audience: str | None
    issuer: str | None

    async def step(self, step: Step) -> StepResult:
        warnings_from = len(self.recorder.warnings)
        requests_from = len(self.transport.requests)
        writes_from = len(self.cache.writes)
        token = step["token"]
        operation = step["op"]
        if operation == "auth_jwt":
            value, error = await self.auth_jwt(token)
        elif operation == "decode":
            value, error = self.decode(token)
        else:
            if "http" in step:
                self.transport.replies.extend(step["http"])
            value, error = await self.userinfo(token)
        return StepResult(
            value=value,
            error=error,
            warnings=self.recorder.warnings[warnings_from:],
            requests=self.transport.requests[requests_from:],
            writes=self.cache.writes[writes_from:],
        )

    async def auth_jwt(self, token: str) -> tuple[JsonValue, Failure | None]:
        call = cast(AuthCall, self.handler)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/handle_jwt.py#L1295-L1335
        task = asyncio.ensure_future(call.auth_jwt(token))
        await asyncio.wait({task})
        return auth_outcome(task)

    def decode(self, token: str) -> tuple[JsonValue, Failure | None]:
        call = cast(DecodeCall, self.handler)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/handle_jwt.py#L1217-L1261
        try:
            payload = call._decode_jwt_with_public_key(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/handle_jwt.py#L1217-L1261
                token=token,
                public_key=self.key,
                audience=self.audience,
                issuer=self.issuer,
            )
        except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/handle_jwt.py#L1217-L1261
            return (None, proxy_failure(error))
        return (JSON.validate_json(json.dumps(payload)), None)

    async def userinfo(self, token: str) -> tuple[JsonValue, Failure | None]:
        call = cast(UserinfoCall, self.handler)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/handle_jwt.py#L1009-L1066
        task = asyncio.ensure_future(call.get_oidc_userinfo(token))
        await asyncio.wait({task})
        return userinfo_outcome(task)


async def scenario_answer(entry: Scenario, recorder: Recorder) -> ScenarioResult:
    audience = entry.get("audience")
    issuer = entry.get("issuer")
    set_variable(AUDIENCE_VARIABLE, audience)
    set_variable(ISSUER_VARIABLE, issuer)
    JWTHandler._unscoped_jwt_warning_emitted = False  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/handle_jwt.py#L1068
    cache = Cache(entry["now"])
    transport = UserinfoTransport(entry.get("userinfo_endpoint"))
    handler = build_handler(entry, cache, transport)
    session = Session(
        handler=handler,
        key=entry["key"],
        transport=transport,
        cache=cache,
        recorder=recorder,
        audience=audience,
        issuer=issuer,
    )
    steps = [await session.step(step) for step in entry["steps"]]
    return ScenarioResult(steps=steps)


async def answer(input_value: Input) -> Answer:
    recorder = Recorder()
    object.__setattr__(handle_jwt, "verbose_proxy_logger", recorder)
    try:
        scenarios = [
            await scenario_answer(entry, recorder) for entry in input_value["scenarios"]
        ]
    finally:
        object.__setattr__(
            handle_jwt, "verbose_proxy_logger", logging.getLogger("LiteLLM Proxy")
        )
        for name in (AUDIENCE_VARIABLE, ISSUER_VARIABLE):
            os.environ.pop(name, None)
    return Answer(scenarios=scenarios)


def run(input_value: Input) -> Answer:
    with asyncio.Runner() as runner:
        return runner.run(answer(input_value))
