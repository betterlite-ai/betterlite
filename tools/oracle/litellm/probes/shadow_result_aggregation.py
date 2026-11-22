from litellm.proxy.management_endpoints.auto_router_endpoints import _pct_of  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/auto_router_endpoints.py#L1331-L1332
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class Pair(Strict):
    numerator: int
    denominator: int


class Output(Strict):
    rates: list[float]


class ShadowResultAggregation(Operation):
    pairs: list[Pair]

    @override
    def run(self) -> Output:
        return Output(rates=[_pct_of(pair.numerator, pair.denominator) for pair in self.pairs])
