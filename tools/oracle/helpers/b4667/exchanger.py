from litellm.proxy._experimental.mcp_server.outbound_credentials.oauth_token_store import (
    OAuthToken,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.result import (
    Error,
    Ok,
    Result,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.types import (
    CredError,
    ServerSpec,
    TokenExchangeConfig,
)
from pydantic import JsonValue

from b4667.errors import error_from_tag
from b4667.wire import ExchangerInput


class FakeExchanger:
    def __init__(self, token: str | None, error: CredError | None) -> None:
        self._token = token
        self._error = error
        self.exchanges: list[dict[str, JsonValue]] = []
        self.invalidations: list[dict[str, JsonValue]] = []

    async def exchange(
        self,
        subject_token: str,
        server: ServerSpec,
        config: TokenExchangeConfig,
        *,
        tenant_id: str = "",
    ) -> Result[OAuthToken, CredError]:
        del server, config
        self.exchanges.append({"subject_token": subject_token, "tenant_id": tenant_id})
        if self._error is not None:
            return Error(self._error)
        if self._token is None:
            return Error(CredError.of_misconfigured("token exchange collaborator not wired"))
        return Ok(OAuthToken(access_token=self._token))

    async def invalidate(
        self,
        subject_token: str,
        server: ServerSpec,
        config: TokenExchangeConfig,
        *,
        tenant_id: str = "",
    ) -> None:
        del server, config
        record: dict[str, JsonValue] = {"subject_token": subject_token, "tenant_id": tenant_id}
        self.invalidations.append(record)


def build_exchanger(exchanger: ExchangerInput | None) -> FakeExchanger:
    if exchanger is None:
        not_wired = CredError.of_misconfigured("token exchange collaborator not wired")
        return FakeExchanger(None, not_wired)
    match exchanger["kind"]:
        case "token":
            return FakeExchanger(exchanger["access_token"], None)
        case "failure":
            return FakeExchanger(None, error_from_tag(exchanger["tag"], exchanger["detail"]))
