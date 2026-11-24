from datetime import UTC, datetime

from litellm.proxy._experimental.mcp_server.outbound_credentials.sso_assertion_store import (
    AssertionStoreUnavailable,
    SSOIdentityAssertion,
)
from pydantic import SecretStr

from b4667.wire import AssertionInput


class FakeAssertions:
    def __init__(self, assertion: SSOIdentityAssertion | None, *, unavailable: bool) -> None:
        self._assertion = assertion
        self._unavailable = unavailable

    async def fetch(self, user_id: str) -> SSOIdentityAssertion | None:
        del user_id
        if self._unavailable:
            raise AssertionStoreUnavailable
        return self._assertion


def build_assertion(id_token: str, expires_at_unix: int | None) -> SSOIdentityAssertion:
    expires_at: datetime | None = None
    if expires_at_unix is not None:
        expires_at = datetime.fromtimestamp(expires_at_unix, tz=UTC)
    return SSOIdentityAssertion(id_token=SecretStr(id_token), expires_at=expires_at)


def build_assertion_store(store: AssertionInput | None) -> FakeAssertions:
    if store is None:
        return FakeAssertions(None, unavailable=False)
    match store["kind"]:
        case "stored":
            assertion = build_assertion(store["id_token"], store["expires_at_unix"])
            return FakeAssertions(assertion, unavailable=False)
        case "missing":
            return FakeAssertions(None, unavailable=False)
        case "unavailable":
            return FakeAssertions(None, unavailable=True)
