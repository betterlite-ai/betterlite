from litellm.proxy._experimental.mcp_server.outbound_credentials.client_credentials import (
    ClientCredentialsTokenSource,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.oauth_token_store import (
    OAuthToken,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.result import (
    Error,
    Ok,
    Result,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.types import (
    ClientCredentialsConfig,
    CredError,
)
from typing_extensions import override

from b4667.wire import CredentialsFake


class FakeCredentials(ClientCredentialsTokenSource):
    def __init__(self, access_token: str | None, refetch: str | None = None) -> None:
        super().__init__()
        self._access_token = access_token
        self._refetched = refetch

    @override
    async def get(
        self, server_id: str, config: ClientCredentialsConfig
    ) -> Result[OAuthToken, CredError]:
        del server_id, config
        if self._access_token is None:
            return Error(CredError.of_misconfigured("client_credentials collaborator not wired"))
        return Ok(OAuthToken(access_token=self._access_token))

    @override
    async def refetch(
        self,
        server_id: str,
        config: ClientCredentialsConfig,
        failed_access_token: str,
    ) -> str | None:
        del server_id, config, failed_access_token
        return self._refetched


def build_credentials(credentials: CredentialsFake | None) -> FakeCredentials:
    if credentials is None:
        return FakeCredentials(None)
    return FakeCredentials(credentials["access_token"], refetch=credentials["refetch"])
