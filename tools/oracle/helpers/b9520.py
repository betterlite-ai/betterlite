from copy import deepcopy
from datetime import datetime
from typing import Protocol, TypeAlias, cast

import litellm
from litellm import _custom_logger_compatible_callbacks_literal
from litellm.integrations.s3_v2 import S3Logger
from litellm.litellm_core_utils import litellm_logging

# pyright: reportPrivateUsage=false, reportUnknownMemberType=false, reportUnknownVariableType=false
Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

SETUP = litellm_logging.StandardLoggingPayloadSetup


class _LoggerAttributes(Protocol):
    s3_path: str | None
    s3_partition_granularity: object


def run(input_value: dict[str, Json]) -> Json:
    request = deepcopy(input_value)
    logger_name = _optional_text(request.get("cold_storage_custom_logger"))
    s3_v2 = _optional_object(request.get("s3_v2"))
    metadata = _optional_object(request.get("metadata"))
    start_time = _optional_text(request.get("start_time"))
    response_id = _optional_text(request.get("response_id"))
    callbacks = litellm.callbacks
    previous_logger = litellm.cold_storage_custom_logger
    try:
        litellm.cold_storage_custom_logger = cast(
            "_custom_logger_compatible_callbacks_literal | None", logger_name
        )
        if logger_name == "s3_v2" and s3_v2 is not None:
            litellm.callbacks = [*callbacks, _s3_v2_logger(s3_v2)]
        record = SETUP.get_standard_logging_metadata(
            metadata=metadata,
            start_time=None if start_time is None else datetime.fromisoformat(start_time),
            response_id=response_id,
        )
        return {"cold_storage_object_key": record.get("cold_storage_object_key")}
    finally:
        litellm.cold_storage_custom_logger = previous_logger
        litellm.callbacks = callbacks


def _s3_v2_logger(values: dict[str, Json]) -> S3Logger:
    logger = object.__new__(S3Logger)
    attributes = cast("_LoggerAttributes", logger)
    attributes.s3_path = _optional_text(values.get("path"))
    attributes.s3_partition_granularity = values.get("partition_granularity")
    return logger


def _optional_text(value: Json) -> str | None:
    return value if isinstance(value, str) else None


def _optional_object(value: Json) -> dict[str, Json] | None:
    return value if isinstance(value, dict) else None
