import asyncio
import os
from collections.abc import Generator
from contextlib import contextmanager
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


@contextmanager
def case_environment(values: dict[str, str]) -> Generator[None]:
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(values)
    try:
        yield
    finally:
        os.environ.clear()
        os.environ.update(saved)


def run(input_value: Input) -> ClientView:
    environment = input_value["environment"]
    with case_environment(environment):
        client = CustomMicrosoftSSO(
            client_id=input_value["client_id"],
            client_secret=environment["MICROSOFT_CLIENT_SECRET"],
            tenant=os.getenv("MICROSOFT_TENANT"),
            redirect_uri=input_value["redirect_url"],
            allow_insecure_http=True,
        )
        document = asyncio.run(client.get_discovery_document())
        return {"tenant": client.tenant, "discovery": document}
