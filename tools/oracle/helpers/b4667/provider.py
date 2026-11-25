from litellm.proxy._experimental.mcp_server.outbound_credentials.resolver import (
    UpstreamCredentialProvider,
)

from b4667.assertion_store import build_assertion_store
from b4667.credentials import build_credentials
from b4667.endpoint import FakeEndpoint
from b4667.exchanger import FakeExchanger, build_exchanger
from b4667.token_store import build_token_store
from b4667.wire import CollaboratorsInput


def build_provider(
    collaborators: CollaboratorsInput | None,
) -> tuple[UpstreamCredentialProvider, FakeEndpoint, FakeExchanger]:
    fields: CollaboratorsInput = collaborators if collaborators is not None else {}
    tokens: list[str] = []
    if "token_endpoint" in fields:
        tokens = fields["token_endpoint"]
    endpoint = FakeEndpoint(tokens)
    exchanger = build_exchanger(fields.get("exchanger"))
    provider = UpstreamCredentialProvider(
        oauth_token_store=build_token_store(fields.get("oauth_token_store")),
        token_exchanger=exchanger,
        token_endpoint=endpoint,
        client_credentials_source=build_credentials(fields.get("client_credentials")),
        sso_assertion_store=build_assertion_store(fields.get("assertion_store")),
    )
    return provider, endpoint, exchanger
