from collections.abc import Mapping

from betterlite._endpoints.payload import CallValue


# LiteLLM reads the model first, so a missing one raises at the call, not inside the task
def model_from_arguments(
    args: tuple[CallValue, ...],
    kwargs: Mapping[str, CallValue],
) -> CallValue:
    if len(args) > 0:
        return args[0]
    return kwargs["model"]
