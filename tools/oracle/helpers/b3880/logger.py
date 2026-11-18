from typing import Final

import litellm
from litellm.integrations.prometheus import PrometheusLogger
from prometheus_client import REGISTRY

# The "both" mode adds user_email, and the counter fixes its label set when LOGGER is built.
litellm.prometheus_deployment_and_latency_caller_identity = "both"

LOGGER: Final[PrometheusLogger] = PrometheusLogger()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L269-L272

TOTAL_REQUESTS_METRIC: Final = "litellm_deployment_total_requests"


def emitted_labels() -> dict[str, str] | None:
    for metric in REGISTRY.collect():
        if metric.name != TOTAL_REQUESTS_METRIC:
            continue
        for sample in reversed(metric.samples):
            if sample.name == TOTAL_REQUESTS_METRIC + "_total":
                return dict(sample.labels)
    return None
