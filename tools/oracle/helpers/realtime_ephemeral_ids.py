from collections.abc import Callable
from typing import Final, NotRequired, TypedDict

from litellm.proxy.realtime_endpoints.endpoints import (
    _coerce_realtime_session_type as coerce_session_type,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/realtime_endpoints/endpoints.py#L43-L46
    _decode_realtime_token_payload as decode_token_payload,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/realtime_endpoints/endpoints.py#L194-L213
    _encode_realtime_token_payload as encode_token_payload,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/realtime_endpoints/endpoints.py#L170-L192
    _set_transcription_model_on_session as set_transcription_model,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/realtime_endpoints/endpoints.py#L75-L120
    _transcription_model_candidates_from_session as transcription_model_candidates,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/realtime_endpoints/endpoints.py#L54-L72
)
from pydantic import JsonValue

# LiteLLM types the session parameters as a bare dict and the decoded token as dict[str, Any]
CANDIDATES: Final[Callable[[dict[str, JsonValue]], list[str]]] = transcription_model_candidates
SET_MODEL: Final[Callable[[dict[str, JsonValue], str, bool], None]] = set_transcription_model
DECODE_TOKEN: Final[Callable[[str], dict[str, JsonValue] | None]] = decode_token_payload


class TokenInput(TypedDict):
    ephemeral_key: str
    model_id: str
    user_id: str | None
    team_id: str | None
    expires_at: int | None
    session_type: str


class Input(TypedDict):
    function: str
    session_type: NotRequired[str | None]
    session: NotRequired[dict[str, JsonValue]]
    model: NotRequired[str]
    create_if_missing: NotRequired[bool]
    token: NotRequired[TokenInput]
    decrypted: NotRequired[str]


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "session_type":
        return coerce_session_type(input_value.get("session_type"))
    if function == "candidates":
        return list[JsonValue](CANDIDATES(input_value["session"]))
    if function == "set_model":
        session = input_value["session"]
        SET_MODEL(session, input_value["model"], input_value["create_if_missing"])
        return session
    if function == "encode":
        token = input_value["token"]
        return encode_token_payload(
            ephemeral_key=token["ephemeral_key"],
            model_id=token["model_id"],
            user_id=token["user_id"],
            team_id=token["team_id"],
            expires_at=token["expires_at"],
            session_type=token["session_type"],
        )
    if function == "decode":
        return DECODE_TOKEN(input_value["decrypted"])
    raise ValueError(f"unsupported realtime_ephemeral_ids function {function}")
