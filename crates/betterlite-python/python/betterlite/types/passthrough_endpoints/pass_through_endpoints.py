from enum import StrEnum
from typing import Final

from pydantic import JsonValue
from typing_extensions import TypedDict

# ast-grep-ignore: no-long-names
LITELLM_PASS_THROUGH_CUSTOM_BODY_STATE_KEY: Final = "litellm_" + "pass_through_custom_body"
# ast-grep-ignore: no-long-names
LITELLM_PASS_THROUGH_RAW_BODY_STATE_KEY: Final = "litellm_" + "pass_through_raw_body"
# ast-grep-ignore: no-long-names
LITELLM_PASS_THROUGH_DEPLOYMENT_MODEL_INFO_STATE_KEY: Final = (
    "litellm_" + "pass_through_deployment_model_info"
)
# ast-grep-ignore: no-long-names
LITELLM_PASS_THROUGH_ENDPOINT_MARKER: Final = "__litellm_" + "pass_through_endpoint__"


class EndpointType(StrEnum):
    VERTEX_AI = "vertex-ai"
    GEMINI = "gemini"
    ANTHROPIC = "anthropic"
    OPENAI = "openai"
    TINYFISH = "tinyfish"
    GENERIC = "generic"
    DECISIONS = "decisions"


# ast-grep-ignore: no-long-names
class PassthroughStandardLoggingPayload(TypedDict, total=False):
    url: str
    request_method: str | None
    request_body: dict[str, JsonValue] | None
    response_body: dict[str, JsonValue] | None
    cost_per_request: float | None
