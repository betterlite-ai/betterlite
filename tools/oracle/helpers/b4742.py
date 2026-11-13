import os
from datetime import UTC, datetime
from unittest.mock import patch

import httpx2
import litellm
from litellm.experimental_mcp_client.client import MCPSigV4Auth
from pydantic import BaseModel, ConfigDict, JsonValue

STS_CREDENTIALS = {
    "AccessKeyId": "ASIATEMPORARY",
    "SecretAccessKey": "temporarysecret",
    "SessionToken": "temporarytoken",
}
SIGNATURE_HEADERS = ("authorization", "x-amz-date", "x-amz-security-token")


class Settings(BaseModel):
    model_config = ConfigDict(extra="forbid")
    aws_access_key_id: str | None = None
    aws_secret_access_key: str | None = None
    aws_session_token: str | None = None
    aws_region_name: str | None = None
    aws_service_name: str | None = None
    aws_role_name: str | None = None
    aws_session_name: str | None = None


class Call(BaseModel):
    model_config = ConfigDict(extra="forbid")
    method: str
    url: str
    headers: list[tuple[str, str]] = []
    body: str = ""


class Input(BaseModel):
    model_config = ConfigDict(extra="forbid")
    settings: Settings
    environment: dict[str, str] = {}
    now: int
    requests: list[Call]


class FixtureSts:
    def __init__(self) -> None:
        self.calls: list[JsonValue] = []
        self.kwargs: dict[str, str] = {}

    def client(self, _service: str, **kwargs: str) -> "FixtureSts":
        self.kwargs = kwargs
        return self

    def assume_role(self, RoleArn: str, RoleSessionName: str) -> dict[str, dict[str, str]]:  # noqa: N803
        self.calls.append(
            {
                "region": self.kwargs.get("region_name"),
                "access_key": self.kwargs.get("aws_access_key_id"),
                "session_token": self.kwargs.get("aws_session_token"),
                "role_arn": RoleArn,
                "session_name": RoleSessionName,
            }
        )
        return {"Credentials": STS_CREDENTIALS}


def signed_headers(auth: MCPSigV4Auth, call: Call) -> dict[str, JsonValue]:
    body = call.body.encode() or None
    request = httpx2.Request(call.method, call.url, headers=call.headers, content=body)
    signed = next(auth.auth_flow(request))
    return {name: signed.headers.get(name) for name in SIGNATURE_HEADERS}


def run(argument: JsonValue) -> JsonValue:
    supplied = Input.model_validate(argument)
    settings = supplied.settings
    now = datetime.fromtimestamp(supplied.now, UTC).replace(tzinfo=None)
    sts = FixtureSts()
    _ = litellm
    with (
        patch.dict(os.environ, supplied.environment, clear=True),
        patch("boto3.client", side_effect=sts.client),
        patch("time.time", return_value=float(supplied.now)),
    ):
        try:
            auth = MCPSigV4Auth(
                aws_access_key_id=settings.aws_access_key_id,
                aws_secret_access_key=settings.aws_secret_access_key,
                aws_session_token=settings.aws_session_token,
                aws_region_name=settings.aws_region_name,
                aws_service_name=settings.aws_service_name,
                aws_role_name=settings.aws_role_name,
                aws_session_name=settings.aws_session_name,
            )
        except ValueError as error:
            return {"error": type(error).__name__, "sts_calls": sts.calls, "signed": []}
    with patch("botocore.auth.get_current_datetime", return_value=now):
        signed: list[JsonValue] = [signed_headers(auth, call) for call in supplied.requests]
    return {"error": None, "sts_calls": sts.calls, "signed": signed}
