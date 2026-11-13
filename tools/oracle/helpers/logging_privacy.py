from __future__ import annotations

import json
from collections.abc import Callable
from typing import TypeAlias, cast

import litellm
from litellm import EmbeddingResponse
from litellm.litellm_core_utils import redact_messages
from litellm.litellm_core_utils.initialize_dynamic_callback_params import (
    inherit_message_logging_privacy,
    initialize_standard_callback_dynamic_params,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(value: dict[str, Json]) -> Json:
    previous = litellm.turn_off_message_logging, litellm.redact_user_api_key_info
    try:
        litellm.turn_off_message_logging = value.get("global") is True
        litellm.redact_user_api_key_info = value.get("redact_key_info") is True
        request = value.get("request")
        kwargs: dict[str, object] = dict(request) if isinstance(request, dict) else {}
        with inherit_message_logging_privacy(value.get("inherited") is True):
            dynamic = initialize_standard_callback_dynamic_params(kwargs)
        if value.get("action") == "metadata":
            return cast(Json, dict(dynamic))
        if "dynamic" in value and value["dynamic"] is not None:
            dynamic["turn_off_message_logging"] = cast(bool, value["dynamic"])
        details: dict[str, object] = {
            "litellm_params": {"metadata": {"headers": value.get("headers", {})}},
            "standard_callback_dynamic_params": dynamic,
        }
        private = cast(
            Callable[[dict[str, object]], bool], redact_messages.should_redact_message_logging
        )(details)
        payload = value.get("payload")
        snapshot: Json = payload
        if private and isinstance(payload, dict):
            call_details: dict[str, object] = dict(payload)
            response = payload.get("response")
            result: object = response
            if isinstance(response, dict) and response.get("object") == "list":
                result = EmbeddingResponse.model_validate(response)
            redacted = cast(
                Callable[[dict[str, object], object], object], redact_messages.perform_redaction
            )(call_details, result)
            if isinstance(redacted, EmbeddingResponse):
                redacted = redacted.model_dump(exclude_none=True)
            call_details["response"] = redacted
            snapshot = cast(Json, json.loads(json.dumps(call_details)))
        if value.get("action") == "copies":
            return {"private": snapshot, "public": payload, "source": payload}
        key_info = value.get("key_info")
        if isinstance(key_info, dict):
            key_info = cast(
                Callable[[dict[str, Json]], Json], redact_messages.redact_user_api_key_info
            )(key_info)
        return {"should_redact": private, "snapshot": snapshot, "key_info": key_info}
    finally:
        litellm.turn_off_message_logging, litellm.redact_user_api_key_info = previous
