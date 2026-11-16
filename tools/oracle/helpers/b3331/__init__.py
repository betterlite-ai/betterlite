from .inputs import Input, Output
from .operations import (
    async_content_fallback,
    async_status_fallback,
    complete_url,
    error_class,
    prefetch_default,
    unsupported_request,
    unsupported_response,
)
from .subject import SUBJECT


def run(input_value: Input) -> Output:
    result: Output = {"value": None}
    match input_value["function"]:
        case "multipart_default":
            result = {"value": SUBJECT.use_multipart_form_data()}
        case "prefetch_default":
            result = prefetch_default()
        case "complete_url":
            result = complete_url(input_value)
        case "async_content_fallback":
            result = async_content_fallback(input_value)
        case "async_status_fallback":
            result = async_status_fallback(input_value)
        case "unsupported_request":
            result = unsupported_request(input_value)
        case "unsupported_response":
            result = unsupported_response(input_value)
        case "error_class":
            result = error_class(input_value)
    return result
