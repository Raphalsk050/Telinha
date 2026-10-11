# Telinha release notes
## Features

None.

## Improvements

1. Calls and screen shares connect faster. The app now also uses a fourth message server, which delivered in under half a second where the others took several seconds.
2. A call that drops starts reconnecting about 10 seconds sooner.
3. The preview of a live screen share is lighter and refreshes every 20 seconds, so it no longer holds back people who are trying to connect.

## Fixes

1. A person could stay on "conectando…" forever when one connection message was lost on the way. The app now sends it again after 4 seconds.

Notes:
The faster connection needs both sides on this version.
