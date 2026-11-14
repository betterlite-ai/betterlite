from enum import Enum, StrEnum
from typing import Final

from pydantic import JsonValue
from typing_extensions import NotRequired, TypedDict

from betterlite.types.integrations.custom_logger import StandardCustomLoggerInitParams

DD_MAX_BATCH_SIZE: Final = 1000
DD_MAX_PAYLOAD_SIZE_BYTES: Final = 4_000_000


class DataDogStatus(StrEnum):
    INFO = "info"
    WARN = "warning"
    ERROR = "error"


DatadogPayload = TypedDict(
    "DatadogPayload",
    {
        "ddsource": str,
        "ddtags": str,
        "hostname": str,
        "message": str,
        "service": str,
        "status": str,
        "dd.trace_id": NotRequired[str],
        "dd.span_id": NotRequired[str],
    },
    total=False,
)


class DD_ERRORS(Enum):  # noqa: N801  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/integrations/datadog.py#L34
    DATADOG_413_ERROR = (
        "Datadog API Error - Payload too large (batch is above 5MB uncompressed). "
        "If you want this logged either disable request/response logging or set "
        "`DD_BATCH_SIZE=50`"
    )


class DatadogInitParams(StandardCustomLoggerInitParams):
    pass


# ast-grep-ignore: no-long-names
class DatadogProxyFailureHookJsonMessage(TypedDict, total=False):
    exception: str
    error_class: str
    status_code: int | None
    traceback: str
    user_api_key_dict: dict[str, JsonValue]
