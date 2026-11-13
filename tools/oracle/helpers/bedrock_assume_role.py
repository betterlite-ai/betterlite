import os
from collections.abc import Callable
from datetime import UTC, datetime, tzinfo
from types import SimpleNamespace
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from botocore.credentials import Credentials
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from pydantic import BaseModel, ConfigDict, JsonValue

Json: TypeAlias = JsonValue
Params: TypeAlias = dict[str, Json]


class Input(BaseModel):
    model_config = ConfigDict(extra="forbid")
    operation: str
    params: Params


class FixtureSts:
    def __init__(self, reply: Json) -> None:
        self.reply = reply
        self.identity_calls = 0
        self.assume_calls = 0

    def get_caller_identity(self) -> dict[str, str]:
        self.identity_calls += 1
        if isinstance(self.reply, dict):
            return {"Arn": cast(str, self.reply["identity"])}
        if self.reply == "no_arn":
            return {}
        raise RuntimeError(self.reply)

    def assume_role(self, **_kwargs: object) -> None:
        self.assume_calls += 1


def parse_arn(params: Params) -> Json:
    found = BaseAWSLLM._parse_arn_account_and_role_name(str(params["arn"]))  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
    return {"value": None if found is None else list(found)}


def already_running(params: Params) -> Json:
    sts = FixtureSts(params["reply"])
    environment = cast(dict[str, str], params["environment"])
    with patch.dict(os.environ, environment, clear=True), patch("boto3.client", return_value=sts):
        matches = BaseAWSLLM()._is_already_running_as_role(str(params["target"]))  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
    return {
        "matches": matches,
        "sts_calls": sts.identity_calls,
        "assume_role_calls": sts.assume_calls,
    }


def extract_ttl(params: Params) -> Json:
    response = cast(dict[str, Params], params["sts_response"])
    expiration = datetime.fromisoformat(cast(str, response["Credentials"]["Expiration"]))
    now = datetime.fromisoformat(cast(str, params["now"]))

    def clock(zone: tzinfo | None = None) -> datetime:
        return now.astimezone(UTC).replace(tzinfo=None) if zone is None else now.astimezone(zone)

    sts_response: dict[str, object] = {
        "Credentials": {**response["Credentials"], "Expiration": expiration}
    }
    with patch("litellm.llms.bedrock.base_aws_llm.datetime", SimpleNamespace(now=clock)):
        extractor = cast(
            Callable[[dict[str, object]], tuple[Credentials, int]],
            BaseAWSLLM()._extract_credentials_and_ttl,  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        )
        credentials, ttl = extractor(sts_response)
    token = cast(str | None, credentials.token)  # pyright: ignore[reportUnknownMemberType]
    return {"credentials": [credentials.access_key, credentials.secret_key, token], "ttl": ttl}


HANDLERS: dict[str, Callable[[Params], Json]] = {
    "parse_arn": parse_arn,
    "already_running": already_running,
    "extract_ttl": extract_ttl,
}


def run(argument: Json) -> Json:
    supplied = Input.model_validate(argument)
    handler = HANDLERS.get(supplied.operation)
    if handler is None:
        raise ValueError(f"unknown operation: {supplied.operation}")
    _ = litellm
    return handler(supplied.params)
