import asyncio
from typing import Final, TypedDict

from fastapi_sso.sso.google import GoogleSSO
from litellm.proxy.management_endpoints.types import CustomOpenID
from pydantic import JsonValue, TypeAdapter


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


def run(input_value: Input) -> Identity:
    google = GoogleSSO(
        client_id="client-id",
        client_secret="client-secret",
        redirect_uri=None,
    )
    openid = asyncio.run(google.openid_from_response(input_value["userinfo"]))
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
