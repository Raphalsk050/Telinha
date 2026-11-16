# Telinha release notes
## Features

None.

## Improvements

None.

## Fixes

1. A call could stay on "conectando…" forever when the other person had Telinha minimized: the minimized app stopped taking in who joined or left and kept calling an old copy of the person. It now keeps up while minimized.
2. Restarting the app left an old copy of the person in the member list and in the call for others. The new copy now replaces the old one at once.
