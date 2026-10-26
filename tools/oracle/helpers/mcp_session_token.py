import secrets
from datetime import UTC, datetime
from typing import TypeAlias
from unittest.mock import patch

from litellm.proxy._experimental.mcp_server.outbound_credentials import session_token as tokens

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def step_of(fields: dict[str, Json], name: str) -> dict[str, Json] | None:
    step = fields.get(name)
    if step is not None and not isinstance(step, dict):
        raise ValueError(f"{name} must be an object")
    return step


def keys_of(step: dict[str, Json]) -> tokens.SessionSigningKeys:
    keys = step["keys"]
    if isinstance(keys, dict) and "signing_key" in keys:
        return tokens.SessionKeys.model_validate(keys)
    return tokens.AsymmetricSessionKeys.model_validate(keys)


def clock(step: dict[str, Json]) -> datetime:
    now = step["now"]
    if not isinstance(now, int):
        raise ValueError("now must be an integer")
    return datetime.fromtimestamp(now, UTC)


def mint(step: dict[str, Json]) -> dict[str, Json]:
    entropy = step["entropy"]
    if not isinstance(entropy, str):
        raise ValueError("entropy must be hex text")
    principal = tokens.SessionPrincipal.model_validate(step["principal"])
    keys = keys_of(step)
    minter = tokens.mint_session_refresh_token if step["kind"] == "session_refresh" else tokens.mint_session_token
    with patch.object(secrets, "token_bytes", return_value=bytes.fromhex(entropy)):
        minted = minter(principal, keys, clock(step))
    if isinstance(minted, tokens.SessionTokenTooLarge):
        return minted.model_dump()
    public = tokens.session_public_key_pem(keys) if isinstance(keys, tokens.AsymmetricSessionKeys) else None
    return {"token": minted.token.get_secret_value(), "expires_at": int(minted.expires_at.timestamp()), "public_key_pem": public}


def open_token(step: dict[str, Json], minted: Json) -> dict[str, Json]:
    candidate = step.get("candidate")
    if candidate is None and isinstance(minted, dict):
        candidate = minted.get("token")
    if not isinstance(candidate, str):
        raise ValueError("open needs a candidate or a minted token")
    opener = tokens.open_session_refresh_token if step["kind"] == "session_refresh" else tokens.open_session_token
    return opener(candidate, keys_of(step), clock(step)).model_dump(mode="json")


def run(fields: dict[str, Json]) -> Json:
    mint_step, open_step = step_of(fields, "mint"), step_of(fields, "open")
    minted = None if mint_step is None else mint(mint_step)
    opened = None if open_step is None else open_token(open_step, minted)
    return {"mint": minted, "open": opened}
