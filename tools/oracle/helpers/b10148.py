import asyncio
import os
from typing import TypedDict

from fastapi_sso.sso.base import DiscoveryDocument
from litellm.proxy.management_endpoints.sso import CustomMicrosoftSSO


class Input(TypedDict):
    client_id: str
    redirect_url: str
    environment: dict[str, str]


class ClientView(TypedDict):
    tenant: str
    discovery: DiscoveryDocument


def run(input_value: Input) -> ClientView:
    previous = dict(os.environ)
    os.environ.clear()
    os.environ.update(input_value["environment"])
    try:
        client = CustomMicrosoftSSO(
            client_id=input_value["client_id"],
            client_secret=input_value["environment"]["MICROSOFT_CLIENT_SECRET"],
            tenant=os.getenv("MICROSOFT_TENANT"),
            redirect_uri=input_value["redirect_url"],
            allow_insecure_http=True,
        )
        document = asyncio.run(client.get_discovery_document())
        return {"tenant": client.tenant, "discovery": document}
    finally:
        os.environ.clear()
        os.environ.update(previous)
