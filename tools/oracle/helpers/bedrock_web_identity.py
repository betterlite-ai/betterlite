import os
import tempfile
from collections.abc import Callable, Mapping, Sequence
from datetime import datetime
from pathlib import Path
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch
from urllib.parse import urlparse

import litellm
from litellm.llms.bedrock import base_aws_llm
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM

if TYPE_CHECKING:
    from litellm.types.llms.bedrock import AwsSessionTag

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]
Reply: TypeAlias = dict[str, Json]


class InvalidIdentityTokenError(Exception):
    pass


class AccessDeniedError(Exception):
    pass


class FakeExceptions:
    InvalidIdentityTokenException = InvalidIdentityTokenError
    AccessDeniedException = AccessDeniedError


def object_of(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_of(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_of(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_string(params: Mapping[str, object], name: str) -> str | None:
    value = params.get(name)
    return value if isinstance(value, str) else None


def optional_bool(params: Mapping[str, object], name: str) -> bool | None:
    value = params.get(name)
    return value if isinstance(value, bool) else None


def environment_of(params: Params) -> dict[str, str]:
    fields = object_of(params.get("environment", {}), "environment")
    return {name: string_of(value, name) for name, value in fields.items()}


def reply_credentials(value: Json) -> dict[str, Json]:
    fields = object_of(value, "credentials")
    credentials: dict[str, Json] = {
        "AccessKeyId": string_of(fields.get("access_key_id"), "access_key_id"),
        "SecretAccessKey": string_of(fields.get("secret_access_key"), "secret_access_key"),
        "SessionToken": string_of(fields.get("session_token"), "session_token"),
        "Expiration": string_of(fields.get("expiration"), "expiration"),
    }
    return {"Credentials": credentials}


def with_packed(value: Json, reply: dict[str, Json]) -> dict[str, Json]:
    packed = object_of(value, "credentials").get("packed_policy_size")
    result = dict(reply)
    if packed is not None:
        result["PackedPolicySize"] = cast(Json, packed)
    return result


def web_identity_reply(value: Json) -> dict[str, Json]:
    kind, fields = one_entry(value, "reply")
    payload = object_of(fields, "reply")
    match kind:
        case "credentials":
            return with_packed(payload, reply_credentials(payload))
        case "missing_packed_policy_size":
            return reply_credentials(payload)
        case "missing_credentials":
            return {"PackedPolicySize": 1}
        case "invalid_identity_token":
            raise InvalidIdentityTokenError(string_of(payload.get("message"), "message"))
        case "access_denied":
            raise AccessDeniedError(string_of(payload.get("message"), "message"))
        case _:
            raise ValueError(f"unknown web identity reply: {kind}")


def one_entry(value: Json, name: str) -> tuple[str, Json]:
    fields = object_of(value, name)
    if len(fields) != 1:
        raise ValueError(f"{name} must have exactly one member")
    return next(iter(fields.items()))


class FakeClient:
    exceptions = FakeExceptions
    transport: "FakeTransport"
    credentials: dict[str, Json]
    region: str
    endpoint: str

    def __init__(
        self,
        transport: "FakeTransport",
        credentials: dict[str, Json],
        region: str,
        endpoint: str,
    ) -> None:
        self.transport = transport
        self.credentials = credentials
        self.region = region
        self.endpoint = endpoint

    def assume_role_with_web_identity(self, **params: object) -> dict[str, Json]:
        self.transport.record("AssumeRoleWithWebIdentity", self, params)
        return self.transport.responses["AssumeRoleWithWebIdentity"]

    def assume_role(self, **params: object) -> dict[str, Json]:
        self.transport.record("AssumeRole", self, params)
        return self.transport.responses["AssumeRole"]

    def get_caller_identity(self) -> dict[str, Json]:
        self.transport.record("GetCallerIdentity", self, {})
        if self.transport.identity is None:
            raise RuntimeError("the fixture identity call is denied")
        return {"Arn": self.transport.identity}


class FakeTransport:
    def __init__(self, responses: dict[str, Reply], identity: str | None) -> None:
        self.responses = responses
        self.identity = identity
        self.calls: list[Json] = []
        self.environment: dict[str, str] = {}

    def client(self, _service: str, **kwargs: object) -> FakeClient:
        region = (
            optional_string(kwargs, "region_name")
            or self.environment.get("AWS_REGION")
            or self.environment.get("AWS_DEFAULT_REGION")
            or "aws-global"
        )
        endpoint = optional_string(kwargs, "endpoint_url")
        host = urlparse(endpoint).hostname if endpoint is not None else None
        if host is None:
            host = "sts.amazonaws.com" if region == "aws-global" else f"sts.{region}.amazonaws.com"
        credentials: dict[str, Json] = {
            "access_key_id": optional_string(kwargs, "aws_access_key_id")
            or self.environment.get("AWS_ACCESS_KEY_ID"),
            "session_token": optional_string(kwargs, "aws_session_token")
            or self.environment.get("AWS_SESSION_TOKEN"),
        }
        return FakeClient(self, credentials, region, host)

    def record(self, action: str, client: FakeClient, params: dict[str, object]) -> None:
        tags: Json = None
        raw_tags = params.get("Tags")
        if isinstance(raw_tags, list):
            entries: list[Json] = []
            for tag in cast("list[object]", raw_tags):
                fields = object_of(cast(Json, tag), "tag")
                entries.append(
                    [
                        string_of(fields.get("Key"), "Key"),
                        string_of(fields.get("Value"), "Value"),
                    ]
                )
            tags = entries
        duration = params.get("DurationSeconds")
        self.calls.append(
            {
                "action": action,
                "access_key_id": client.credentials["access_key_id"],
                "session_token": client.credentials["session_token"],
                "region": client.region,
                "endpoint": client.endpoint,
                "role_arn": optional_string(params, "RoleArn"),
                "session_name": optional_string(params, "RoleSessionName"),
                "web_identity_token": optional_string(params, "WebIdentityToken"),
                "duration_seconds": cast(Json, duration) if isinstance(duration, int) else None,
                "policy": optional_string(params, "Policy"),
                "external_id": optional_string(params, "ExternalId"),
                "tags": tags,
            }
        )


def credentials_list(credentials: dict[str, Json]) -> list[Json]:
    return [
        credentials["AccessKeyId"],
        credentials["SecretAccessKey"],
        credentials["SessionToken"],
    ]


def epoch(value: Json) -> int:
    return int(datetime.fromisoformat(string_of(value, "Expiration")).timestamp())


def audiences(params: Params) -> Json:
    return {
        "audiences": [
            BaseAWSLLM._unverified_web_identity_audience(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
                string_of(token, "token")
            )
            for token in list_of(params.get("tokens"), "tokens")
        ]
    }


def web_identity(params: Params) -> Json:
    environment = environment_of(params)
    reply = object_of(params.get("reply"), "reply")
    transport = FakeTransport({"AssumeRoleWithWebIdentity": web_identity_reply(reply)}, None)
    transport.environment = environment
    warnings: list[str] = []

    def record(message: str, *args: object) -> None:
        warnings.append(message % args if args else message)

    max_policy_size = params.get("max_policy_size")
    threshold = cast(int, max_policy_size) if isinstance(max_policy_size, int) else 75
    instance = BaseAWSLLM()
    with (
        patch.dict(os.environ, environment, clear=True),
        patch.object(
            base_aws_llm, "get_secret", return_value=optional_string(params, "resolved_token")
        ),
        patch.object(base_aws_llm, "BEDROCK_MAX_POLICY_SIZE", threshold),
        patch("boto3.client", side_effect=transport.client),
        patch.object(base_aws_llm.verbose_logger, "warning", new=record),
    ):
        credentials, ttl = instance._auth_with_web_identity_token(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            aws_web_identity_token=string_of(params.get("token_reference"), "token_reference"),
            aws_role_name=string_of(params.get("role_arn"), "role_arn"),
            aws_session_name=string_of(params.get("session_name"), "session_name"),
            aws_region_name=optional_string(params, "region_name"),
            aws_sts_endpoint=optional_string(params, "sts_endpoint"),
            aws_external_id=optional_string(params, "external_id"),
            ssl_verify=optional_bool(params, "verify"),
        )
    token = cast(str | None, credentials.token)  # pyright: ignore[reportUnknownMemberType]
    return cast(
        Json,
        {
            "calls": transport.calls,
            "credentials": [credentials.access_key, credentials.secret_key, token],  # pyright: ignore[reportUnknownMemberType]
            "ttl": ttl,
            "warnings": warnings,
        },
    )


def tags_input(params: Params) -> list[Json]:
    tags: list[Json] = []
    for tag in list_of(params.get("tags", []), "tags"):
        fields = object_of(tag, "tag")
        tags.append(
            {
                "Key": string_of(fields.get("key"), "key"),
                "Value": string_of(fields.get("value"), "value"),
            }
        )
    return tags


def irsa_environment(params: Params) -> dict[str, str]:
    return environment_of(params)


def same_account_irsa(params: Params) -> Json:
    environment = irsa_environment(params)
    transport = FakeTransport(
        {
            "GetCallerIdentity": {},
            "AssumeRole": reply_credentials(params.get("reply")),
        },
        identity_string(params.get("identity")),
    )
    transport.environment = environment
    instance = BaseAWSLLM()
    with (
        patch.dict(os.environ, environment, clear=True),
        patch("boto3.client", side_effect=transport.client),
    ):
        response: Json = cast(Json, instance._handle_irsa_same_account(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            aws_role_name=string_of(params.get("role_arn"), "role_arn"),
            aws_session_name=string_of(params.get("session_name"), "session_name"),
            aws_external_id=optional_string(params, "external_id"),
            aws_sts_endpoint=optional_string(params, "sts_endpoint"),
            ssl_verify=optional_bool(params, "verify"),
            aws_region_name=optional_string(params, "region_name"),
            aws_session_tags=cast("Sequence[AwsSessionTag] | None", tags_input(params)),
        ))
    credentials = object_of(response, "response")["Credentials"]
    return cast(
        Json,
        {
            "calls": transport.calls,
            "credentials": credentials_list(object_of(credentials, "Credentials")),
            "expiration": epoch(object_of(credentials, "Credentials")["Expiration"]),
        },
    )


def identity_string(value: Json) -> str | None:
    if value is None:
        return None
    kind, fields = one_entry(value, "identity")
    if kind != "arn":
        return None
    return string_of(object_of(fields, "identity").get("arn"), "arn")


def cross_account_irsa(params: Params) -> Json:
    environment = irsa_environment(params)
    token_file = object_of(params.get("token_file"), "token_file")
    content = string_of(token_file.get("content"), "content")
    fake_irsa = reply_credentials(params.get("irsa_reply"))
    transport = FakeTransport(
        {
            "AssumeRoleWithWebIdentity": fake_irsa,
            "GetCallerIdentity": {},
            "AssumeRole": reply_credentials(params.get("reply")),
        },
        identity_string(params.get("identity")),
    )
    transport.environment = environment
    instance = BaseAWSLLM()
    with tempfile.TemporaryDirectory() as folder:
        path = Path(folder) / "token"
        path.write_text(content, encoding="utf-8")
        with (
            patch.dict(os.environ, environment, clear=True),
            patch("boto3.client", side_effect=transport.client),
        ):
            response: Json = cast(Json, instance._handle_irsa_cross_account(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
                irsa_role_arn=string_of(params.get("irsa_role_arn"), "irsa_role_arn"),
                aws_role_name=string_of(params.get("role_arn"), "role_arn"),
                aws_session_name=string_of(params.get("session_name"), "session_name"),
                web_identity_token_file=str(path),
                aws_external_id=optional_string(params, "external_id"),
                aws_sts_endpoint=optional_string(params, "sts_endpoint"),
                ssl_verify=optional_bool(params, "verify"),
                aws_region_name=optional_string(params, "region_name"),
                aws_session_tags=cast("Sequence[AwsSessionTag] | None", tags_input(params)),
            ))
    credentials = object_of(response, "response")["Credentials"]
    return cast(
        Json,
        {
            "calls": transport.calls,
            "credentials": credentials_list(object_of(credentials, "Credentials")),
            "expiration": epoch(object_of(credentials, "Credentials")["Expiration"]),
        },
    )


HANDLERS: dict[str, Callable[[Params], Json]] = {
    "jwt_audience_hint": audiences,
    "malformed_token": audiences,
    "web_identity_assume_role": web_identity,
    "secure_transport_session_policy": web_identity,
    "invalid_identity_token_error": web_identity,
    "sts_credentials_missing": web_identity,
    "packed_policy_warning_and_ttl": web_identity,
    "same_account_irsa": same_account_irsa,
    "cross_account_irsa": cross_account_irsa,
}


def run(input_value: dict[str, Json]) -> Json:
    kind, payload = one_entry(cast(Json, input_value), "input")
    handler = HANDLERS.get(kind)
    if handler is None:
        raise ValueError(f"unknown operation: {kind}")
    _ = litellm
    return handler(object_of(payload, "payload"))
