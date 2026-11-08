# Telinha release notes
## Features

None.

## Improvements

None.

## Fixes

1. During fast motion a shared screen could show black specks and leftovers of old frames until a line swept down the screen and cleaned it. The app dropped some video frames before sending them, and every frame after a dropped one was drawn on the wrong picture; frames are no longer dropped, and a gap now asks for a fresh keyframe.
