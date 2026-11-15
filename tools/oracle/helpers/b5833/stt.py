import litellm
from openai import BadRequestError
from pydantic import JsonValue

from .views import STT
from .wire import STT_MODEL, Call, Object


def stt_value(call: Call) -> JsonValue:
    given_non_default = call.get("non_default")
    non_default: Object = {} if given_non_default is None else dict(given_non_default)
    given_optional = call.get("optional_params")
    optional: Object = {} if given_optional is None else dict(given_optional)
    litellm.drop_params = call.get("global_drop_params") is True
    drop = call.get("drop_params") is True
    try:
        mapped = STT.map_openai_params_stt(non_default, optional, STT_MODEL, drop_params=drop)
    except BadRequestError as error:
        return {"error": {"status": error.status_code}}
    value: Object = {"optional_params": mapped}
    return value
