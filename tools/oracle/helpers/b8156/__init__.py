import litellm

from .embedding import embedding_cases
from .params import param_cases
from .reply import reply_cases
from .request import request_cases
from .stream import stream_cases
from .support import Input, Json

__all__ = ["litellm", "run"]


def run(input_value: Input) -> Json:
    match input_value["op"]:
        case "params":
            return param_cases(input_value)
        case "request":
            return request_cases(input_value)
        case "reply":
            return reply_cases(input_value)
        case "stream":
            return stream_cases(input_value)
        case "embedding":
            return embedding_cases(input_value)
