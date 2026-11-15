import os

import litellm
from openai import BadRequestError
from pydantic import JsonValue

from .capabilities import LookupFailureError
from .views import CHAT
from .wire import MODEL, Call, Object


def credential(call: Call) -> JsonValue:
    os.environ.clear()
    base = call.get("environment_base")
    if base is not None:
        os.environ["GROQ_API_BASE"] = base
    key = call.get("environment_key")
    if key is not None:
        os.environ["GROQ_API_KEY"] = key
    resolved: tuple[str, str, str | None, str | None] = litellm.get_llm_provider(
        model=MODEL,
        api_base=call.get("api_base"),
        api_key=call.get("api_key"),
    )
    return {"api_base": resolved[3], "api_key": resolved[2]}


def supported_value(call: Call) -> JsonValue:
    try:
        params: list[JsonValue] = [*CHAT.get_supported_openai_params(call["model"])]
    except LookupFailureError:
        return {"error": {"status": None}}
    return {"params": params}


def messages_value(call: Call) -> JsonValue:
    body = CHAT.transform_request(call["model"], call["messages"], {}, {}, {})
    return {"messages": body.get("messages")}


def options_value(call: Call) -> JsonValue:
    given_non_default = call.get("non_default")
    non_default: Object = {} if given_non_default is None else dict(given_non_default)
    given_optional = call.get("optional_params")
    optional: Object = {} if given_optional is None else dict(given_optional)
    try:
        mapped = CHAT.map_openai_params(non_default, optional, call["model"], drop_params=False)
    except BadRequestError as error:
        return {"error": {"status": error.status_code}}
    except LookupFailureError:
        return {"error": {"status": None}}
    value: Object = {"optional_params": mapped, "non_default": non_default}
    return value
