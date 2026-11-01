import dataclasses
import json
import os
import tempfile
import uuid
from datetime import UTC, datetime
from pathlib import Path
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.integrations.opik import utils as opik_utils
from litellm.integrations.opik.opik_payload_builder.extractors import extract_opik_metadata
from litellm.integrations.opik.opik_payload_builder.payload_builders import build_span_payload
from litellm.types.utils import Usage

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run_uuid() -> Json:
    created = opik_utils.create_uuid7()
    parsed = uuid.UUID(created)
    return {
        "version": parsed.version,
        "variant": parsed.bytes[8] >> 6,
        "canonical": str(parsed) == created,
    }


def run_config(input_value: dict[str, Json]) -> Json:
    environment = cast("dict[str, str]", input_value.get("environment", {}))
    config_file = cast("str | None", input_value.get("config_file"))
    lookups = cast("list[dict[str, Json]]", input_value["lookups"])
    with tempfile.TemporaryDirectory() as home:
        config_path = Path(home) / "opik.config"
        if config_file is None:
            config_path = Path(home) / "missing.config"
        else:
            config_path.write_text(config_file, encoding="utf-8", newline="")
        with (
            patch.object(opik_utils, "CONFIG_FILE_PATH_DEFAULT", str(config_path)),
            patch.dict(os.environ, environment, clear=True),
        ):
            values = [
                opik_utils.get_opik_config_variable(
                    cast("str", lookup["key"]),
                    cast("str | None", lookup.get("user")),
                    cast("str | None", lookup.get("default")),
                )
                for lookup in lookups
            ]
    return cast("Json", {"values": values})


def run_metadata(input_value: dict[str, Json]) -> Json:
    litellm_metadata = cast("dict[str, Json]", input_value.get("litellm_metadata", {}))
    standard_logging_metadata = cast(
        "dict[str, Json]", input_value.get("standard_logging_metadata", {})
    )
    metadata = extract_opik_metadata(litellm_metadata, standard_logging_metadata)
    return {"metadata": cast("Json", json.loads(json.dumps(metadata)))}


def run_payload(input_value: dict[str, Json]) -> Json:
    response = cast("dict[str, Json]", input_value["response"])
    usage_input = cast("dict[str, Json]", response["usage"])
    usage_object = Usage(
        prompt_tokens=cast("int | None", usage_input.get("prompt_tokens")),
        completion_tokens=cast("int | None", usage_input.get("completion_tokens")),
        total_tokens=cast("int | None", usage_input.get("total_tokens")),
    )
    usage = cast(
        "dict[str, int]",
        opik_utils.create_usage_object(usage_object),  # pyright: ignore[reportUnknownMemberType]
    )
    start = datetime(2026, 1, 1, 0, 0, 0, tzinfo=UTC)
    end = datetime(2026, 1, 1, 0, 0, 1, tzinfo=UTC)
    span = build_span_payload(
        project_name=cast("str", input_value["project_name"]),
        trace_id=cast("str", input_value["trace_id"]),
        parent_span_id=cast("str | None", input_value.get("parent_span_id")),
        response_obj=response,
        start_time=start,
        end_time=end,
        input_data=input_value.get("input"),
        output_data=input_value.get("output"),
        metadata=cast("dict[str, object]", input_value.get("metadata", {})),
        tags=cast("list[str]", input_value.get("tags", [])),
        usage=usage,
        provider=cast("str | None", input_value.get("provider")),
        cost=cast("float | None", input_value.get("cost")),
    )
    _traces, spans = opik_utils.get_traces_and_spans_from_payload([dataclasses.asdict(span)])
    result = spans[0]
    for key in ("id", "start_time", "end_time"):
        result.pop(key, None)
    return cast("Json", json.loads(json.dumps(result)))


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    match function:
        case "uuid":
            return run_uuid()
        case "config":
            return run_config(input_value)
        case "metadata":
            return run_metadata(input_value)
        case "payload":
            return run_payload(input_value)
        case _:
            raise ValueError(f"unknown function: {function}")
