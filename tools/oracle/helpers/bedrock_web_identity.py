import os
import tempfile
from collections.abc import Callable, Mapping, Sequence
from datetime import datetime
from pathlib import Path
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.llms.bedrock import base_aws_llm
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM

if TYPE_CHECKING:
    from litellm.types.llms.bedrock import AwsSessionTag

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]


class InvalidIdentityTokenError(Exception):
    pass


class AccessDeniedError(Exception):
    pass


class FakeExceptions:
    InvalidIdentityTokenException = InvalidIdentityTokenError
    AccessDeniedException = AccessDeniedError


class FakeClient:
    exceptions = FakeExceptions
    transport: "FakeTransport"
    credentials: dict[str, Json]

    def __init__(self, transport: "FakeTransport", credentials: dict[str, Json]) -> None:
        self.transport = transport
        self.credentials = credentials

    def assume_role_with_web_identity(self, **params: object) -> dict[str, Json]:
        self.transport.record("AssumeRoleWithWebIdentity", self, params)
        return cast(dict[str, Json], self.transport.answers["AssumeRoleWithWebIdentity"])

    def assume_role(self, **params: object) -> dict[str, Json]:
        self.transport.record("AssumeRole", self, params)
        return cast(dict[str, Json], self.transport.answers["AssumeRole"])

    def get_caller_identity(self) -> dict[str, Json]:
        self.transport.record("GetCallerIdentity", self, {})
        raise RuntimeError("the fixture identity call is denied")


class FakeTransport:
    def __init__(self, answers: dict[str, Json], environment: dict[str, str]) -> None:
        self.answers = answers
        self.environment = environment
        self.calls: list[Json] = []

    def client(self, _service: str, **kwargs: Mapping[str, object]) -> FakeClient:
        value = kwargs.get("aws_access_key_id")
        token = kwargs.get("aws_session_token")
        credentials: dict[str, Json] = {
            "access_key_id": value
            if isinstance(value, str)
            else self.environment.get("AWS_ACCESS_KEY_ID"),
            "session_token": token
            if isinstance(token, str)
            else self.environment.get("AWS_SESSION_TOKEN"),
        }
        return FakeClient(self, credentials)

    def record(self, action: str, client: FakeClient, params: Mapping[str, object]) -> None:
        form: dict[str, str] = {"Action": action, "Version": "2011-06-15"}
        for name, value in params.items():
            if isinstance(value, (list, tuple)):
                for index, entry in enumerate(cast("Sequence[Mapping[str, str]]", value), start=1):
                    for field, item in entry.items():
                        form[f"{name}.member.{index}.{field}"] = item
            else:
                form[name] = str(value)
        self.calls.append(
            cast(
                Json,
                {
                    "form": form,
                    "access_key_id": client.credentials["access_key_id"],
                    "session_token": client.credentials["session_token"],
                },
            )
        )


def field(params: Mapping[str, Json], name: str) -> str:
    return cast(str, params[name])


def optional(params: Mapping[str, Json], name: str) -> str | None:
    return cast("str | None", params.get(name))


def environment(params: Mapping[str, Json]) -> dict[str, str]:
    return cast(dict[str, str], params.get("environment", {}))


def one(value: Json, name: str) -> tuple[str, Json]:
    if not isinstance(value, dict) or len(value) != 1:
        raise ValueError(f"{name} must have exactly one member")
    return next(iter(value.items()))


def identity_fields(value: Json) -> dict[str, Json]:
    fields = cast(dict[str, Json], value)
    return {
        "AccessKeyId": fields["access_key_id"],
        "SecretAccessKey": fields["secret_access_key"],
        "SessionToken": fields["session_token"],
        "Expiration": fields["expiration"],
    }


def reply_credentials(value: Json) -> dict[str, Json]:
    return {"Credentials": identity_fields(value)}


def web_identity_reply(value: Json) -> dict[str, Json]:
    kind, payload = one(value, "reply")
    match kind:
        case "credentials":
            reply = reply_credentials(payload)
            packed = cast(dict[str, Json], payload).get("packed_policy_size")
            if packed is not None:
                reply["PackedPolicySize"] = cast(Json, packed)
            return reply
        case "missing_packed_policy_size":
            return reply_credentials(payload)
        case "invalid_identity_token":
            fields = cast(dict[str, Json], payload)
            raise InvalidIdentityTokenError(cast(str, fields.get("message", "")))
        case _:
            raise ValueError(f"unknown web identity reply: {kind}")


def irsa_result(calls: list[Json], value: Json) -> Json:
    credentials = cast(dict[str, Json], cast(dict[str, Json], value)["Credentials"])
    return cast(
        Json,
        {
            "calls": calls,
            "credentials": [
                credentials["AccessKeyId"],
                credentials["SecretAccessKey"],
                credentials["SessionToken"],
            ],
            "expiration": int(
                datetime.fromisoformat(cast(str, credentials["Expiration"])).timestamp()
            ),
        },
    )


def tags(params: Mapping[str, Json]) -> list[Json]:
    entries = cast(list[dict[str, str]], params.get("tags", []))
    return [{"Key": entry["key"], "Value": entry["value"]} for entry in entries]


def web_identity(params: Params) -> Json:
    env = environment(params)
    transport = FakeTransport(
        {"AssumeRoleWithWebIdentity": web_identity_reply(params["reply"])}, env
    )
    warnings: list[str] = []

    def record(message: str, *args: object) -> None:
        warnings.append(message % args if args else message)

    max_policy_size = params.get("max_policy_size", 75)
    with (
        patch.dict(os.environ, env, clear=True),
        patch.object(base_aws_llm, "get_secret", return_value=optional(params, "resolved_token")),
        patch.object(base_aws_llm, "BEDROCK_MAX_POLICY_SIZE", cast(int, max_policy_size)),
        patch("boto3.client", side_effect=transport.client),
        patch.object(base_aws_llm.verbose_logger, "warning", new=record),
    ):
        creds, ttl = BaseAWSLLM()._auth_with_web_identity_token(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
            aws_web_identity_token=field(params, "token_reference"),
            aws_role_name=field(params, "role_arn"),
            aws_session_name=field(params, "session_name"),
            aws_external_id=optional(params, "external_id"),
            aws_sts_endpoint=optional(params, "sts_endpoint"),
            aws_region_name=optional(params, "region_name"),
            ssl_verify=True,
        )
    token = cast(str | None, creds.token)  # pyright: ignore[reportUnknownMemberType]
    return cast(
        Json,
        {
            "calls": transport.calls,
            "credentials": [creds.access_key, creds.secret_key, token],  # pyright: ignore[reportUnknownMemberType]
            "ttl": ttl,
            "warnings": warnings,
        },
    )


def same_account_irsa(params: Params) -> Json:
    env = environment(params)
    transport = FakeTransport({"AssumeRole": reply_credentials(params["reply"])}, env)
    tags_value = tags(params)
    with (
        patch.dict(os.environ, env, clear=True),
        patch("boto3.client", side_effect=transport.client),
    ):
        response = BaseAWSLLM()._handle_irsa_same_account(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
            aws_role_name=field(params, "role_arn"),
            aws_session_name=field(params, "session_name"),
            aws_session_tags=cast("Sequence[AwsSessionTag] | None", tags_value),
            aws_external_id=optional(params, "external_id"),
            aws_sts_endpoint=optional(params, "sts_endpoint"),
            aws_region_name=optional(params, "region_name"),
            ssl_verify=True,
        )
    return irsa_result(transport.calls, cast(Json, response))


def cross_account_irsa(params: Params) -> Json:
    env = environment(params)
    token_file = cast(dict[str, str], params["token_file"])
    transport = FakeTransport(
        {
            "AssumeRoleWithWebIdentity": reply_credentials(params["irsa_reply"]),
            "AssumeRole": reply_credentials(params["reply"]),
        },
        env,
    )
    tags_value = tags(params)
    with tempfile.TemporaryDirectory() as folder:
        path = Path(folder) / "token"
        path.write_text(token_file["content"], encoding="utf-8")
        with (
            patch.dict(os.environ, env, clear=True),
            patch("boto3.client", side_effect=transport.client),
        ):
            response = BaseAWSLLM()._handle_irsa_cross_account(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
                irsa_role_arn=field(params, "irsa_role_arn"),
                aws_role_name=field(params, "role_arn"),
                aws_session_name=field(params, "session_name"),
                web_identity_token_file=str(path),
                aws_session_tags=cast("Sequence[AwsSessionTag] | None", tags_value),
                aws_external_id=optional(params, "external_id"),
                aws_sts_endpoint=optional(params, "sts_endpoint"),
                aws_region_name=optional(params, "region_name"),
                ssl_verify=True,
            )
    return irsa_result(transport.calls, cast(Json, response))


def audiences(params: Params) -> Json:
    return cast(
        Json,
        {
            "audiences": [
                BaseAWSLLM._unverified_web_identity_audience(token)  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
                for token in cast(list[str], params.get("tokens", []))
            ]
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


def run(input_value: Params) -> Json:
    kind, payload = one(cast(Json, input_value), "input")
    handler = HANDLERS.get(kind)
    if handler is None:
        raise ValueError(f"unknown operation: {kind}")
    _ = litellm
    return handler(cast(Params, payload))
