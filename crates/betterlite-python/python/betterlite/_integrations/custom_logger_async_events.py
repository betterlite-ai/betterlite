from pydantic import JsonValue

from betterlite._integrations.custom_logger_helpers import add_input_event_fields
from betterlite._integrations.custom_logger_types import AsyncInputEventCallback, VerboseFn


async def run_async_input(
    callback_func: AsyncInputEventCallback, model: str, messages: list[JsonValue],
    kwargs: dict[str, JsonValue], print_verbose: VerboseFn,
) -> None:
    _ = await callback_func(add_input_event_fields(kwargs, model, messages))
    print_verbose(f"Custom Logger - model call details: {kwargs}")
