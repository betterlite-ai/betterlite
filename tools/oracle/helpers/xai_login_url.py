from typing import TypedDict, cast
from unittest.mock import patch

from litellm.llms.xai.oauth import XAIOAuthAuthenticator
from pydantic import JsonValue


class Input(TypedDict):
    entropy: list[int]
    authorization_endpoint: str
    redirect_uri: str
    state: str
    nonce: str


class Fixture(XAIOAuthAuthenticator):
    def evaluate(self, case: Input) -> JsonValue:
        def token_bytes(_count: int) -> bytes:
            return bytes(case["entropy"])

        with patch("litellm.llms.xai.oauth.secrets.token_bytes", token_bytes):
            verifier, challenge = self._pkce_pair()
        url = self._build_authorize_url(
            case["authorization_endpoint"],
            case["redirect_uri"],
            challenge,
            case["state"],
            case["nonce"],
        )
        return cast(JsonValue, {"verifier": verifier, "challenge": challenge, "url": url})


def run(input_value: Input) -> JsonValue:
    return Fixture().evaluate(input_value)
