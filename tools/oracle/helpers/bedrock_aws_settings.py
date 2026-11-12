import sys
from collections.abc import Callable, MutableMapping
from datetime import UTC, datetime
from types import ModuleType
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast

from botocore.credentials import Credentials  # type: ignore[import-untyped]  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/credentials.py#L340
from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.llms.bedrock.base_aws_llm import (
    AwsAuthError,
    BaseAWSLLM,
    bedrock_bearer_token,
    build_web_identity_session_policy,
    pop_aws_auth_params,
)
from litellm.llms.bedrock.common_utils import s3_static_key_pair
from litellm.llms.custom_httpx.http_handler import get_ssl_verify
from litellm.types.llms.bedrock import AwsSessionTag
from pydantic import JsonValue, TypeAdapter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class AwsConfig(TypedDict, total=False):
    aws_access_key_id: str | None
    aws_secret_access_key: str | None
    aws_session_token: str | None
    aws_region_name: str | None
    aws_session_name: str | None
    aws_profile_name: str | None
    aws_role_name: str | None
    aws_web_identity_token: str | None
    aws_sts_endpoint: str | None
    aws_external_id: str | None
    aws_session_tags: list[AwsSessionTag] | None
    ssl_verify: bool | str | None


class RoleParams(TypedDict):
    role_arn: str
    session_name: str
    external_id: NotRequired[str | None]
    tags: NotRequired[list[AwsSessionTag] | None]


class AuthErrorParams(TypedDict):
    status: int
    message: str
    ssl_verify: NotRequired[bool | str | None]


class CacheStep(TypedDict):
    now: int
    access: NotRequired[str | None]
    fail: NotRequired[bool]


class CacheParams(TypedDict):
    parameters: AwsConfig
    steps: list[CacheStep]


class LocalParams(TypedDict):
    settings: AwsConfig
    fixture: list[str | None]


class Input(TypedDict):
    operation: Literal["static_key_pair", "canonical_tags", "assume_role", "web_policy", "bearer",
                       "pop_auth", "cache_key", "auth_selector", "local", "cache", "auth_error"]
    params: Json


StsValue: TypeAlias = None | str | int | datetime | dict[str, "StsValue"]


class FakeBoto3(ModuleType):
    Session: "FakeBoto3"
    client: "FakeBoto3"

    def __init__(self, access: str | None, secret: str | None, token: str | None) -> None:
        super().__init__("boto3")
        self.access = access
        self.secret = secret
        self.token = token
        self.now = 0.0
        self.fetches = 0
        self.fail = False
        self.session_calls: list[dict[str, Json]] = []
        self.sts_calls: list[tuple[str, dict[str, Json]]] = []
        self.Session = self
        self.client = self

    def clock(self) -> float:
        return self.now

    def __call__(self, *args: Json, **kwargs: Json) -> "FakeBoto3":
        if not args:
            self.session_calls.append(dict(kwargs))
        return self

    def get_credentials(self) -> Credentials:  # type: ignore[no-any-unimported]  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/credentials.py#L340
        self.fetches += 1
        if self.fail:
            raise ValueError("fixture credential fetch failed")
        return Credentials(self.access, self.secret, self.token)  # type: ignore[misc]  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/credentials.py#L340

    def get_caller_identity(self) -> dict[str, Json]:
        raise RuntimeError("fixture sts has no caller identity")

    def assume_role(self, **kwargs: Json) -> dict[str, StsValue]:
        self.sts_calls.append(("assume_role", dict(kwargs)))
        return self.reply()

    def assume_role_with_web_identity(self, **kwargs: Json) -> dict[str, StsValue]:
        self.sts_calls.append(("assume_role_with_web_identity", dict(kwargs)))
        return self.reply()

    def reply(self) -> dict[str, StsValue]:
        credentials: dict[str, StsValue] = {
            "AccessKeyId": self.access, "SecretAccessKey": self.secret, "SessionToken": self.token,
            "Expiration": datetime(2030, 1, 1, tzinfo=UTC),
        }
        return {"Credentials": credentials, "PackedPolicySize": 0}

    def source(self) -> str:
        names = [name for name, _arguments in self.sts_calls]
        if "assume_role_with_web_identity" in names:
            return "web_identity"
        if "assume_role" in names:
            return "role"
        if not self.session_calls:
            return "session_token"
        last_call = self.session_calls[-1]
        if "profile_name" in last_call:
            return "profile"
        if "aws_access_key_id" in last_call:
            return "static"
        return "env"

    def assume_role_kwargs(self) -> dict[str, Json]:
        for name, arguments in reversed(self.sts_calls):
            if name == "assume_role":
                return arguments
        raise ValueError("get_credentials never called sts assume_role")


class AuthRequest(Protocol):
    method: str
    url: str


class AuthReply(Protocol):
    status_code: int


class AuthFailure(Protocol):
    status_code: int
    message: str
    request: AuthRequest
    response: AuthReply


def prepare(fixture: FakeBoto3, clock: Callable[[], float] | None = None) -> BaseAWSLLM:
    # LiteLLM imports boto3 inside each source method, so these imports find the fake module
    sys.modules["boto3"] = fixture
    instance = BaseAWSLLM()
    memory = InMemoryCache(max_size_in_memory=1000, clock=clock)
    instance.iam_cache = DualCache(in_memory_cache=memory)
    return instance


def credentials_call(instance: BaseAWSLLM, config: AwsConfig) -> Credentials:  # type: ignore[no-any-unimported]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L346
    return cast(Credentials, instance.get_credentials(**config))  # type: ignore[no-any-unimported, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L346


def cached_ttl(instance: BaseAWSLLM, now: float) -> int | None:
    entries = cast(dict[str, float], instance.iam_cache.in_memory_cache.ttl_dict)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/in_memory_cache.py#L51
    expiry = next(iter(entries.values()), None)
    return None if expiry is None else int(expiry - now)


def role_config(params: RoleParams) -> AwsConfig:
    return AwsConfig(
        aws_role_name=params["role_arn"], aws_session_name=params["session_name"],
        aws_external_id=params.get("external_id"), aws_session_tags=params.get("tags"))


def static_pair(raw: Json) -> Json:
    pair = s3_static_key_pair(TypeAdapter(dict[str, str | None]).validate_python(raw))
    return {"value": None if pair is None else [pair[0], pair[1]]}


def canonical_tags(raw: Json) -> Json:
    fixture = FakeBoto3(None, None, None)
    credentials_call(prepare(fixture), role_config(TypeAdapter(RoleParams).validate_python(raw)))
    return {"value": fixture.assume_role_kwargs().get("Tags")}


def assume_role(raw: Json) -> Json:
    fixture = FakeBoto3(None, None, None)
    credentials_call(prepare(fixture), role_config(TypeAdapter(RoleParams).validate_python(raw)))
    return {"value": fixture.assume_role_kwargs()}


def web_policy(_raw: Json) -> Json:
    return cast(Json, {"value": build_web_identity_session_policy()})  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L155


def bearer(raw: Json) -> Json:
    params = TypeAdapter(dict[str, str | None]).validate_python(raw)
    return {"value": bedrock_bearer_token(params.get("api_key"))}


def pop_auth(raw: Json) -> Json:
    remaining = TypeAdapter(dict[str, JsonValue]).validate_python(raw)
    try:
        pop_aws_auth_params(cast(MutableMapping[str, object], remaining))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L171
    except ValueError:
        return {"remaining": remaining, "valid": False}
    return {"remaining": remaining, "valid": True}


def cache_key(raw: Json) -> Json:
    params = TypeAdapter(dict[str, dict[str, str | bool | None]]).validate_python(raw)
    instance = BaseAWSLLM()
    left = instance.get_cache_key(params["left"])
    right = instance.get_cache_key(params["right"])
    return {"length": len(left), "equal": left == right}


def auth_selector(raw: Json) -> Json:
    fixture = FakeBoto3(None, None, None)
    credentials_call(prepare(fixture), TypeAdapter(AwsConfig).validate_python(raw))
    return {"source": fixture.source()}


def local(raw: Json) -> Json:
    params = TypeAdapter(LocalParams).validate_python(raw)
    fixture = FakeBoto3(*params["fixture"])
    instance = prepare(fixture, fixture.clock)
    credentials = credentials_call(instance, params["settings"])  # type: ignore[misc]  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/credentials.py#L340
    values: list[Json] = [credentials.access_key, credentials.secret_key, credentials.token]  # type: ignore[misc]  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/credentials.py#L340
    return {"credentials": values, "ttl": cached_ttl(instance, fixture.now),
            "source": fixture.source()}


def cache_step(
    instance: BaseAWSLLM, fixture: FakeBoto3, parameters: AwsConfig, step: CacheStep
) -> Json:
    fixture.now = step["now"]
    fixture.access = step.get("access")
    fixture.fail = step.get("fail") is True or step.get("access") is None
    try:
        credentials = credentials_call(instance, parameters)  # type: ignore[misc]  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/credentials.py#L340
    except ValueError:
        return {"error": True, "ttl": cached_ttl(instance, fixture.now)}
    access: str | None = credentials.access_key  # type: ignore[misc]  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/credentials.py#L340
    return {"access": access, "ttl": cached_ttl(instance, fixture.now)}


def cache(raw: Json) -> Json:
    params = TypeAdapter(CacheParams).validate_python(raw)
    fixture = FakeBoto3(None, "fixture", None)
    instance = prepare(fixture, fixture.clock)
    steps = params["steps"]
    results = [cache_step(instance, fixture, params["parameters"], step) for step in steps]
    return {"fetches": fixture.fetches, "results": results}


def failure_body(failure: AuthFailure) -> Json:
    request: Json = {"method": failure.request.method, "url": str(failure.request.url)}
    return {
        "status": failure.status_code,
        "message": failure.message,
        "request": request,
        "response": {"status_code": failure.response.status_code, "request": request},
    }


def auth_error(raw: Json) -> Json:
    params = TypeAdapter(AuthErrorParams).validate_python(raw)
    failure = cast(AuthFailure, AwsAuthError(params["status"], params["message"]))  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L203
    verification = get_ssl_verify(ssl_verify=params.get("ssl_verify"))
    return {"error": failure_body(failure), "ssl_verify": verification}


HANDLERS: dict[str, Callable[[Json], Json]] = {
    "static_key_pair": static_pair, "canonical_tags": canonical_tags, "assume_role": assume_role,
    "web_policy": web_policy, "bearer": bearer, "pop_auth": pop_auth, "cache_key": cache_key,
    "auth_selector": auth_selector, "local": local, "cache": cache, "auth_error": auth_error,
}


def run(input_value: Input) -> Json:
    handler = HANDLERS.get(input_value["operation"])
    if handler is None:
        raise ValueError(f"unknown operation: {input_value['operation']}")
    return handler(input_value["params"])
