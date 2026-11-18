import asyncio
from typing import Final, Protocol, TypedDict, runtime_checkable

from fastapi_sso.sso.base import OpenID
from fastapi_sso.sso.google import GoogleSSO
from litellm.proxy.management_endpoints.types import CustomOpenID
from pydantic import ConfigDict, JsonValue, TypeAdapter

CLIENT_VALUE: Final = "client-secret"
ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)


class Input(TypedDict):
    userinfo: dict[str, JsonValue]


class Identity(TypedDict):
    id: str | None
    email: str | None
    first_name: str | None
    last_name: str | None
    display_name: str | None
    picture: str | None
    provider: str | None
    team_ids: list[str]
    user_role: str | None
    extra_fields: dict[str, JsonValue] | None


IDENTITY: Final = TypeAdapter(Identity)


@runtime_checkable
class Converter(Protocol):
    async def openid_from_response(self, response: dict[str, JsonValue]) -> OpenID: ...


CONVERTER: Final = TypeAdapter[Converter](Converter, config=ARBITRARY)


def run(input_value: Input) -> Identity:
    converter = CONVERTER.validate_python(
        GoogleSSO(client_id="client-id", client_secret=CLIENT_VALUE, redirect_uri=None)
    )
    openid = asyncio.run(converter.openid_from_response(input_value["userinfo"]))
    record = CustomOpenID(
        id=openid.id,
        email=openid.email,
        first_name=openid.first_name,
        last_name=openid.last_name,
        display_name=openid.display_name,
        picture=openid.picture,
        provider=openid.provider,
        team_ids=[],
    )
    return IDENTITY.validate_json(record.model_dump_json())
