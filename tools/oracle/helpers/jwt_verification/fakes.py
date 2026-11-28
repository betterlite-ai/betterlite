from typing import Final, Literal, NotRequired, Required, TypedDict

import httpx
from litellm.caching.in_memory_cache import InMemoryCache
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
KEY_OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
TTL: Final[TypeAdapter[float]] = TypeAdapter(float)
LogArgument = str | int | float | BaseException
PublicKey = str | dict[str, JsonValue]


class Issuer(TypedDict, total=False):
    issuer: Required[str]
    jwks_url: str | None
    audience: str | list[str] | None
    disable_audience_validation: Required[bool]
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
    token: str
    http: NotRequired[list[Reply]]


class Scenario(TypedDict):
    now: float
    key: PublicKey
    steps: list[Step]
    userinfo_ttl: float
    issuers: list[Issuer]
    audience: NotRequired[str | None]
    issuer: NotRequired[str | None]
    userinfo_endpoint: NotRequired[str | None]


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


# the key B761's helper fetches and compares, served at the same lookup
class ScriptedKeys:
    def __init__(self, key: PublicKey) -> None:
        self.key = key

    async def get_public_key(self, kid: str | None) -> dict[str, JsonValue]:
        del kid
        return KEY_OBJECT.validate_python(self.key)

    async def get_public_key_from_jwks_url(
        self, jwks_url: str, kid: str | None
    ) -> dict[str, JsonValue]:
        del jwks_url, kid
        return KEY_OBJECT.validate_python(self.key)


# the identity provider's userinfo endpoint and the replies it serves
class UserinfoTransport:
    def __init__(self, userinfo_endpoint: str | None) -> None:
        self.userinfo_endpoint = userinfo_endpoint
        self.replies: list[Reply] = []
        self.requests: list[RequestView] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        url = str(request.url)
        if url != self.userinfo_endpoint:
            raise httpx.ConnectError(f"unscripted request to {url}", request=request)
        self.requests.append(
            RequestView(
                url=url,
                authorization=request.headers["authorization"],
                accept=request.headers["accept"],
            )
        )
        return self.userinfo_reply(request)

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
