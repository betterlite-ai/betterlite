import os
from collections.abc import Callable, Iterator, Mapping
from contextlib import contextmanager
from datetime import UTC, datetime
from importlib import import_module
from typing import Final, Protocol, cast
from unittest.mock import patch

from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.types.llms.bedrock import AwsAuthParams
from pydantic import JsonValue, TypeAdapter

from .models import Keys

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


class FixedCredentials:
    def __init__(self, keys: Keys) -> None:
        self.access_key = keys["access_key_id"]
        self.secret_key = keys["secret_access_key"]
        self.token: str | None = keys.get("session_token")

    def __call__(self, _auth_params: AwsAuthParams, _region: str | None) -> "FixedCredentials":
        return self


class FailingCredentials:
    def __init__(self, error: Exception) -> None:
        self.error = error

    def __call__(self, _auth_params: AwsAuthParams, _region: str | None) -> FixedCredentials:
        raise self.error


class FrozenClock:
    def __init__(self, now: int) -> None:
        self.frozen = datetime.fromtimestamp(now, tz=UTC).replace(tzinfo=None)

    def __call__(self) -> datetime:
        return self.frozen


@contextmanager
def environment(values: Mapping[str, str]) -> Iterator[None]:
    saved = {name: os.environ.pop(name, None) for name in ENVIRONMENT_KEYS}
    os.environ.update(values)
    try:
        yield
    finally:
        for name in values:
            _ = os.environ.pop(name, None)
        os.environ.update({name: value for name, value in saved.items() if value is not None})


@contextmanager
def signing_harness(credentials: FixedCredentials | FailingCredentials, now: int) -> Iterator[None]:
    with (
        patch.object(BaseAWSLLM, "resolve_credentials", credentials),
        patch.object(AUTH_MODULE, "get_current_datetime", FrozenClock(now)),
    ):
        yield


def region_params(region: str | None) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {}
    if region is not None:
        params["aws_region_name"] = region
    return params
