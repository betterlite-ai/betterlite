from __future__ import annotations

import json
from typing import Final, Literal, cast

from pydantic import BaseModel, ConfigDict, Field, TypeAdapter
from typing_extensions import TypedDict


class Credentials(TypedDict, total=False):
    auth_value: str | None
    client_id: str | None
    client_secret: str | None
    scopes: list[str] | None
    aws_access_key_id: str | None
    aws_secret_access_key: str | None
    aws_session_token: str | None
    aws_region_name: str | None
    aws_service_name: str | None
    aws_role_name: str | None
    aws_session_name: str | None
    audience: str | None
    token_exchange_endpoint: str | None
    id_jag_resource_token_endpoint: str | None
    id_jag_resource: str | None
    upstream_resource: str | None
    upstream_token_header: str | None
    client_private_key: str | None
    client_private_key_id: str | None
    client_assertion_signing_alg: str | None
    token_endpoint_auth_method: str | None
    redirect_uris: list[str] | None
    token_exchange_profile: str | None


class PinnedTool(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    description: str = ""
    input_schema: dict[str, object] = Field(default_factory=dict)


PINNED: Final = TypeAdapter(dict[str, PinnedTool] | None)
PROTOCOLS: Final = TypeAdapter(
    Literal["auto", "2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"]
)


def run(input_value: dict[str, object]) -> object:
    function = input_value["function"]
    value = input_value["value"]
    if function == "credential_contract":
        return TypeAdapter(Credentials).validate_python(value)
    if function == "pinned_tool_contract":
        decoded: object = json.loads(value) if isinstance(value, str) and value else value
        return PINNED.validate_python(decoded or None)
    if function == "protocol_metadata":
        metadata = cast("dict[str, object]", value)
        if "protocol_version" in metadata:
            PROTOCOLS.validate_python(metadata["protocol_version"])
        return value
    raise ValueError(f"unknown function: {function}")
