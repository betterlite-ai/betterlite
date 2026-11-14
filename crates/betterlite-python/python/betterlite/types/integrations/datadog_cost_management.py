from typing_extensions import TypedDict

from betterlite.types.integrations.custom_logger import StandardCustomLoggerInitParams


# ast-grep-ignore: no-long-names
class DatadogCostManagementInitParams(StandardCustomLoggerInitParams):
    cost_tag_keys: list[str] | None = None


class DatadogFOCUSCostEntry(TypedDict):
    ProviderName: str
    ChargeDescription: str
    ChargePeriodStart: str
    ChargePeriodEnd: str
    BilledCost: float
    BillingCurrency: str
    Tags: dict[str, str] | None
