import os
from collections.abc import Callable, Generator, Mapping
from contextlib import contextmanager
from datetime import UTC, datetime
from importlib import import_module
from typing import Final, Literal, NotRequired, Protocol, TypedDict, cast

from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.types.llms.bedrock import AwsAuthParams
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from .views import LitellmGlobals

JSON: Final = TypeAdapter(JsonValue)
AUTH_FAILURE_MESSAGE: Final = (
    "Bedrock Mantle auth failed: no Bearer token and no usable AWS credentials. "
    "Set BEDROCK_MANTLE_API_KEY (or AWS_BEARER_TOKEN_BEDROCK) or pass api_key for Bearer auth, "
    "or provide AWS credentials (IAM role / access key / profile / web identity) for SigV4."
)
ENVIRONMENT_KEYS: Final = (
    "AWS_BEARER_TOKEN_BEDROCK",
    "AWS_REGION",
    "AWS_REGION_NAME",
    "AWS_BEDROCK_RUNTIME_ENDPOINT",
    "BEDROCK_MANTLE_API_BASE",
    "BEDROCK_MANTLE_API_KEY",
    "BEDROCK_MANTLE_REGION",
)


class Keys(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: NotRequired[str]


class BaseCase(TypedDict):
    model: NotRequired[str]
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    aws_region_name: NotRequired[str]
    inject_signer: NotRequired[bool]
    environment: NotRequired[dict[str, str]]


class BasePayload(TypedDict):
    model_cost: NotRequired[dict[str, JsonValue]]
    cases: list[BaseCase]


class SignCase(TypedDict):
    api_base: str
    headers: NotRequired[list[tuple[str, str | None]]]
    optional_params: NotRequired[dict[str, JsonValue]]
    request_data: NotRequired[dict[str, JsonValue]]
    api_key: NotRequired[str]
    model: NotRequired[str]
    stream: NotRequired[bool]
    fake_stream: NotRequired[bool]
    credentials: Keys
    environment: NotRequired[dict[str, str]]
    now: int


class SignPayload(TypedDict):
    cases: list[SignCase]


class Catalog(TypedDict):
    chat_models: NotRequired[list[str]]
    text_models: NotRequired[list[str]]


class ErrorProbe(TypedDict):
    message: str
    status_code: int
    headers: NotRequired[dict[str, str]]


class ChatCase(TypedDict):
    model: str
    project_id: NotRequired[str]
    api_key: NotRequired[str]
    headers: NotRequired[dict[str, str]]
    reasoning: NotRequired[Literal["supported", "unsupported", "panic"]]
    error: ErrorProbe
    json_mode: NotRequired[bool | None]
    sync_stream: NotRequired[bool]
    environment: NotRequired[dict[str, str]]


class ChatPayload(TypedDict):
    catalog: NotRequired[Catalog]
    cases: list[ChatCase]


class MessageCase(TypedDict):
    api_base: NotRequired[str]
    aws_region_name: NotRequired[str]
    environment: NotRequired[dict[str, str]]


class MessagePayload(TypedDict):
    cases: list[MessageCase]


class PassthroughCase(TypedDict):
    api_base: NotRequired[str]
    aws_region_name: NotRequired[str]
    aws_bedrock_runtime_endpoint: NotRequired[str]
    endpoint_type: NotRequired[Literal["runtime", "agent", "agentcore"]]
    api_key: NotRequired[JsonValue]
    environment: NotRequired[dict[str, str]]


class PassthroughPayload(TypedDict):
    cases: list[PassthroughCase]


FailureKind = Literal[
    "no_credentials",
    "partial_credentials",
    "profile_not_found",
    "credential_retrieval",
    "region",
]


class FailureCase(TypedDict):
    kind: FailureKind
    api_base: NotRequired[str]
    aws_region_name: NotRequired[str]
    environment: NotRequired[dict[str, str]]


class FailurePayload(TypedDict):
    cases: list[FailureCase]


class ReplyCase(TypedDict):
    custom_llm_provider: str
    endpoint: str


class ReplyPayload(TypedDict):
    cases: list[ReplyCase]


BASE_PAYLOAD: Final = TypeAdapter(BasePayload)
SIGN_PAYLOAD: Final = TypeAdapter(SignPayload)
CHAT_PAYLOAD: Final = TypeAdapter(ChatPayload)
MESSAGE_PAYLOAD: Final = TypeAdapter(MessagePayload)
PASSTHROUGH_PAYLOAD: Final = TypeAdapter(PassthroughPayload)
FAILURE_PAYLOAD: Final = TypeAdapter(FailurePayload)
REPLY_PAYLOAD: Final = TypeAdapter(ReplyPayload)


class BotoCoreErrorFactory(Protocol):
    def __call__(self, **kwargs: str) -> Exception: ...


class BotocoreExceptions(Protocol):
    NoCredentialsError: BotoCoreErrorFactory
    PartialCredentialsError: BotoCoreErrorFactory
    ProfileNotFound: BotoCoreErrorFactory
    CredentialRetrievalError: BotoCoreErrorFactory


class AuthClockModule(Protocol):
    get_current_datetime: Callable[[], datetime]


EXCEPTIONS = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/common_utils.py#L19-L24
    "BotocoreExceptions", import_module("botocore.exceptions")
)
AUTH_MODULE = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L1697-L1701
    "AuthClockModule", import_module("botocore.auth")
)
LITELLM = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563
    "LitellmGlobals", import_module("litellm")
)


class FixedCredentials(BaseAWSLLM):
    def __init__(self, keys: Keys) -> None:
        super().__init__()
        self.access_key = keys["access_key_id"]
        self.secret_key = keys["secret_access_key"]
        self.token: str | None = keys.get("session_token")

    @override
    def resolve_credentials(  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L512
        self, auth_params: AwsAuthParams, aws_region_name: str | None
    ) -> "FixedCredentials":
        _ = (auth_params, aws_region_name)
        return self


class FailingCredentials(BaseAWSLLM):
    def __init__(self, error: Exception) -> None:
        super().__init__()
        self.error = error

    @override
    def resolve_credentials(  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L512
        self, auth_params: AwsAuthParams, aws_region_name: str | None
    ) -> FixedCredentials:
        _ = (auth_params, aws_region_name)
        raise self.error


class FrozenClock:
    def __init__(self, now: int) -> None:
        self.frozen = datetime.fromtimestamp(now, tz=UTC).replace(tzinfo=None)

    def __call__(self) -> datetime:
        return self.frozen


@contextmanager
def environment(values: Mapping[str, str]) -> Generator[None]:
    saved: dict[str, str | None] = {
        name: os.environ[name] if name in os.environ else None for name in ENVIRONMENT_KEYS
    }
    for name in ENVIRONMENT_KEYS:
        if name in os.environ:
            del os.environ[name]
    os.environ.update(values)
    try:
        yield
    finally:
        for name in values:
            if name in os.environ:
                del os.environ[name]
        os.environ.update({name: value for name, value in saved.items() if value is not None})


def region_params(region: str | None) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {}
    if region is not None:
        params["aws_region_name"] = region
    return params
