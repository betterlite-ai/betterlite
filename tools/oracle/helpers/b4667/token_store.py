from litellm.proxy._experimental.mcp_server.outbound_credentials.oauth_token_store import (
    OAuthToken,
    TokenStoreUnavailable,
)

from b4667.wire import TokenStoreInput


class FakeTokens:
    def __init__(self, token: OAuthToken | None, *, unavailable: bool) -> None:
        self._token = token
        self._unavailable = unavailable

    async def fetch(self, user_id: str, server_id: str) -> OAuthToken | None:
        del user_id, server_id
        if self._unavailable:
            raise TokenStoreUnavailable
        return self._token


def build_token_store(store: TokenStoreInput | None) -> FakeTokens:
    if store is None:
        return FakeTokens(None, unavailable=False)
    match store["kind"]:
        case "token":
            return FakeTokens(OAuthToken(access_token=store["access_token"]), unavailable=False)
        case "unavailable":
            return FakeTokens(None, unavailable=True)
