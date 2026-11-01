from collections.abc import Callable
from copy import deepcopy
from datetime import datetime
from typing import TypeAlias, cast

import litellm
from litellm.integrations.s3_v2 import S3Logger
from litellm.types.integrations.s3_v2 import s3BatchLoggingElement

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_FIELDS = (
    "bucket_name",
    "region_name",
    "api_version",
    "endpoint_url",
    "path",
    "config",
    "max_concurrent_uploads",
    "max_queue_size",
    "max_retry_age_seconds",
    "max_adaptive_concurrency",
    "use_ssl",
    "use_team_prefix",
    "use_key_prefix",
    "strip_base64_files",
    "use_virtual_hosted_style",
    "drop_on_terminal_error",
    "adaptive_concurrency",
    "batch_file_upload",
    "server_side_encryption",
    "sse_kms_key_id",
)


def run(input_value: dict[str, Json]) -> Json:
    request = deepcopy(input_value)
    logger = object.__new__(S3Logger)
    initialize = cast(Callable[..., None], logger._init_s3_params)  # pyright: ignore[reportPrivateUsage]
    operation = request["operation"]
    if operation == "params":
        litellm.s3_callback_params = cast(dict[str, Json], request.get("global", {}))
        constructor = cast(dict[str, Json], request.get("constructor", {}))
        initialize(params_source=request.get("audit"), **constructor)
        return {field: cast(Json, getattr(logger, f"s3_{field}")) for field in _FIELDS}
    initialize(params_source=request.get("source", {}))
    if operation == "partition":
        output: list[Json] = []
        for raw in cast(list[Json], request["values"]):
            logger.s3_partition_granularity = raw
            output.append(logger.resolve_partition_granularity())
        return output
    litellm.cold_storage_custom_logger = "s3_v2" if request.get("cold_storage") else None
    create = cast(
        Callable[..., s3BatchLoggingElement | None], logger.create_s3_batch_logging_element
    )
    element = create(
        start_time=datetime.fromisoformat(cast(str, request["start_time"])),
        standard_logging_payload=request.get("payload"),
    )
    return {"element": None if element is None else cast(Json, element.model_dump())}
