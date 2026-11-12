import logging
from logging.handlers import BufferingHandler

from litellm._logging import verbose_logger
from litellm.models.credentials import CredentialItem

from b5367.markers import Json
from b5367.views import STATE, UTILS, Input


def run_credentials(input_value: Input) -> Json:
    credentials = input_value.get("credentials")
    no_info: dict[str, str] = {}
    STATE.credential_list = [
        CredentialItem(
            credential_name=credential["credential_name"],
            credential_info=no_info,
            credential_values=credential["credential_values"],
        )
        for credential in ([] if credentials is None else credentials)
    ]
    request = input_value["request"]
    capture = BufferingHandler(capacity=10_000)
    capture.setLevel(logging.WARNING)
    verbose_logger.addHandler(capture)
    verbose_logger.setLevel(logging.WARNING)
    UTILS.load_credentials_from_list(request)
    verbose_logger.removeHandler(capture)
    warning = request["litellm_credential_name"] if capture.buffer else None
    return {"request": request, "warning": warning}


def run_request_flags(input_value: Input) -> Json:
    request = input_value["request"]
    pass_through = input_value.get("is_pass_through")
    return {
        "async": UTILS._is_async_request(request, is_pass_through=pass_through is True),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L2318-L2321
        "streaming": UTILS._is_streaming_request(request, input_value["call_type"]),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L2361-L2364
    }
