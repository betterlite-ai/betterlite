import io
from contextlib import redirect_stdout

import litellm
from pydantic import JsonValue

from .contracts import CONFIG, JSON, Input, JsonObject
from .values import (
    auth_value,
    fields_value,
    map_value,
    provider_value,
    request_value,
    response_value,
    url_value,
)

litellm.suppress_debug_info = True


def run(values: Input) -> JsonValue:
    answer: JsonObject = {}
    with redirect_stdout(io.StringIO()):
        litellm.model_cost = values["model_cost"]
        litellm.open_ai_chat_completion_models = set(values["chat_models"])
        litellm.open_ai_text_completion_models = set(values["text_models"])
        if "urls" in values:
            answer["urls"] = [url_value(call) for call in values["urls"]]
        if "supported" in values:
            answer["supported"] = [
                JSON.validate_python(CONFIG.get_supported_openai_params(model))
                for model in values["supported"]
            ]
        if "maps" in values:
            answer["maps"] = [map_value(call) for call in values["maps"]]
        if "requests" in values:
            answer["requests"] = [request_value(call) for call in values["requests"]]
        if "responses" in values:
            answer["responses"] = [response_value(call) for call in values["responses"]]
        if "providers" in values:
            answer["providers"] = [provider_value(call) for call in values["providers"]]
        if "auths" in values:
            answer["auths"] = [auth_value(call) for call in values["auths"]]
        if values.get("fields") is True:
            answer["fields"] = fields_value()
    return answer
