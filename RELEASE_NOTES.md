# Telinha release notes
## Features

None.

## Improvements

None.

## Fixes

1. Keyboard shortcuts record and work while Telinha is the window in front. Windows does not send the global keyboard hook the keys of the app's own window, so the window now listens to its own keys as well.
