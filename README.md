# Heating Controller #
This rust program is ran on a Raspberry pi, connected to various sensors and mounted on a gpio relay board in order
to allow it to control our heat pump and heating.

It has extensive unit testing to ensure correctness and the GitHub Pipelines tests and cross compiles it for the raspberry pi
to ensure every commit passes.

The current implementation has a few defined states, clearly showing what it is doing and thinking at the current time.
Having only a few defined states allows us to be more sure about the correctness of each state, and know what transitions
are posssible.

The program is highly configurable, allowing it to account for cheaper electricity at certain times, times when
the heat pump is more efficient (i.e warmer temperatures / midday).

It connects to [Our wiser hub](https://wiser.draytoncontrols.co.uk/) so it knows when heating is needed.

### History ###
Originally the program was written in Python, but it was changed to rust so that it could be more effectively tested
and reduce bugs.

Rust version started: 28th October 2021.

## Cross Compilation for raspberry pi zero W ##
- `cargo install cross`
- `sudo apt-get install podman` (Can also use docker)
- cross build --release --target=arm-unknown-linux-gnueabihf
