# r-u-still-there

A home automation sensor for human presence using thermal cameras.

The most common way to detect if a space is occupied or not is with a 
[PIR sensor][wiki-pir], but this comes with the downside that it doesn't detect
stationary people. r-u-still-there is an application that can be installed on
an embedded Linux system along with a [thermal camera](#cameras).
r-u-still-there will then notify your home automation system when it detects a
person, and when that person leaves its view. In other words, you can use it to
sense if a room is occupied, even if the people in that room are keeping still
(like when they're watching a movie).

[wiki-pir]: https://en.wikipedia.org/wiki/Passive_infrared_sensor

## Features

* Efficient usage of CPU and network.
* Messages sent through an [MQTT][wiki-mqtt] broker, allowing use by multiple
  different home automation systems.
* Easy integration with [Home Assistant][hass].
* MJPEG stream available so you can feel like the Predator.

[wiki-mqtt]: https://en.wikipedia.org/wiki/MQTT
[hass]: https://www.home-assistant.io/

## Hardware

r-u-still-there has been tested and used on a variety of Raspberry Pis from the
low-cost 0 and low-speed 1B+ up through an 8GB 4B. I also use it on BeagleBone
Greens, and it should run on any Linux device that you can connect an
[I²C][wiki-i2c] peripheral to.

[wiki-i2c]: https://en.wikipedia.org/wiki/I%C2%B2C

Performance wise, I recommend using something with at least an ARMv7 CPU if you
can. The ARMv6 CPU on the Raspberry Pi 0 and 1 works, but it can struggle to
render the image stream at higher frame rates and larger sizes. The SIMD
instructions and faster speeds on newer processors makes a noticable difference.

### Cameras

Currently (as of v0.1.0) just the Panasonic GridEYE is supported, but I'm
planning on adding support for Melexis MLX90640 and MLX90641 as well as
Omron D6T sensors in the future.

## Installation

**TODO**: Actually cut a release. As it stands, this section is describing the
*planned* installation process.

You can install just the program from cargo (crate name 'r-u-still-there'). The
preferred process though is to use the .deb packages, either manually downloaded
from the releases on GitHub or from my package repo. In any case, I would
recommend using the version that matches your hardware closest, as each jump in
ARM instruction sets makes a noticeable improvement in performance (even on the
exact same hardware).

### Debian-based Distributions (including Ubuntu, Raspberry Pi OS)

**TODO**: Set up automagic builds, then manually sign with my existing keys.
Provide config that adds my deb repo, but pins it lower (see apt_preferences(5)
for details on pinning packages from an origin lower than default).

A systemd service file is added and started, but your configuration file will
need to be modified with some details before r-u-still-there will start. Edit
`/etc/r-u-still-there/config.toml` in your text editor of choice:

```shell
sudo nano /etc/r-u-still-there/config.toml
```

The configuration file documents each value, with the values uncommented by
default required to be present.

Once you're made your changes and saved them, start the service with `sudo
systemctl start r-u-still-there`. You can check to make sure it's running with
`systemctl status r-u-still-there`. Next to the "Active: " label, it should say,
"active (running)". If it isn't (and it will probably say that it's failed in
this case) the most recent logs are listed below. Use the arrow keys to scroll
through them and see what went wrong. Press the `Q` key to exit the status view
when you're done.

## FAQ
#### Why is the CPU usage is really high?
Drawing the text of the temperatures is fairly CPU intensive at the moment. If
you can disable that (by commenting out the `units` value in the config file),
CPU usage will go down. Another option is to limit the frame rate of the video
stream with the `frame_rate_limit` setting.

Rendering the video stream is the most "expensive" part of r-u-still-there at
the moment as it's all being done on the CPU. If there is no client
connected to the MJPEG stream though, no rendering is done and CPU usage
should drop back down.

#### How do I configure it?
For the Debian packages, the configuration file is located at
`/etc/r-u-still-there/config.toml`. That is also the default location if no
config file is given as a command line argument.

#### How do you connect the cameras?
You need to connect the camera to your device's I²C bus. This varies between
different devices, but here are a few examples for some devices:

* [Raspberry Pi](https://pinout.xyz/pinout/i2c)
* [BeagleBone Black](https://beagleboard.org/Support/bone101/#headers)
* [BeagleBone Green](https://wiki.seeedstudio.com/BeagleBone_Green/#hardware-overview)
  
#### How do I get more detailed logs?
Logging can be configured using the `RUST_LOG` environment variable. Setting
`RUST_LOG=debug` will give pretty verbose logs, but if you want even more,
`trace` is also available.

#### What MQTT brokers can I use?
I use [mosquitto](https://mosquitto.org/), but any MQTT 3 compatible broker that
supports retained messages should work.

#### How far away can the sensor detect a person?
This depends the resolution and field of view of the camera you're using. Higher
resolution and a narrower field of view will result in a the camera being able
to detect a person from farther away. In my experience, a GridEYE can usually
detect me (a moderately tall, average build man) from about 4m (13 ft) away.

#### There's a lot of noise (rapidly changing, but not by much) in the temperatures. How can I get rid of that?
On the GridEYE, setting the camera to 1 FPS will internally de-noise the image.
I'm also planning on adding other methods in the future.

#### This sounds a lot like what [room-assistant][room-assistant] does.

It does! I used room-assistant for a while, and think it's a really cool piece
of software. If you want presence detection using Bluetooth, it's still what
first comes to mind. Over time I encountered a few pain points that I feel
r-u-still-there better addresses:

* room-assistant can be pretty taxing on the CPU, as it's rendering the image
  for every frame, regardless of if there's someone watching it. r-u-still-there
  only renders the image if there's an active client. This results in CPU usage
  of around 2% on a Raspberry Pi Model 1B+, while room-assistant would normally
  be around 50-60% on older versions, and be maxed out on newer versions
  ([v2.12.0][r-a_v2.12] changed to using Javascript libraries for image
  rendering). Even when streaming video though, r-u-still-there is generally
  more performant, with a 1 FPS stream taking roughly 15% CPU usage, and 5 FPS
  taking around 60% CPU.

* r-u-still-there has a few more configuration knobs, such as the size of the
  generated thermal image, the color scheme used in that image, camera frame
  rate, and the units used for generated data.

* Some of my devices have poor WiFi reception, so they slow down the other
  devices on the network when they need to communicate. room-assistant generates
  a lot of network traffic, with a full image being sent over MQTT every second
  in addition to a fair bit of multicast traffic (which can be turned off, but
  is enabled by default). r-u-still-there does not send images over MQTT (there
  may be an option to enable this in the future, but not currently).

* Some cameras offer extra capabilites that room-assistant doesn't expose. For
  example, the GridEYE can be run at 10 FPS at the cost of increased noise in
  the image. Most thermal cameras also have an ambient temperature sensor, which
  is also exposed by r-u-still-there.

[room-assistant]: https://www.room-assistant.io/
[r-a_v2.12]: https://github.com/mKeRix/room-assistant/releases/tag/v2.12.0

All that being said, I'm still very thankful to room-assistant for inspiring me
to create r-u-still-there.

## Development

This repository should just build with `cargo` once checked out from git:

```shell
git clone https://github.com/paxswill/r-u-still-there.git
cd r-u-still-there
cargo build --release
```

You will almost always want to build in release mode, as without the more
extra optimization passes it's unusably slow. Use `cargo check` for syntax
checking (if you're not already using another tool such as [RLS][rls] or
[rust-analyzer][rust-analyzer]).

[rls]: https://github.com/rust-lang/rls
[rust-analyzer]: https://rust-analyzer.github.io/

### Cross-compiling

Building on the target device itself can be very slow, and the device may not
even have enough memory. Cross compilation is pretty easy with Rust, especially
if using the [musl][musl] targets as they are (at least currently) statically
linked. When using the `mozjpeg` optional dependency (the default setting is to
use it) you will also need a linker and C compiler for your target.
[musl-cross-make][musl-cross-make] is a pretty easy way to build and install the
toolchains required. Once you've installed the toolchains, you will need to add
these lines to the `.cargo/config.toml` file (creating it if it doesn't exist):

[musl]: https://musl.libc.org/
[musl-cross-make]: https://github.com/richfelker/musl-cross-make

```toml
[target.arm-unknown-linux-musleabihf]
linker = "arm-linux-musleabihf-gcc"

[target.armv7-unknown-linux-musleabihf]
linker = "armv7-linux-musleabihf-gcc"

[target.aarch64-unknown-linux-musl]
linker = "aarch64-linux-musl-gcc"
```

You also need to provide come extra options to the C compiler for the 32-bit ARM
architectures:

```shell
# ARMv6, for Raspberry Pi 0 and 1
TARGET_CFLAGS="-march=armv6+fp" cargo build --release --target arm-unknown-linux-musleabihf
# ARMv7, for earlier version of the Raspberry Pi 2 and BeagleBones
TARGET_CFLAGS="-march=armv7-a+simd" cargo build --release --target armv7-unknown-linux-musleabihf
# 64-bit ARMv8, for Raspberry Pi Model 4
cargo build --release --target aarch64-unknown-linux-musl
```

Building the Debian package is done using [cargo-deb][cargo-deb]. Once you have
it installed, just replace `build --release` in the commands above with `deb`,
like so:

```shell
TARGET_CFLAGS="-march=armv6+fp" cargo deb --target arm-unknown-linux-musleabihf
```

[cargo-deb]: https://github.com/mmstick/cargo-deb

