from typing import Final

from betterlite.types.integrations.custom_logger import StandardCustomLoggerInitParams

AZURE_SENTINEL_MAX_PAYLOAD_SIZE_BYTES: Final = 1_000_000  # ast-grep-ignore: no-long-names


class AzureSentinelInitParams(StandardCustomLoggerInitParams):
    pass
