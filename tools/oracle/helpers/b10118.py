from datetime import datetime, tzinfo
from typing import ClassVar, Literal, NotRequired, Self, TypeAlias, TypedDict, cast

import jwt
import litellm
from jwt.exceptions import InvalidKeyError
from jwt.types import Options
from litellm.proxy.auth import login_utils
from litellm.proxy.management_endpoints.ui_sso import (
    get_disabled_non_admin_personal_key_creation,
)
from litellm.types.proxy.ui_sso import ReturnedUITokenObject
from litellm.types.utils import StandardKeyGenerationConfig
from pydantic import JsonValue
from typing_extensions import override


class EncodeCall(TypedDict):
    function: Literal["encode"]
    token: ReturnedUITokenObject
    master_key: str
    duration: str
    now: str


class FlagCall(TypedDict):
    function: Literal["personal_key_flag"]
    key_generation_settings: NotRequired[StandardKeyGenerationConfig | None]


Call: TypeAlias = EncodeCall | FlagCall


class FixedClock(datetime):
    moment: ClassVar[datetime | None] = None

    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        moment = cls.moment
        if moment is None:
            raise RuntimeError("the clock is not fixed")
        return cls.fromtimestamp(moment.timestamp(), tz)


def decode(token: str, key: str) -> JsonValue:
    opts: Options = {"verify_exp": False}
    header = cast("JsonValue", jwt.get_unverified_header(token))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/login_utils.py#L488-L498
    claims = cast("JsonValue", jwt.decode(token, key, algorithms=["HS256"], options=opts))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/login_utils.py#L488-L498
    return {"header": header, "claims": claims}


def encode(call: EncodeCall) -> JsonValue:
    FixedClock.moment = datetime.fromisoformat(call["now"])
    login_utils.datetime = FixedClock  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/login_utils.py#L478-L498
    login_utils.LITELLM_UI_SESSION_DURATION = call["duration"]  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L1761
    try:
        token = login_utils.encode_ui_session_jwt(call["token"], call["master_key"])
    except (ValueError, InvalidKeyError) as error:
        return type(error).__name__
    return decode(token, call["master_key"])


def personal_key_flag(call: FlagCall) -> JsonValue:
    litellm.key_generation_settings = call.get("key_generation_settings")
    return get_disabled_non_admin_personal_key_creation()  # type: ignore[misc, no-any-return, no-untyped-call]  # pyright: ignore[reportAny, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L1744-L1750


def run(input_value: Call) -> JsonValue:
    if input_value["function"] == "encode":
        return encode(input_value)
    return personal_key_flag(input_value)
