from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class IoTokenReservations(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "io_token_reservations is not built")
