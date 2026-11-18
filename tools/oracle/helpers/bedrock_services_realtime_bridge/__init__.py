import asyncio
import uuid

import fastuuid
import litellm
from pydantic import JsonValue

from bedrock_services_realtime_session import FixedFast, FixedStd

from .fakes import install_sdk
from .models import Input, JsonAdapter
from .pumps import bridge_run

litellm.suppress_debug_info = True


def run(input_value: Input) -> JsonValue:
    uuid.uuid4 = FixedStd(input_value["name_ids"])
    fastuuid.uuid4 = FixedFast(input_value["event_ids"])
    install_sdk()
    results: list[JsonValue] = [
        JsonAdapter.validate_python(asyncio.run(bridge_run(run_value, input_value["model"])))
        for run_value in input_value["runs"]
    ]
    return {"runs": results}
