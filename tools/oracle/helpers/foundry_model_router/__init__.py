import io
from contextlib import redirect_stdout

import litellm
from litellm.llms.azure_ai.common_utils import AzureFoundryModelInfo
from pydantic import JsonValue

from .contracts import Input, JsonObject
from .values import (
    catalog_value,
    property_value,
    reader_value,
    response_value,
    strip_value,
    validate_value,
    version_value,
)

litellm.suppress_debug_info = True


def run(values: Input) -> JsonValue:
    answer: JsonObject = {}
    with redirect_stdout(io.StringIO()):
        if "strips" in values:
            answer["strips"] = [strip_value(call) for call in values["strips"]]
        if "responses" in values:
            answer["responses"] = [response_value(call) for call in values["responses"]]
        if "readers" in values:
            answer["readers"] = [reader_value(call) for call in values["readers"]]
        if "catalog" in values:
            answer["catalog"] = [catalog_value(call) for call in values["catalog"]]
        if "token_models" in values:
            answer["token_models"] = [
                AzureFoundryModelInfo(model=model).get_token_counter() is not None
                for model in values["token_models"]
            ]
        if "versions" in values:
            answer["versions"] = [version_value(call) for call in values["versions"]]
        if "properties" in values:
            answer["properties"] = [property_value(call) for call in values["properties"]]
        if values.get("validate") is True:
            answer["validate"] = validate_value()
    return answer
