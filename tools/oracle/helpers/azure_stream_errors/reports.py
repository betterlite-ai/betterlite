from typing import cast

import httpx
from litellm.llms.azure.common_utils import AzureOpenAIError
from pydantic import JsonValue

from .records import JSON, ErrorView


def header_map(headers: httpx.Headers | dict[str, str] | None) -> JsonValue:
    if headers is None:
        return None
    return dict(headers.items())


def azure_error_view(error: AzureOpenAIError) -> JsonValue:
    view = cast(ErrorView, cast(object, error))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46-L63
    return {
        "class": type(error).__name__,
        "status_present": True,
        "status": view.status_code,
        "message_present": True,
        "message": view.message,
        "headers": header_map(view.headers),
        "body": JSON.validate_python(view.body),
    }
