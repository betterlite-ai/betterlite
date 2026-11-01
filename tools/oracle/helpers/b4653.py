import os
from datetime import UTC, datetime
from typing import Literal, NotRequired, TypedDict
from unittest.mock import patch

from litellm.proxy._experimental.mcp_server.outbound_credentials import (
    bridge_credentials,
    envelope,
    session_credentials,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials import (
    session_token as tokens,
)
from pydantic import SecretStr


class ScryptInput(TypedDict):
    function: Literal["scrypt"]
    master_key: str
    domains: list[Literal["envelope_signing", "envelope_encryption", "session_signing"]]


class PrefixCheck(TypedDict):
    family: Literal["bridge", "session"]
    candidate: str


class BearerPrefixStripInput(TypedDict):
    function: Literal["bearer_prefix_strip"]
    checks: list[PrefixCheck]


class EnvelopeMintInput(TypedDict):
    server_id: str
    subject_type: Literal["key_hash", "user_id"]
    subject: str
    access_token: NotRequired[str | None]
    token_type: NotRequired[str | None]
    refresh_token: NotRequired[str | None]
    scope: NotRequired[str | None]
    expires_in: NotRequired[int | None]
    mint_now: int


class EnvelopeInput(TypedDict):
    function: Literal["bridge_envelope", "bridge_refresh"]
    signing_key: str
    encryption_key: str
    open_signing_key: NotRequired[str | None]
    open_encryption_key: NotRequired[str | None]
    candidate: NotRequired[str | None]
    mint: NotRequired[EnvelopeMintInput | None]
    open_now: int
    expected_server_id: str


class SessionMintInput(TypedDict):
    principal: dict[str, object]
    kind: Literal["session", "session_refresh"]
    mint_now: int


class SessionInput(TypedDict):
    function: Literal["session_bearer", "session_refresh"]
    signing_key: str
    open_signing_key: NotRequired[str | None]
    candidate: NotRequired[str | None]
    mint: NotRequired[SessionMintInput | None]
    open_now: int
    expected_client_id: NotRequired[str | None]


class SigningConfigInput(TypedDict):
    function: Literal["signing_config"]
    master_key: str
    general_settings: NotRequired[object | None]
    environment: NotRequired[dict[str, str]]


Input = ScryptInput | BearerPrefixStripInput | EnvelopeInput | SessionInput | SigningConfigInput


def _at(seconds: int) -> datetime:
    return datetime.fromtimestamp(seconds, tz=UTC)


def _require_candidate(candidate: str | None) -> str:
    if candidate is None:
        raise ValueError("either mint or candidate is required")
    return candidate


def _envelope_keys(signing_key: str, encryption_key: str) -> envelope.EnvelopeKeys:
    return envelope.EnvelopeKeys(signing_key=signing_key, encryption_key=encryption_key)  # type: ignore[arg-type]


def _envelope_identity(mint: EnvelopeMintInput) -> envelope.EnvelopeIdentity:
    return envelope.EnvelopeIdentity(
        server_id=mint["server_id"],
        subject_type=mint["subject_type"],
        subject=mint["subject"],
    )


def _open_bridge_keys(
    input_value: EnvelopeInput, mint_keys: envelope.EnvelopeKeys
) -> envelope.EnvelopeKeys:
    open_signing = input_value.get("open_signing_key")
    open_encryption = input_value.get("open_encryption_key")
    if open_signing and open_encryption:
        return _envelope_keys(open_signing, open_encryption)
    return mint_keys


def _mint_bridge_access_candidate(mint: EnvelopeMintInput, mint_keys: envelope.EnvelopeKeys) -> str:
    identity = _envelope_identity(mint)
    refresh_token_value = mint.get("refresh_token")
    grant = envelope.UpstreamTokenGrant(
        access_token=SecretStr(mint.get("access_token") or ""),
        token_type=mint.get("token_type") or "",
        refresh_token=SecretStr(refresh_token_value) if refresh_token_value else None,
        scope=mint.get("scope"),
        expires_in=mint.get("expires_in"),
    )
    sealed = envelope.mint_envelope(identity, grant, mint_keys, _at(mint["mint_now"]))
    if not isinstance(sealed, envelope.SealedEnvelope):
        raise ValueError(f"mint_envelope failed: {sealed}")
    return sealed.token.get_secret_value()


def _mint_bridge_refresh_candidate(
    mint: EnvelopeMintInput, mint_keys: envelope.EnvelopeKeys
) -> str:
    identity = _envelope_identity(mint)
    refresh = envelope.RefreshCredential(
        refresh_token=SecretStr(mint.get("refresh_token") or ""),
        scope=mint.get("scope"),
        expires_in=mint.get("expires_in"),
    )
    sealed = envelope.mint_refresh_envelope(identity, refresh, mint_keys, _at(mint["mint_now"]))
    if not isinstance(sealed, envelope.SealedEnvelope):
        raise ValueError(f"mint_refresh_envelope failed: {sealed}")
    return sealed.token.get_secret_value()


def _session_keys(signing_key: str) -> tokens.SessionKeys:
    return tokens.SessionKeys(signing_key=signing_key)  # type: ignore[arg-type]


def _principal(fields: dict[str, object]) -> tokens.SessionPrincipal:
    return tokens.SessionPrincipal.model_validate(fields)


def _open_session_keys(
    input_value: SessionInput, mint_keys: tokens.SessionKeys
) -> tokens.SessionKeys:
    open_signing = input_value.get("open_signing_key")
    if open_signing:
        return _session_keys(open_signing)
    return mint_keys


def _mint_session_candidate(mint: SessionMintInput, mint_keys: tokens.SessionKeys) -> str:
    principal = _principal(mint["principal"])
    minter = (
        tokens.mint_session_refresh_token
        if mint["kind"] == "session_refresh"
        else tokens.mint_session_token
    )
    minted = minter(principal, mint_keys, _at(mint["mint_now"]))
    if not isinstance(minted, tokens.MintedSessionToken):
        raise ValueError(f"mint_session_token failed: {minted}")
    return minted.token.get_secret_value()


def _scrypt_hex_for_domain(
    master_key: str,
    domain: Literal["envelope_signing", "envelope_encryption", "session_signing"],
) -> str:
    match domain:
        case "envelope_signing":
            keys = bridge_credentials.envelope_keys_from_master_key(master_key)
            return keys.signing_key.get_secret_value()
        case "envelope_encryption":
            keys = bridge_credentials.envelope_keys_from_master_key(master_key)
            return keys.encryption_key.get_secret_value()
        case "session_signing":
            session_keys = session_credentials.session_keys_from_master_key(master_key)
            return session_keys.signing_key.get_secret_value()


def _run_scrypt(input_value: ScryptInput) -> object:
    master_key = input_value["master_key"]
    hex_values = [_scrypt_hex_for_domain(master_key, domain) for domain in input_value["domains"]]
    return {"hex": hex_values}


def _prefix_strip_result(check: PrefixCheck) -> dict[str, bool]:
    shaped = (
        bridge_credentials.is_bridge_envelope_shaped
        if check["family"] == "bridge"
        else session_credentials.is_session_bearer_shaped
    )
    candidate = check["candidate"]
    return {
        "bare": shaped(candidate),
        "bearer": shaped(f"Bearer {candidate}"),
        "mixed_case": shaped(f"BEARER {candidate}"),
    }


def _run_bearer_prefix_strip(input_value: BearerPrefixStripInput) -> object:
    results = [_prefix_strip_result(check) for check in input_value["checks"]]
    return {"results": results}


def _run_bridge_envelope(input_value: EnvelopeInput) -> object:
    mint_keys = _envelope_keys(input_value["signing_key"], input_value["encryption_key"])
    mint = input_value.get("mint")
    if mint is not None:
        candidate = _mint_bridge_access_candidate(mint, mint_keys)
    else:
        candidate = _require_candidate(input_value.get("candidate"))
    open_keys = _open_bridge_keys(input_value, mint_keys)
    result = bridge_credentials.resolve_bridge_envelope(
        candidate, open_keys, _at(input_value["open_now"]), input_value["expected_server_id"]
    )
    if isinstance(result, bridge_credentials.NotBridgeEnvelope):
        return {"tag": "not_bridge_envelope"}
    if isinstance(result, bridge_credentials.BridgeEnvelopeAdmitted):
        return {
            "tag": "admitted",
            "identity": result.identity.model_dump(mode="json"),
            "upstream_authorization": result.upstream_authorization.get_secret_value(),
        }
    return {"tag": "invalid"}


def _run_bridge_refresh(input_value: EnvelopeInput) -> object:
    mint_keys = _envelope_keys(input_value["signing_key"], input_value["encryption_key"])
    mint = input_value.get("mint")
    if mint is not None:
        candidate = _mint_bridge_refresh_candidate(mint, mint_keys)
    else:
        candidate = _require_candidate(input_value.get("candidate"))
    open_keys = _open_bridge_keys(input_value, mint_keys)
    result = bridge_credentials.open_bridge_refresh_envelope(
        candidate, open_keys, _at(input_value["open_now"]), input_value["expected_server_id"]
    )
    if isinstance(result, bridge_credentials.BridgeRefreshOpened):
        return {
            "tag": "opened",
            "identity": result.identity.model_dump(mode="json"),
            "refresh": {
                "refresh_token": result.refresh.refresh_token.get_secret_value(),
                "scope": result.refresh.scope,
                "expires_in": result.refresh.expires_in,
            },
        }
    return {"tag": "invalid"}


def _run_session_bearer(input_value: SessionInput) -> object:
    mint_keys = _session_keys(input_value["signing_key"])
    mint = input_value.get("mint")
    if mint is not None:
        candidate = _mint_session_candidate(mint, mint_keys)
    else:
        candidate = _require_candidate(input_value.get("candidate"))
    open_keys = _open_session_keys(input_value, mint_keys)
    now = _at(input_value["open_now"])
    result = session_credentials.resolve_session_bearer(candidate, open_keys, now)
    if isinstance(result, session_credentials.NotSessionBearer):
        return {"tag": "not_session_bearer"}
    if isinstance(result, session_credentials.SessionBearerAdmitted):
        return {"tag": "admitted", "principal": result.principal.model_dump(mode="json")}
    return {"tag": "invalid", "expired": result.expired}


def _run_session_refresh(input_value: SessionInput) -> object:
    mint_keys = _session_keys(input_value["signing_key"])
    mint = input_value.get("mint")
    if mint is not None:
        candidate = _mint_session_candidate(mint, mint_keys)
    else:
        candidate = _require_candidate(input_value.get("candidate"))
    open_keys = _open_session_keys(input_value, mint_keys)
    expected_client_id = input_value.get("expected_client_id")
    if expected_client_id is None:
        raise ValueError("expected_client_id is required")
    result = session_credentials.open_session_refresh_bearer(
        candidate, open_keys, _at(input_value["open_now"]), expected_client_id
    )
    if isinstance(result, session_credentials.SessionRefreshOpened):
        return {
            "tag": "opened",
            "principal": result.principal.model_dump(mode="json"),
            "jti": result.jti,
        }
    return {"tag": "invalid"}


def _run_signing_config(input_value: SigningConfigInput) -> object:
    general_settings = input_value.get("general_settings")
    environment = input_value.get("environment") or {}
    with patch.dict(os.environ, environment, clear=False):
        resolved = session_credentials.resolve_session_signing_keys(
            input_value["master_key"], general_settings
        )
    if isinstance(resolved, tokens.SessionKeys):
        return {"tag": "symmetric"}
    if isinstance(resolved, tokens.AsymmetricSessionKeys):
        return {
            "tag": "asymmetric",
            "kid": resolved.kid,
            "public_key_pem": tokens.session_public_key_pem(resolved),
        }
    return {"tag": "invalid"}


def run(input_value: Input) -> object:
    match input_value["function"]:
        case "scrypt":
            result = _run_scrypt(input_value)
        case "bearer_prefix_strip":
            result = _run_bearer_prefix_strip(input_value)
        case "bridge_envelope":
            result = _run_bridge_envelope(input_value)
        case "bridge_refresh":
            result = _run_bridge_refresh(input_value)
        case "session_bearer":
            result = _run_session_bearer(input_value)
        case "session_refresh":
            result = _run_session_refresh(input_value)
        case "signing_config":
            result = _run_signing_config(input_value)
    return result
