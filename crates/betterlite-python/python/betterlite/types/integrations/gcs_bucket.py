from collections.abc import Mapping
from typing import Final

from pydantic import JsonValue
from typing_extensions import TypedDict

GCS_DEFAULT_BATCH_SIZE: Final = 2048
# ast-grep-ignore: no-long-names
GCS_DEFAULT_FLUSH_INTERVAL_SECONDS: Final = 20
# ast-grep-ignore: no-long-names
GCS_DEFAULT_USE_BATCHED_LOGGING: Final = True


class GCSLoggingConfig(TypedDict):
    bucket_name: str
    vertex_instance: JsonValue
    path_service_account: str | None


class GCSLogQueueItem(TypedDict):
    payload: Mapping[str, JsonValue]
    kwargs: dict[str, JsonValue]
    response_obj: JsonValue | None
