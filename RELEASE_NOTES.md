# Telinha release notes
## Features

None.

## Improvements

1. All calls and file links of the app share one WebRTC engine instead of starting a new one each time.

## Fixes

1. After someone left a voice room and came back, the people who stayed connected but no longer heard them. When the last call of the app ended, the part that delivers incoming voice stopped for good until the app was restarted.
