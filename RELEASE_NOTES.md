# Telinha release notes
## Features

None.

## Improvements

None.

## Fixes

1. Parts of a shared game could keep showing an old frame in blocks while the game itself looked fine. The encoder took parts that Windows did not report as changed as static and coded them at much lower quality; recently changed parts and keyframes now keep full quality.
