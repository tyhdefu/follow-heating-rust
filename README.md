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

## Cross Compilation via Github Actions (Easy) ##
- Push your code to github (`git push`)
- Open the Github Actions tab of the project: https://github.com/tyhdefu/follow-heating-rust/actions
- Click on the latest "workflow run"
- At the bottom, in the artifacts section, there should be a file `follow_heating_pi_0w`, click to download
- Unzip the download, giving you a binary `follow_heating`

## Cross Compilation for raspberry pi zero W ##
- `cargo install cross`
- `sudo apt-get install podman` (Can also use docker)
- cross build --release --target=arm-unknown-linux-gnueabihf

## Deployment
- Copy the binary to the pi: `scp follow_heating pi@heatingpi:` to put it in the home folder
- Make a backup of the existing binary in `/home/pi/heating/follow_heating_rust`
- Delete the old binary / move it somewhere else
- Copy the new binary into /home/pi/heating/follow_heating_rust
- Restart the heating with `systemctl --user restart follow_heating`
- Check `./watch` and `systemctl --user status follow_heating` to see if its working
