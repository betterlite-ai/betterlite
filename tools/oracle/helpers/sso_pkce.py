import asyncio
import os
from typing import TypedDict
from unittest import mock
from urllib.parse import parse_qs, urlparse

from litellm.proxy.management_endpoints.ui_sso import SSOAuthenticationHandler
from pydantic import JsonValue
from starlette.responses import RedirectResponse


class Input(TypedDict):
    state: str | None
    environment: dict[str, str]


class GenericSso:
    def __enter__(self) -> "GenericSso":
        return self

    def __exit__(self, *_args: object) -> None:
        return None

    async def get_login_redirect(self, **state_params: str) -> RedirectResponse:
        state = state_params["state"]
        return RedirectResponse(
            url=f"https://idp.example/authorize?state={state}", status_code=302
        )


def redirect_query(response: RedirectResponse) -> dict[str, JsonValue]:
    location = response.headers["location"]
    query = parse_qs(urlparse(location).query)
    fields: dict[str, JsonValue] = {}
    for name, values in query.items():
        fields[name] = values[0]
    return fields


def run(input_value: Input) -> JsonValue:
    with mock.patch.dict(os.environ, input_value["environment"], clear=True):
        response = asyncio.run(
            SSOAuthenticationHandler.get_generic_sso_redirect_response(
                GenericSso(),
                state=input_value["state"],
                generic_authorization_endpoint=None,
            )
        )
    if response is None:
        raise ValueError("the generic SSO redirect helper returned no response")
    return redirect_query(response)
