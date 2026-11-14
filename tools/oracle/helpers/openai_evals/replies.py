import os

import httpx
import litellm
from pydantic import JsonValue

from .config import CONFIG, REPLY_TRANSFORMS, logging_object
from .probes import Probe, headers_of, params_or_none, required


def reply_probe(probe: Probe) -> JsonValue:
    operation = required(probe.get("reply"), "reply")
    status = probe.get("status")
    if status is None:
        raise ValueError("a reply probe needs status")
    transform = REPLY_TRANSFORMS.get(operation)
    if transform is None:
        raise ValueError(f"unknown reply operation: {operation}")
    response = httpx.Response(
        status, text=required(probe.get("text"), "text"), headers=headers_of(probe)
    )
    try:
        transform(response, logging_object())
    except (ValueError, TypeError, KeyError):
        return {"error": {"status": None}}
    return {"value": None}


def auth_probe(probe: Probe) -> JsonValue:
    env = probe.get("env")
    if env is None:
        env = {}
    litellm.api_key = probe.get("litellm_api_key")
    litellm.openai_key = probe.get("litellm_openai_key")
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(env)
    try:
        headers = CONFIG.validate_environment(headers_of(probe), params_or_none(probe))
    except ValueError:
        return {"error": {"status": None}}
    finally:
        os.environ.clear()
        os.environ.update(saved)
    return {"headers": headers}


def url_probe(probe: Probe) -> JsonValue:
    endpoint = required(probe.get("endpoint"), "endpoint")
    try:
        url = CONFIG.get_complete_url(probe.get("api_base"), endpoint, probe.get("eval_id"))
    except (ValueError, TypeError, KeyError):
        return {"error": {"status": None}}
    return {"url": url}
