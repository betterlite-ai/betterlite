from dataclasses import dataclass
from typing import Protocol, cast

from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.embed.embedding import BedrockEmbedding
from litellm.types.llms.bedrock import AwsAuthParams
from pydantic import JsonValue, TypeAdapter

from .types import CredentialRequest, Keys
from .views import set_environment

RECORD: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
TAGS: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class Credentials(Protocol):
    access_key: str
    secret_key: str
    token: str | None


@dataclass(frozen=True)
class FakeCredentials:
    access_key: str
    secret_key: str
    token: str | None


def auth_record(auth: AwsAuthParams) -> dict[str, JsonValue]:
    return {
        "aws_access_key_id": auth.aws_access_key_id,
        "aws_secret_access_key": auth.aws_secret_access_key,
        "aws_session_token": auth.aws_session_token,
        "aws_session_name": auth.aws_session_name,
        "aws_profile_name": auth.aws_profile_name,
        "aws_role_name": auth.aws_role_name,
        "aws_web_identity_token": auth.aws_web_identity_token,
        "aws_sts_endpoint": auth.aws_sts_endpoint,
        "aws_external_id": auth.aws_external_id,
        "aws_session_tags": TAGS.validate_python(auth.aws_session_tags),
    }


class CredentialProvider:
    def __init__(self, keys: Keys) -> None:
        self.keys = keys
        self.calls: list[dict[str, JsonValue]] = []

    def __call__(self, auth: AwsAuthParams, region: str) -> FakeCredentials:
        self.calls.append({"auth": auth_record(auth), "region": region})
        return FakeCredentials(
            self.keys["access_key_id"],
            self.keys["secret_access_key"],
            self.keys.get("session_token"),
        )


class Loader(BedrockEmbedding):
    def load(
        self, optional_params: dict[str, JsonValue], bearer_token: str | None
    ) -> tuple[Credentials | None, str]:
        if bearer_token is None:
            return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L67-L106
                tuple[Credentials | None, str],
                self._load_credentials(optional_params, None),  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L67-L106
            )
        return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L67-L106
            tuple[Credentials | None, str],
            self._load_credentials(optional_params, bearer_token),  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L67-L106
        )


def embedding_credentials(case: CredentialRequest) -> JsonValue:
    set_environment(case["environment"])
    provider = CredentialProvider(case["credentials"])
    BaseAWSLLM.resolve_credentials = provider  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L512
    optional_params = dict(case["optional_params"])
    credentials, region = Loader().load(optional_params, case.get("bearer_token"))
    credential_fields: dict[str, JsonValue] | None = None
    if credentials is not None:
        credential_fields = {
            "access_key_id": credentials.access_key,
            "secret_access_key": credentials.secret_key,
            "session_token": credentials.token,
        }
    return {
        "region": region,
        "credentials": credential_fields,
        "resolved": len(provider.calls),
        "resolve_auth": provider.calls[0]["auth"] if provider.calls else None,
        "resolve_region": provider.calls[0]["region"] if provider.calls else None,
        "remaining": RECORD.validate_python(optional_params),
    }
