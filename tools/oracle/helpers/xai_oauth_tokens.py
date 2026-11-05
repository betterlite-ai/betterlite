from typing import NotRequired, TypedDict, cast
from unittest.mock import patch

from litellm.llms.xai.oauth import XAIOAuthAuthenticator, XAIOAuthError, XAIOAuthRecord
from pydantic import JsonValue


class Payload(TypedDict, total=False):
    access_token: str
    refresh_token: str
    id_token: str | None
    token_type: str
    expires_in: float


class Expired(TypedDict):
    now: float
    expires_at: NotRequired[JsonValue]


class Record(TypedDict):
    now: float
    payload: Payload
    token_endpoint: str
    fallback: str | None


class ExpiredInput(TypedDict):
    expired: Expired


class EndpointInput(TypedDict):
    endpoint: str


class RecordInput(TypedDict):
    record: Record


Input = ExpiredInput | EndpointInput | RecordInput


class Fixture(XAIOAuthAuthenticator):
    def evaluate(self, input_value: Input) -> JsonValue:
        if "endpoint" in input_value:
            try:
                _ = self._validate_xai_endpoint(input_value["endpoint"])
            except XAIOAuthError:
                return {"valid": False}
            return {"valid": True}
        if "expired" in input_value:
            case = input_value["expired"]
            auth = cast(XAIOAuthRecord, {"expires_at": case.get("expires_at")})
            with patch("litellm.llms.xai.oauth.time.time", lambda: case["now"]):
                return {"expired": self._is_expired(auth)}
        record = input_value["record"]
        with patch("litellm.llms.xai.oauth.time.time", lambda: record["now"]):
            try:
                built = self._build_auth_record(
                    record["payload"], record["token_endpoint"], record["fallback"]
                )
            except XAIOAuthError as error:
                return {"error": str(error)}
        return {"record": cast(JsonValue, dict(built))}


def run(input_value: Input) -> JsonValue:
    return Fixture().evaluate(input_value)
