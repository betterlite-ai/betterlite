import asyncio
from collections.abc import Coroutine
from typing import Protocol

from litellm.types.llms.bedrock import AwsAuthParams
from pydantic import JsonValue

from .fakes import ClientGoneError, ClientSendError, CredentialLookupError
from .models import COMMITTED_KEY, FAILURE_KEY, PENDING_KEY, Auth, Run


class FakeWebSocket:
    def __init__(self, run: Run) -> None:
        self.scope: dict[str, JsonValue] = {}
        if run["committed"]:
            self.scope[COMMITTED_KEY] = True
        if run["failure"] is not None:
            self.scope[FAILURE_KEY] = run["failure"]
        if run["pending"] is not None:
            self.scope[PENDING_KEY] = run["pending"]
        self.frames = list(run["frames"])
        self.block = run["block"] == "client"
        self.send_failure = run["client_send_failure"]
        self.sent: list[str] = []
        self.close_code: int | None = None

    async def receive_text(self) -> str:
        if self.frames:
            return self.frames.pop(0)
        if self.block:
            await asyncio.Future[None]()
        raise ClientGoneError("the client is gone")

    async def send_text(self, data: str) -> None:
        index = len(self.sent)
        self.sent.append(data)
        if self.send_failure == index:
            raise ClientSendError("the client send failed")

    async def close(self, code: int = 1000, reason: str | None = None) -> None:
        _ = reason
        if self.close_code is None:
            self.close_code = code


class Frozen:
    def __init__(self, auth: Auth | None) -> None:
        self.access_key = "" if auth is None else auth["access_key_id"]
        self.secret_key = "" if auth is None else auth["secret_access_key"]
        self.token = None if auth is None else auth["session_token"]


class FrozenProvider(Protocol):
    def get_frozen_credentials(self) -> Frozen: ...


class FakeCredentials(FrozenProvider):
    def __init__(self, auth: Auth | None) -> None:
        self.frozen = Frozen(auth)

    def get_frozen_credentials(self) -> Frozen:
        return self.frozen


class CredentialResolver:
    def __init__(self, run: Run) -> None:
        self.mode = run["credentials"]
        self.auth = run["auth"]

    def __call__(
        self, _auth_params: AwsAuthParams, _aws_region_name: str | None
    ) -> FakeCredentials | None:
        if self.mode == "absent":
            return None
        if self.mode == "error":
            raise CredentialLookupError("aws_session_tags")
        return FakeCredentials(self.auth)


class DiscardingWorker:
    def __call__(self, work: Coroutine[None, None, None]) -> None:
        work.close()
