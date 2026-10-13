# Telinha release notes
## Features

None.

## Improvements

1. A call that fails to connect tries again through different network ports, instead of repeating the same attempt.
2. The app log now records each call attempt: how long it took, and whether the other side was reached.

## Fixes

1. A person could stay on "conectando…" for minutes, or forever, on routers that handle UDP ports 50000 to 50100 in a special way. Calls and screen shares used fixed ports inside that range. They now use ports between 51200 and 59999, different on each computer.
2. Two computers on the same home network no longer compete for the same ports on the router.

Notes:
The ports are no longer fixed, so a router rule that forwards UDP 50000 to 50039 has no effect anymore and can be removed.
