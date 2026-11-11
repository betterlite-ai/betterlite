import os
from datetime import UTC, datetime

from litellm.types.llms.bedrock import AwsAuthParams

from .types import Keys


class FrozenClock:
    def __init__(self, epoch: int) -> None:
        self.moment = datetime.fromtimestamp(epoch, tz=UTC).replace(tzinfo=None)

    def __call__(self) -> datetime:
        return self.moment


class CredentialProvider:
    def __init__(self, keys: Keys) -> None:
        self.access_key = keys["access_key_id"]
        self.secret_key = keys["secret_access_key"]
        self.token = keys.get("session_token")

    def __call__(self, _auth: AwsAuthParams, _region: str | None) -> "CredentialProvider":
        return self


def set_environment(environment: dict[str, str] | None) -> None:
    os.environ.clear()
    os.environ["LITELLM_RUST"] = "false"
    if environment is not None:
        os.environ.update(environment)
