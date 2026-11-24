from collections.abc import Mapping

from litellm.proxy._experimental.mcp_server.outbound_credentials.result import (
    Error,
    Ok,
    Result,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.token_endpoint import (
    ExchangedToken,
    TokenEndpointClient,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.types import (
    ClientAuth,
    CredError,
)
from pydantic import JsonValue
from typing_extensions import override

from b4667.configs import client_auth_answer
from b4667.wire import JSON_ADAPTER


class FakeEndpoint(TokenEndpointClient):
    def __init__(self, tokens: list[str]) -> None:
        super().__init__()
        self._tokens = list(tokens)
        self.calls: list[dict[str, JsonValue]] = []

    @override
    async def fetch(
        self,
        endpoint: str,
        client_id: str,
        grant_params: Mapping[str, str],
        client_auth: ClientAuth,
    ) -> Result[ExchangedToken, CredError]:
        params: dict[str, str] = dict(grant_params)
        self.calls.append(
            {
                "endpoint": endpoint,
                "client_id": client_id,
                "params": JSON_ADAPTER.validate_python(params),
                "client_auth": client_auth_answer(client_auth),
            }
        )
        if not self._tokens:
            return Error(CredError.of_upstream_unavailable("no canned token endpoint reply"))
        return Ok(ExchangedToken(access_token=self._tokens.pop(0), expires_in=None))
