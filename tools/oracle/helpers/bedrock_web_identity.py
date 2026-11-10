import os
import tempfile
from collections.abc import Callable, Generator, Mapping, Sequence
from contextlib import contextmanager
from datetime import datetime
from pathlib import Path
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.llms.bedrock import base_aws_llm
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.types.llms.bedrock import AwsSessionTag
from pydantic import JsonValue

Json: TypeAlias = JsonValue
Params: TypeAlias = dict[str, Json]


class InvalidIdentityTokenError(Exception):
    pass


class FakeExceptions:
    InvalidIdentityTokenException = InvalidIdentityTokenError


class FakeSts:
    exceptions = FakeExceptions

    def __init__(self, answers: dict[str, Json], environment: dict[str, str]) -> None:
        self.answers = answers
        self.environment = environment
        self.calls: list[Json] = []
        self.access_key_id: str | None = None
        self.session_token: str | None = None

    def client(self, _service: str, **kwargs: object) -> "FakeSts":
        access = kwargs.get("aws_access_key_id")
        token = kwargs.get("aws_session_token")
        if not isinstance(access, str):
            access = self.environment.get("AWS_ACCESS_KEY_ID")
        if not isinstance(token, str):
            token = self.environment.get("AWS_SESSION_TOKEN")
        self.access_key_id = access
        self.session_token = token
        return self

    def assume_role_with_web_identity(self, **params: object) -> Json:
        return self.record("AssumeRoleWithWebIdentity", params)

    def assume_role(self, **params: object) -> Json:
        return self.record("AssumeRole", params)

    def get_caller_identity(self) -> Json:
        self.record("GetCallerIdentity", {})
        raise RuntimeError("the fixture identity call is denied")

    def record(self, action: str, params: Mapping[str, object]) -> Json:
        form: dict[str, str] = {"Action": action, "Version": "2011-06-15"}
        for name, value in params.items():
            if isinstance(value, (list, tuple)):
                entries = cast("Sequence[Mapping[str, str]]", value)
                for index, entry in enumerate(entries, start=1):
                    for field, item in entry.items():
                        form[f"{name}.member.{index}.{field}"] = item
            else:
                form[name] = str(value)
        unsigned = action == "AssumeRoleWithWebIdentity"
        call = cast(
            Json,
            {
                "form": form,
                "access_key_id": None if unsigned else self.access_key_id,
                "session_token": None if unsigned else self.session_token,
            },
        )
        self.calls.append(call)
        answer = self.answers[action]
        if isinstance(answer, dict) and "error" in answer:
            raise InvalidIdentityTokenError(cast(str, answer["error"]))
        return answer


def one(value: Json, name: str) -> tuple[str, Json]:
    if not isinstance(value, dict) or len(value) != 1:
        raise ValueError(f"{name} must have exactly one member")
    return next(iter(value.items()))


def environment(params: Mapping[str, Json]) -> dict[str, str]:
    return cast(dict[str, str], params.get("environment", {}))


def credentials(params: Mapping[str, Json]) -> dict[str, Json]:
    return {
        "Credentials": {
            "AccessKeyId": params["access_key_id"],
            "SecretAccessKey": params["secret_access_key"],
            "SessionToken": params["session_token"],
            "Expiration": params["expiration"],
        }
    }


def web_identity_answer(value: Json) -> dict[str, Json]:
    kind, reply = one(value, "reply")
    match kind:
        case "credentials":
            answer = credentials(cast(dict[str, Json], reply))
            packed = cast(dict[str, Json], reply).get("packed_policy_size")
            if packed is not None:
                answer["PackedPolicySize"] = packed
            return answer
        case "missing_packed_policy_size":
            return credentials(cast(dict[str, Json], reply))
        case "invalid_identity_token":
            return {"error": cast(str, cast(dict[str, Json], reply).get("message", ""))}
        case _:
            raise ValueError(f"unknown web identity reply: {kind}")


def irsa_result(calls: list[Json], value: Json) -> Json:
    fields = cast(dict[str, Json], cast(dict[str, Json], value)["Credentials"])
    expiration = datetime.fromisoformat(cast(str, fields["Expiration"]))
    return {
        "calls": calls,
        "credentials": [fields["AccessKeyId"], fields["SecretAccessKey"], fields["SessionToken"]],
        "expiration": int(expiration.timestamp()),
    }


def session_tags(params: Mapping[str, Json]) -> list[AwsSessionTag]:
    entries = cast(list[dict[str, str]], params.get("tags", []))
    return [AwsSessionTag(Key=entry["key"], Value=entry["value"]) for entry in entries]


@contextmanager
def harness(sts: FakeSts, params: Params) -> Generator[list[str], None, None]:
    warnings: list[str] = []

    def record(message: str, *args: object) -> None:
        warnings.append(message % args if args else message)

    max_policy_size = cast(int, params.get("max_policy_size", 75))
    resolved = cast("str | None", params.get("resolved_token"))
    with (
        patch.dict(os.environ, sts.environment, clear=True),
        patch("boto3.client", side_effect=sts.client),
        patch.object(base_aws_llm, "get_secret", return_value=resolved),
        patch.object(base_aws_llm, "BEDROCK_MAX_POLICY_SIZE", max_policy_size),
        patch.object(base_aws_llm.verbose_logger, "warning", new=record),
    ):
        yield warnings


def web_identity(params: Params) -> Json:
    sts = FakeSts(
        {"AssumeRoleWithWebIdentity": web_identity_answer(params["reply"])}, environment(params)
    )
    with harness(sts, params) as warnings:
        creds, ttl = BaseAWSLLM()._auth_with_web_identity_token(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
            aws_web_identity_token=cast(str, params["token_reference"]),
            aws_role_name=cast(str, params["role_arn"]),
            aws_session_name=cast(str, params["session_name"]),
            aws_external_id=cast("str | None", params.get("external_id")),
            aws_sts_endpoint=None,
            aws_region_name=None,
            ssl_verify=True,
        )
    token = cast(str | None, creds.token)  # pyright: ignore[reportUnknownMemberType]
    return cast(
        Json,
        {
            "calls": sts.calls,
            "credentials": [creds.access_key, creds.secret_key, token],  # pyright: ignore[reportUnknownMemberType]
            "ttl": ttl,
            "warnings": warnings,
        },
    )


def same_account(params: Params) -> Json:
    sts = FakeSts({"AssumeRole": web_identity_answer(params["reply"])}, environment(params))
    with harness(sts, params):
        response = BaseAWSLLM()._handle_irsa_same_account(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
            aws_role_name=cast(str, params["role_arn"]),
            aws_session_name=cast(str, params["session_name"]),
            aws_session_tags=session_tags(params),
            aws_external_id=cast("str | None", params.get("external_id")),
            aws_sts_endpoint=None,
            aws_region_name=None,
            ssl_verify=True,
        )
    return irsa_result(sts.calls, cast(Json, response))


def cross_account(params: Params) -> Json:
    token_file = cast(dict[str, str], params["token_file"])
    sts = FakeSts(
        {
            "AssumeRoleWithWebIdentity": web_identity_answer(params["irsa_reply"]),
            "AssumeRole": web_identity_answer(params["reply"]),
        },
        environment(params),
    )
    with tempfile.TemporaryDirectory() as folder:
        path = Path(folder) / "token"
        path.write_text(token_file["content"], encoding="utf-8")
        with harness(sts, params):
            response = BaseAWSLLM()._handle_irsa_cross_account(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
                irsa_role_arn=cast(str, params["irsa_role_arn"]),
                aws_role_name=cast(str, params["role_arn"]),
                aws_session_name=cast(str, params["session_name"]),
                web_identity_token_file=str(path),
                aws_session_tags=session_tags(params),
                aws_external_id=cast("str | None", params.get("external_id")),
                aws_sts_endpoint=None,
                aws_region_name=None,
                ssl_verify=True,
            )
    return irsa_result(sts.calls, cast(Json, response))


def audiences(params: Params) -> Json:
    tokens = cast(list[str], params.get("tokens", []))
    return {
        "audiences": [
            BaseAWSLLM._unverified_web_identity_audience(token)  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
            for token in tokens
        ]
    }


HANDLERS: dict[str, Callable[[Params], Json]] = {
    "jwt_audience_hint": audiences,
    "web_identity": web_identity,
    "same_account_irsa": same_account,
    "cross_account_irsa": cross_account,
}


def run(input_value: Params) -> Json:
    kind, payload = one(cast(Json, input_value), "input")
    handler = HANDLERS.get(kind)
    if handler is None:
        raise ValueError(f"unknown operation: {kind}")
    _ = litellm
    return handler(cast(Params, payload))
