import asyncio
import json
import logging
import os
from dataclasses import dataclass
from typing import Final, Literal, NotRequired, Protocol, Required, TypedDict

import httpx
import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.proxy._types import JWTIssuerConfig, LiteLLM_JWTAuth
from litellm.proxy.auth import handle_jwt
from litellm.proxy.auth.handle_jwt import JWTHandler
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

litellm.suppress_debug_info = True

AUDIENCE_VARIABLE: Final = "JWT_AUDIENCE"
ISSUER_VARIABLE: Final = "JWT_ISSUER"
PUBLIC_KEY_URL_VARIABLE: Final = "JWT_PUBLIC_KEY_URL"
PUBLIC_KEY_URL: Final = "https://idp.test/jwks"
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
TTL: Final[TypeAdapter[float]] = TypeAdapter(float)
LogArgument = str | int | float | BaseException
PublicKey = str | dict[str, JsonValue]


class Issuer(TypedDict, total=False):
    issuer: Required[str]
    jwks_url: str | None
    audience: str | list[str] | None
    disable_audience_validation: bool
    user_id_jwt_field: str | None
    user_email_jwt_field: str | None
    team_id_jwt_field: str | None
    team_ids_jwt_field: str | None
    org_id_jwt_field: str | None
    end_user_id_jwt_field: str | None


class Reply(TypedDict):
    kind: str
    status: NotRequired[int]
    body: NotRequired[str]


class Step(TypedDict):
    op: Literal["auth_jwt", "decode", "userinfo"]
    token: NotRequired[str]
    http: NotRequired[list[Reply]]


class Scenario(TypedDict):
    now: float
    key: PublicKey
    steps: list[Step]
    audience: NotRequired[str | None]
    issuer: NotRequired[str | None]
    userinfo_endpoint: NotRequired[str | None]
    userinfo_ttl: NotRequired[float]
    issuers: NotRequired[list[Issuer]]


class Input(TypedDict):
    scenarios: list[Scenario]


class Failure(TypedDict):
    status: int | None
    type: str | None
    param: str | None
    code: str | None


class RequestView(TypedDict):
    url: str
    authorization: str
    accept: str


class WriteView(TypedDict):
    key: str
    ttl: float


class StepResult(TypedDict):
    value: JsonValue
    error: Failure | None
    warnings: list[str]
    requests: list[RequestView]
    writes: list[WriteView]


class ScenarioResult(TypedDict):
    steps: list[StepResult]


class Answer(TypedDict):
    scenarios: list[ScenarioResult]


# the two handler methods a step runs, typed so the calls carry no bare LiteLLM dict
class AuthCall(Protocol):
    async def auth_jwt(self, token: str) -> JsonValue: ...


class UserinfoCall(Protocol):
    async def get_oidc_userinfo(self, token: str) -> JsonValue: ...


# the four levels LiteLLM logs on, where only warning is compared: the rest stay silent
class Recorder:
    def __init__(self) -> None:
        self.warnings: list[str] = []

    def debug(self, message: str, *args: LogArgument) -> None: ...

    def info(self, message: str, *args: LogArgument) -> None: ...

    def warning(self, message: str, *args: LogArgument) -> None:
        del message, args
        self.warnings.append("warning")

    def error(self, message: str, *args: LogArgument) -> None: ...


# LiteLLM's in-memory tier, narrowed to the expiry rule the userinfo path relies on
class Cache(InMemoryCache):
    def __init__(self, now: float) -> None:
        super().__init__()
        self.now = now
        self.values: dict[str, JsonValue] = {}
        self.expiries: dict[str, float] = {}
        self.writes: list[WriteView] = []

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue:
        del kwargs
        expires_at = self.expiries.get(key)
        if expires_at is not None and self.now <= expires_at:
            return self.values.get(key)
        self.expiries.pop(key, None)
        self.values.pop(key, None)
        return None

    @override
    def set_cache(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None:
        seconds = TTL.validate_python(kwargs["ttl"])
        self.values[key] = value
        self.expiries[key] = self.now + seconds
        self.writes.append(WriteView(key=key, ttl=seconds))


class ProxyFailure(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    code: str
    type: str
    param: str | None


# The identity provider: the scenario's JWK as its JWKS, a discovery document on demand, and the
# userinfo endpoint answered from the step's script.
class KeyTransport:
    def __init__(self, key: PublicKey, userinfo_endpoint: str | None) -> None:
        self.key = key
        self.userinfo_endpoint = userinfo_endpoint
        self.replies: list[Reply] = []
        self.requests: list[RequestView] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        url = str(request.url)
        if url == self.userinfo_endpoint:
            self.requests.append(
                RequestView(
                    url=url,
                    authorization=request.headers.get("authorization", ""),
                    accept=request.headers.get("accept", ""),
                )
            )
            return self.userinfo_reply(request)
        if ".well-known/openid-configuration" in url:
            discovery = {"jwks_uri": f"{request.url.scheme}://{request.url.netloc}/jwks"}
            return httpx.Response(200, json=discovery)
        return httpx.Response(200, json={"keys": [self.key]})

    def userinfo_reply(self, request: httpx.Request) -> httpx.Response:
        reply = self.replies.pop(0) if self.replies else Reply(kind="transport")
        if reply["kind"] == "transport":
            raise httpx.ConnectError(
                "the scripted userinfo endpoint is unreachable", request=request
            )
        status, body = reply.get("status"), reply.get("body")
        if status is None or body is None:
            raise httpx.ConnectError("the scripted answer has no status or body", request=request)
        return httpx.Response(status, text=body)


def proxy_failure(error: BaseException) -> Failure:
    try:
        proxy = ProxyFailure.model_validate(error)
    except ValidationError:
        return Failure(status=401, type="auth_error", param="None", code="401")
    return Failure(
        status=int(proxy.code) if proxy.code.isdigit() else None,
        type=proxy.type,
        param=proxy.param,
        code=proxy.code,
    )


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
        disable_audience_validation=entry.get("disable_audience_validation", False),
        user_id_jwt_field=entry.get("user_id_jwt_field"),
        user_email_jwt_field=entry.get("user_email_jwt_field"),
        team_id_jwt_field=entry.get("team_id_jwt_field"),
        team_ids_jwt_field=entry.get("team_ids_jwt_field"),
        org_id_jwt_field=entry.get("org_id_jwt_field"),
        end_user_id_jwt_field=entry.get("end_user_id_jwt_field"),
    )


def build_handler(settings: Scenario, cache: Cache, transport: KeyTransport) -> JWTHandler:
    jwtauth = LiteLLM_JWTAuth(
        issuers=[issuer_config(entry) for entry in settings.get("issuers", [])],
        oidc_userinfo_endpoint=settings.get("userinfo_endpoint"),
        oidc_userinfo_cache_ttl=settings.get("userinfo_ttl", 300.0),
    )
    handler = JWTHandler()
    handler.update_environment(
        prisma_client=None,
        user_api_key_cache=UserApiKeyCache(in_memory_cache=cache),
        litellm_jwtauth=jwtauth,
    )
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
    transport: KeyTransport
    cache: Cache
    recorder: Recorder
    audience: str | None
    issuer: str | None

    async def step(self, step: Step) -> StepResult:
        warnings_from = len(self.recorder.warnings)
        requests_from = len(self.transport.requests)
        writes_from = len(self.cache.writes)
        token = step.get("token", "")
        operation = step["op"]
        if operation == "auth_jwt":
            value, error = await self.auth_jwt(token)
        elif operation == "decode":
            value, error = self.decode(token)
        else:
            self.transport.replies.extend(step.get("http", []))
            value, error = await self.userinfo(token)
        return StepResult(
            value=value,
            error=error,
            warnings=self.recorder.warnings[warnings_from:],
            requests=self.transport.requests[requests_from:],
            writes=self.cache.writes[writes_from:],
        )

    async def auth_jwt(self, token: str) -> tuple[JsonValue, Failure | None]:
        call: AuthCall = self.handler
        task = asyncio.ensure_future(call.auth_jwt(token))
        await asyncio.wait({task})
        return auth_outcome(task)

    def decode(self, token: str) -> tuple[JsonValue, Failure | None]:
        try:
            payload = self.handler._decode_jwt_with_public_key(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/handle_jwt.py#L1217-L1261
                token=token,
                public_key=self.key,
                audience=self.audience,
                issuer=self.issuer,
            )
        except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/handle_jwt.py#L1217-L1261
            return (None, proxy_failure(error))
        return (JSON.validate_json(json.dumps(payload)), None)

    async def userinfo(self, token: str) -> tuple[JsonValue, Failure | None]:
        call: UserinfoCall = self.handler
        task = asyncio.ensure_future(call.get_oidc_userinfo(token))
        await asyncio.wait({task})
        return userinfo_outcome(task)


async def scenario_answer(entry: Scenario, recorder: Recorder) -> ScenarioResult:
    audience = entry.get("audience")
    issuer = entry.get("issuer")
    set_variable(AUDIENCE_VARIABLE, audience)
    set_variable(ISSUER_VARIABLE, issuer)
    os.environ[PUBLIC_KEY_URL_VARIABLE] = PUBLIC_KEY_URL
    object.__setattr__(JWTHandler, "_unscoped_jwt_warning_emitted", False)
    cache = Cache(entry["now"])
    transport = KeyTransport(entry["key"], entry.get("userinfo_endpoint"))
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
        for name in (AUDIENCE_VARIABLE, ISSUER_VARIABLE, PUBLIC_KEY_URL_VARIABLE):
            os.environ.pop(name, None)
    return Answer(scenarios=scenarios)


def run(input_value: Input) -> Answer:
    with asyncio.Runner() as runner:
        return runner.run(answer(input_value))
