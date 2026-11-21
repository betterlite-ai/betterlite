import litellm
from pydantic import JsonValue

from azure_passthrough import run as passthrough_run

from .contracts import FIELDS, ROWS, PassthroughCase

litellm.suppress_debug_info = True


def passthrough(case: PassthroughCase) -> JsonValue:
    inference: dict[str, JsonValue] = {
        "function": "body_model_inference_endpoint",
        "inference_steps": case["streaming_steps"],
    }
    reported = ROWS.validate_python(
        FIELDS.validate_python(passthrough_run(inference))["steps"]
    )
    streaming: list[JsonValue] = [
        FIELDS.validate_python(row)["streaming"] for row in reported
    ]
    removal: dict[str, JsonValue] = {
        "function": "api_version_removal",
        "removals": [],
        "url_steps": case["url_steps"],
    }
    urls = FIELDS.validate_python(passthrough_run(removal))["steps"]
    return {"streaming": streaming, "urls": urls}
