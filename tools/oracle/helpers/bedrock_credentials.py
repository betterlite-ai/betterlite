import os
from collections.abc import Callable, Generator, Mapping, Sequence
from contextlib import contextmanager
from datetime import UTC, datetime, tzinfo
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm._logging import verbose_logger
from litellm.llms.bedrock import base_aws_llm
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.types.llms.bedrock import AwsAuthParams, AwsSessionTag
from pydantic import JsonValue

Json: TypeAlias = JsonValue
Params: TypeAlias = dict[str, Json]

DEFAULT_NOW = 1767225600
CASE_KEYS: dict[str, str] = {
    "AssumeRole": "assume_role",
    "AssumeRoleWithWebIdentity": "assume_role_with_web_identity",
    "GetCallerIdentity": "get_caller_identity",
}
CODES: dict[str, str] = {
    (
        "AssumeRole failed for %s (%s). Caller is already running as this role; "
        "falling back to ambient credentials."
    ): "assume_role_failed_same_identity",
    (
        "AssumeRole AccessDenied for %s and caller is NOT the same role. Re-raising. Error: %s"
    ): "assume_role_denied_other_identity",
    (
        "Access denied when trying to assume role %s. Please ensure the trust policy of %s "
        "allows the current role to assume it. Current identity: check logs with verbose mode."
    ): "irsa_denied_trust_policy",
}


class StsError(Exception):
    def __init__(self, code: str, message: str, action: str) -> None:
        super().__init__(
            f"An error occurred ({code}) when calling the {action} operation: {message}"
        )


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def one(value: Json, name: str) -> tuple[str, Json]:
    if not isinstance(value, dict) or len(value) != 1:
        raise ValueError(f"{name} must have exactly one member")
    return next(iter(value.items()))


class FakeSts:
    def __init__(self, answers: dict[str, list[Json]], environment: Mapping[str, str]) -> None:
        self.answers = answers
        self.environment = environment
        self.calls: list[Json] = []
        self.warnings = WarningLog()
        self.access_key_id: str | None = None
        self.session_token: str | None = None

    def client(self, _service: str, **kwargs: object) -> "FakeSts":
        access = kwargs.get("aws_access_key_id")
        token = kwargs.get("aws_session_token")
        self.access_key_id = (
            access if isinstance(access, str) else self.environment.get("AWS_ACCESS_KEY_ID")
        )
        self.session_token = (
            token if isinstance(token, str) else self.environment.get("AWS_SESSION_TOKEN")
        )
        return self

    def assume_role(self, **params: object) -> dict[str, object]:
        return self.record("AssumeRole", params)

    def assume_role_with_web_identity(self, **params: object) -> dict[str, object]:
        return self.record("AssumeRoleWithWebIdentity", params)

    def get_caller_identity(self) -> dict[str, object]:
        return self.record("GetCallerIdentity", {})

    def record(self, action: str, params: Mapping[str, object]) -> dict[str, object]:
        unsigned = action == "AssumeRoleWithWebIdentity"
        form: dict[str, str] = {"Action": action, "Version": "2011-06-15"}
        for name, value in params.items():
            if isinstance(value, (list, tuple)):
                entries = cast("Sequence[Mapping[str, str]]", value)
                for index, entry in enumerate(entries, start=1):
                    for field, item in entry.items():
                        form[f"{name}.member.{index}.{field}"] = item
            else:
                form[name] = str(value)
        call = cast(
            Json,
            {
                "action": action,
                "form": form,
                "access_key_id": None if unsigned else self.access_key_id,
                "session_token": None if unsigned else self.session_token,
            },
        )
        self.calls.append(call)
        queue = self.answers.get(CASE_KEYS[action], [])
        if not queue:
            raise RuntimeError(f"the fixture has no answer for {action}")
        return self.reply(action, queue.pop(0))

    def reply(self, action: str, answer: Json) -> dict[str, object]:
        value = object_value(answer, "reply")
        if "error" in value:
            error = object_value(value["error"], "error reply")
            raise StsError(
                string_value(error.get("code"), "code"),
                string_value(error.get("message"), "message"),
                action,
            )
        if "identity" in value:
            identity = object_value(value["identity"], "identity")
            return {"Arn": string_value(identity.get("arn"), "arn"), "UserId": "fixture"}
        credentials = object_value(value.get("credentials"), "credentials")
        reply: dict[str, object] = {
            "Credentials": {
                "AccessKeyId": string_value(credentials.get("access_key_id"), "access_key_id"),
                "SecretAccessKey": string_value(
                    credentials.get("secret_access_key"), "secret_access_key"
                ),
                "SessionToken": string_value(credentials.get("session_token"), "session_token"),
                "Expiration": datetime.fromisoformat(
                    string_value(credentials.get("expiration"), "expiration")
                ),
            }
        }
        packed = credentials.get("packed_policy_size")
        if packed is not None:
            reply["PackedPolicySize"] = packed
        return reply


class FixedClock:
    def __init__(self, epoch: int) -> None:
        self.epoch = epoch

    def now(self, tz: tzinfo | None = None) -> datetime:
        fixed = datetime.fromtimestamp(self.epoch, tz=UTC)
        return fixed if tz is None else fixed.astimezone(tz)


class WarningLog:
    def __init__(self) -> None:
        self.entries: list[Json] = []

    def warning(self, message: str, *args: object) -> None:
        self.record("warn", message, args)

    def error(self, message: str, *args: object) -> None:
        self.record("error", message, args)

    def record(self, level: str, message: str, args: tuple[object, ...]) -> None:
        code = CODES.get(message)
        if code is None:
            return
        role = args[0] if args and isinstance(args[0], str) else None
        self.entries.append(cast(Json, {"level": level, "code": code, "role": role}))


def params_of(step: Params) -> Params:
    return object_value(step.get("params", {}), "params")


def steps_of(case: Params) -> list[Params]:
    raw = case.get("steps")
    if not isinstance(raw, list):
        raise ValueError("steps must be a list")
    return [object_value(step, "step") for step in raw]


def environment_of(case: Params) -> dict[str, str]:
    raw = object_value(case.get("environment", {}), "environment")
    return {name: string_value(value, name) for name, value in raw.items()}


def files_of(case: Params) -> dict[str, str]:
    raw = object_value(case.get("files", {}), "files")
    return {path: string_value(value, path) for path, value in raw.items()}


def answers_of(case: Params) -> dict[str, list[Json]]:
    raw = object_value(case.get("answers", {}), "answers")
    answers: dict[str, list[Json]] = {}
    for name, value in raw.items():
        if not isinstance(value, list):
            raise ValueError(f"answers.{name} must be a list")
        answers[name] = value
    return answers


def mapped_environment(
    environment: Mapping[str, str], files: Mapping[str, str], folder: str
) -> dict[str, str]:
    written: dict[str, str] = {}
    for index, (path, content) in enumerate(files.items()):
        target = Path(folder) / f"{index}-{Path(path).name}"
        target.write_text(content, encoding="utf-8")
        written[path] = str(target)
    return {name: written.get(value, value) for name, value in environment.items()}


@contextmanager
def harness(case: Params) -> Generator[FakeSts, None, None]:
    raw_now = case.get("now", DEFAULT_NOW)
    if not isinstance(raw_now, int) or isinstance(raw_now, bool):
        raise ValueError("now must be an integer")
    now = raw_now
    with TemporaryDirectory() as folder:
        mapped = mapped_environment(environment_of(case), files_of(case), folder)
        sts = FakeSts(answers_of(case), mapped)
        with (
            patch.dict(os.environ, mapped, clear=True),
            patch("boto3.client", side_effect=sts.client),
            patch.object(base_aws_llm, "datetime", FixedClock(now)),
            patch.object(base_aws_llm, "get_secret", return_value=case.get("resolved_token")),
            patch.object(base_aws_llm, "BEDROCK_MAX_POLICY_SIZE", 75),
            patch.object(verbose_logger, "warning", new=sts.warnings.warning),
            patch.object(verbose_logger, "error", new=sts.warnings.error),
        ):
            yield sts


def status_of(error: Exception) -> int | None:
    carried: object = getattr(error, "status_code", None)
    return carried if isinstance(carried, int) and not isinstance(carried, bool) else None


def credentials_json(credentials: object) -> Json:
    return cast(
        Json,
        [
            credentials.access_key,  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType, reportUnknownVariableType]
            credentials.secret_key,  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType, reportUnknownVariableType]
            credentials.token,  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType, reportUnknownVariableType]
        ],
    )


def invoke(operation: str, step: Params) -> tuple[object, Json]:
    params = params_of(step)
    if operation == "get_credentials":
        credentials = BaseAWSLLM().get_credentials(
            aws_access_key_id=cast("str | None", params.get("aws_access_key_id")),
            aws_secret_access_key=cast("str | None", params.get("aws_secret_access_key")),
            aws_session_token=cast("str | None", params.get("aws_session_token")),
            aws_region_name=cast("str | None", params.get("aws_region_name")),
            aws_session_name=cast("str | None", params.get("aws_session_name")),
            aws_profile_name=cast("str | None", params.get("aws_profile_name")),
            aws_role_name=cast("str | None", params.get("aws_role_name")),
            aws_web_identity_token=cast("str | None", params.get("aws_web_identity_token")),
            aws_sts_endpoint=cast("str | None", params.get("aws_sts_endpoint")),
            aws_external_id=cast("str | None", params.get("aws_external_id")),
            aws_session_tags=cast(
                "Sequence[AwsSessionTag] | None", params.get("aws_session_tags")
            ),
            ssl_verify=cast("bool | str | None", step.get("ssl_verify")),
        )
        return credentials, None
    if operation == "resolve_credentials":
        return (
            BaseAWSLLM().resolve_credentials(
                AwsAuthParams.model_validate(params), cast("str | None", step.get("region"))
            ),
            None,
        )
    if operation == "resolve_s3_credentials":
        return (
            BaseAWSLLM().resolve_s3_credentials(
                cast("Mapping[str, object]", params), cast("str | None", step.get("region"))
            ),
            None,
        )
    credentials, ttl = BaseAWSLLM()._auth_with_aws_role(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
        aws_access_key_id=cast("str | None", params.get("aws_access_key_id")),
        aws_secret_access_key=cast("str | None", params.get("aws_secret_access_key")),
        aws_session_token=cast("str | None", params.get("aws_session_token")),
        aws_role_name=string_value(params.get("aws_role_name"), "aws_role_name"),
        aws_session_name=string_value(params.get("aws_session_name", ""), "aws_session_name"),
        aws_region_name=cast("str | None", params.get("aws_region_name")),
        aws_sts_endpoint=cast("str | None", params.get("aws_sts_endpoint")),
        aws_external_id=cast("str | None", params.get("aws_external_id")),
        aws_session_tags=cast("Sequence[AwsSessionTag] | None", params.get("aws_session_tags")),
        ssl_verify=True,
    )
    return credentials, cast(Json, ttl)


def run_step(operation: str, harnessed: FakeSts, step: Params) -> Json:
    recorded = len(harnessed.calls)
    try:
        credentials, ttl = invoke(operation, step)
    except Exception as error:  # noqa: BLE001 # the helper records every fixture failure as a step outcome
        return cast(
            Json,
            {"ok": False, "status": status_of(error), "calls": harnessed.calls[recorded:]},
        )
    return cast(
        Json,
        {
            "ok": True,
            "credentials": credentials_json(credentials),
            "ttl": ttl,
            "calls": harnessed.calls[recorded:],
        },
    )


def run_case(operation: str, case: Params) -> Json:
    with harness(case) as harnessed:
        steps = [run_step(operation, harnessed, step) for step in steps_of(case)]
        return cast(Json, {"steps": steps, "logs": harnessed.warnings.entries})


HANDLERS: dict[str, Callable[[Params], Json]] = {
    operation: (lambda case, name=operation: run_case(name, case))
    for operation in (
        "get_credentials",
        "resolve_credentials",
        "resolve_s3_credentials",
        "auth_with_aws_role",
    )
}


def run(input_value: Params) -> Json:
    kind, payload = one(cast(Json, input_value), "input")
    handler = HANDLERS.get(kind)
    if handler is None:
        raise ValueError(f"unknown operation: {kind}")
    _ = litellm
    return handler(cast(Params, payload))
