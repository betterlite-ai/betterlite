import secrets
from datetime import datetime
from unittest.mock import patch

from litellm.proxy._experimental.mcp_server.outbound_credentials import session_token as tokens
from pydantic import BaseModel, JsonValue, TypeAdapter


class Step(BaseModel):
    keys: tokens.SessionSigningKeys
    kind: tokens.SessionTokenKind
    now: datetime


class MintStep(Step):
    principal: tokens.SessionPrincipal
    entropy: list[int]


class OpenStep(Step):
    candidate: str | None = None


def run(fields: dict[str, JsonValue]) -> JsonValue:
    minting = TypeAdapter(MintStep | None).validate_python(fields.get("mint"))
    opening = TypeAdapter(OpenStep | None).validate_python(fields.get("open"))
    minted: dict[str, JsonValue] = {}
    if minting is not None:
        refresh = minting.kind == "session_refresh"
        minter = tokens.mint_session_refresh_token if refresh else tokens.mint_session_token
        with patch.object(secrets, "token_bytes", return_value=bytes(minting.entropy)):
            result = minter(minting.principal, minting.keys, minting.now)
        if isinstance(result, tokens.SessionTokenTooLarge):
            minted = result.model_dump()
        else:
            public = None
            if isinstance(minting.keys, tokens.AsymmetricSessionKeys):
                public = tokens.session_public_key_pem(minting.keys)
            expires_at = int(result.expires_at.timestamp())
            token = result.token.get_secret_value()
            minted = {"token": token, "expires_at": expires_at, "public_key_pem": public}
    opened: JsonValue = None
    if opening is not None:
        candidate = opening.candidate
        if candidate is None:
            candidate = str(minted.get("token") or "")
        refreshing = opening.kind == "session_refresh"
        opener = tokens.open_session_refresh_token if refreshing else tokens.open_session_token
        opened = opener(candidate, opening.keys, opening.now).model_dump(mode="json")
    return {"mint": minted or None, "open": opened}
