from collections.abc import Callable
from typing import Final, NotRequired, TypedDict, cast

from litellm.proxy.realtime_endpoints import endpoints as realtime_endpoints
from litellm.proxy.realtime_endpoints.endpoints import (
    _coerce_realtime_session_type as coerce_session_type,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/realtime_endpoints/endpoints.py#L43-L46
)
from litellm.proxy.realtime_endpoints.endpoints import (
    _decode_realtime_token_payload as decode_token_payload,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/realtime_endpoints/endpoints.py#L194-L213
)
from litellm.proxy.realtime_endpoints.endpoints import (
    _encode_realtime_token_payload as encode_token_payload,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/realtime_endpoints/endpoints.py#L170-L192
)
from pydantic import JsonValue, TypeAdapter

SESSION = TypeAdapter(dict[str, JsonValue])
TEXT = TypeAdapter(str)
BOOLEAN = TypeAdapter(bool)

# LiteLLM types both session parameters as a bare dict, so the callables are named here
CANDIDATES: Final = cast(Callable[[dict[str, JsonValue]], list[str]], realtime_endpoints._transcription_model_candidates_from_session)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/realtime_endpoints/endpoints.py#L54-L72
SET_MODEL: Final = cast(Callable[[dict[str, JsonValue], str, bool], None], realtime_endpoints._set_transcription_model_on_session)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/realtime_endpoints/endpoints.py#L75-L120
DECODE_TOKEN: Final[Callable[[str], dict[str, JsonValue] | None]] = decode_token_payload


class TokenInput(TypedDict):
    ephemeral_key: str
    model_id: str
    user_id: str | None
    team_id: str | None
    expires_at: int | None
    session_type: str


TOKEN = TypeAdapter(TokenInput)


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
        return list[JsonValue](CANDIDATES(SESSION.validate_python(input_value.get("session"))))
    if function == "set_model":
        session = SESSION.validate_python(input_value.get("session"))
        SET_MODEL(
            session,
            TEXT.validate_python(input_value.get("model")),
            BOOLEAN.validate_python(input_value.get("create_if_missing")),
        )
        return session
    if function == "encode":
        token = TOKEN.validate_python(input_value.get("token"))
        return encode_token_payload(
            ephemeral_key=token["ephemeral_key"],
            model_id=token["model_id"],
            user_id=token["user_id"],
            team_id=token["team_id"],
            expires_at=token["expires_at"],
            session_type=token["session_type"],
        )
    if function == "decode":
        return DECODE_TOKEN(TEXT.validate_python(input_value.get("decrypted")))
    raise ValueError(f"unsupported realtime_ephemeral_ids function {function}")
