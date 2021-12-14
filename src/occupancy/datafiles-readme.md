There are a handful of recorded scenes for testing purposes. These are
recording my den with an MLX90641 running at 16 FPS.

# `bincode2text.py`
This is a helper script for converting the bincode files recorded by
r-u-still-there into YAML or TOML files formatted in a way so that it's easier
to view the thermal image for each frame.

# Data files
## `empty-room.bin`
Just an empty room for five minutes, taken just before noon on 2021-12-06.
There is sunlight coming in the window. All lights are off, and the desktop in
scene is sleeping. Nobody walks by the hallway.

## `walk-in.bin`
After 6:30, I walk in and briefly turn on the lights (in view at frame #6003,
done walking by frame #6064). I stand in place for 2:30, then walk out of view
(start walking out at frame #8355, out of view at frame #8398). The recording continues for 1:30.

## `warm-up.bin`
The lights have just been turned on. After 2:10, I walk in, wake up the
computer, and start working, continuing for the rest of the recording (in view
starting at frame #2040, at computer by frame #2102).

## `person-overlap.bin`
The displays are on and stay on for the entire recording. No person is in view
at the beginning of the recording. After 3:00, I walk in and sit in the
chair at the computer (in view at frame #2787, in seat by frame #2871). Another
person is briefly in view after about 10 seconds (between frames #2997 through
#3166). After roughly 6:19, I walk out, leaving the displays on (get up at frame
#8226, in view until frame #8279). The recording continues for roughly 5 minutes
before stopping.
