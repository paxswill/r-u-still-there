# Set up from scratch on a Raspberry Pi

This is a quick walkthrough of the process of setting up a Raspberry Pi with
r-u-still-there using a Sparkfun GridEYE module. I'm using a Sparkfun Qwiic
breakout board as well. You should already know how to connect to your Raspberry
Pi over SSH, and be somewhat comfortable with creating and editing text files
over a command line before following this guide. If you don't know how to do
those, but want to learn, Ubuntu has an
[introduction to the Linux command line][ubuntu-cli-intro], and the Raspberry Pi
Foundation has some documentation on how to [connect over SSH][rpi-ssh].

[ubuntu-cli-intro]: https://ubuntu.com/tutorials/command-line-for-beginners
[rpi-ssh]: https://www.raspberrypi.org/documentation/remote-access/ssh/README.md

If you're already familiar with the process of setting up a Raspberry Pi for
headless access, step 5 is where the interesting stuff starts.

[raspbian]: https://www.raspberrypi.org/software/operating-systems/#raspberry-pi-os-32-bit
[rpi-wifi]: https://www.raspberrypi.org/documentation/configuration/wireless/headless.md
[rpi-ip]: https://www.raspberrypi.org/documentation/remote-access/ip-address.md
[raspi-config]: https://www.raspberrypi.org/documentation/configuration/raspi-config.md

1. Download the [Raspberry Pi OS Lite][raspbian] image and image a microSD card
   with it. After the imaging process is complete, open up the boot partition
   (you may need to unplug the microSD card and then plug it back in to get it
   to show up.)

2. I'm going to be connecting the Raspberry Pi over Wifi, and to make the
   process simpler I'm going to use the [headless configuration][rpi-wifi]
   process. So first I create a file in the boot partition named
   `wpa_supplicant.conf` as documented on that page.
   ```
   ctrl_interface=DIR=/var/run/wpa_supplicant GROUP=netdev
   country=<Insert 2 letter ISO 3166-1 country code here>
   update_config=1
   
   network={
    ssid="<Name of your wireless LAN>"
    psk="<Password for your wireless LAN>"
   }
   ```
   Then I create a file called `ssh` on the boot partition so I can log in to
   the Raspberry Pi over SSH.

3. Close your text editor and eject the microSD card. Now plug the microSD card
   in to the Raspberry Pi, and connect the GridEYE. Once everything's hooked up,
   connect the Raspberry Pi to power, and wait a few minutes for it to start up
   fully and appear on your network. Once it's up, I
   [find the IP address][rpi-ip] for the next step.

4. I log in to the Raspberry Pi over SSH using the IP address from the previous
   step with the username `pi` and the password `raspberry`.
   ```
   ssh pi@172.17.40.53
   ```
   
   Then I change the password of the `pi` user using the `passwd` command:
   ```
   pi@raspberrypi:~ $ passwd pi
   Changing password for pi.
   Current password:
   New password:
   Retype new password:
   passwd: password updated successfully
   ```

   Now let's update the software already installed:
   ```shell
   sudo apt update
   sudo apt upgrade
   ```

   Finally, I'm going to enable the I2C interface using
   [`raspi-config`][raspi-config], then reboot:
   ```shell
   sudo raspi-config
   # Navigate to "Interfacing Options", then "I2C", then enable the I2C interface.
   sudo systemctl reboot
   ```

5. Now I'm going to add an APT repository so I can install r-u-still-there and
   also easily update it along with the rest of the software on the Raspberry
   Pi.

   > A quick aside here: Adding an APT repository *requires* that you trust
   > whomever is running it, as it is possible sneak malicious packages on to
   > your system through it. I'm not going to do that, but it is something to be
   > aware of. As a guard against this possibility, this guide also adds a
   > configuration file that allows only r-u-still-there packages to be
   > installed from this repository, and to only use them as a last resort. This
   > also means that if Debian (or Ubuntu, or Raspberry Pi OS) ever distribute
   > r-u-still-there themselves, their version will be installed instead.
   >
   > Finally, if you don't want to add the repository, you can also just
   > download the `.deb` file and install it with `dpkg -i`. There is a
   > dependency on `i2c-tools`, so you might need to install that yourself as
   > well (it should be available through the included repositories). If you go
   > this route, r-u-still-there will not update through APT , so you'll need to
   > apply updates yourself (which is exactly what some people want!). Just pick
   > up at the next step.

   First I install some necessary packages:
   ```
   sudo apt update
   sudo apt install curl ca-certificates apt-transport-https gpg
   ```

   Then I download the signing key used for my package repository:
   ```
   curl -fLsS https://deb.paxswill.com/apt-key.gpg | sudo gpg --dearmor -o /usr/share/keyrings/paxswill.gpg
   ```

   Next I create two files. The first tells APT (the package manager) about my
   package repository. The next is a configuration file for APT so that it
   installs *only* r-u-still-there from that repository.
   ```
   echo "deb [arch=armhf signed-by=/usr/share/keyrings/paxswill.gpg] https://deb.paxswill.com buster main" | sudo tee /etc/apt/sources.list.d/paxswill.list

   printf "Package: *\nPin: origin deb.paxswill.com\nPin-Priority: -1\n\n" | sudo tee /etc/apt/preferences.d/paxswill

   for PKG in r-u-still-there{,-v6,-v7}; do printf "Package: %s\nPin: origin deb.paxswill.com\nPin-Priority: 2\n\n" "$PKG"; done | sudo tee --append /etc/apt/preferences.d/paxswill
   ```
   Now we update apt and install r-u-still-there. Do note that there are two
   packages for 32-bit ARM processors. If you're using a Raspberry Pi 1 or Zero,
   use `r-u-still-there-v6`. Otherwise, use `r-u-still-there-v7`. The `-v6`
   version will still work on the newer Raspberry Pis, but the `-v7` version is
   noticeably faster. If you're using the beta (at least as of July 2021) 64-bit
   version of Raspberry Pi OS, the package name is just `r-u-still-there`.
   ```
   sudo apt update
   sudo apt install r-u-still-there-v6
   ```
   It's expected for there to be a few extra packages to install, mostly related
   to I2C.

6. Now that r-u-still-there is installed, we need to configure it. Open up
   `/etc/r-u-still-there/config.toml` in your preferred text editor (if you
   don't have a favorite, nano will probably work well for you)
   ```
   sudo nano /etc/r-u-still-there/config.toml
   ```

   The config file has comments documenting each configuration value. For this
   example, I'm going to use these settings (with extra explanation in
   comments):
   ```toml
   [camera]
   kind = "grideye"
   # On Raspberry Pis running Raspberry Pi OS, this will almost always be 1
   bus = 1
   address = 0x69
   frame_rate = 10

   [streams]
   # The default for this is a secure, but somewhat useless, "127.0.0.1".
   # Using "0.0.0.0" means r-u-still-there will listen on any network interface 
   # it sees.
   address = "0.0.0.0"
   [streams.mjpeg]
   enabled = true
   # The Raspberry Pi Zero can't quite hit a full 10FPS, but can easily hit 7.
   frame_rate_limit = 7

   [render]
   colors = "turbo"
   upper_limit = { fahrenheit = 90 }
   lower_limit = { fahrenheit = 70 }
   grid_size = 40
   units = "fahrenheit"

   [tracker]
   # Any camera pixel over this temperature will count as a human.
   threshold = { fahrenheit = 80 }

   [mqtt]
   name = "Den Entertainment"
   # Use the IP address (or the hostname if you've set that up) of your MQTT  
   # broker here. If you're using plain MQTT, use 'mqtt' as the scheme, but if 
   # you're using MQTT over TLS, use 'mqtts'.
   server = "mqtts://mqtt.example.com"
   # If your MQTT broker isn't set up to use authentication, you can leave 
   # the 'username' part out.
   username = "r-u-still-there"
   [mqtt.home_assistant]
   enabled = true
   # Home Assistant should be converting whatever units r-u-still-there sends it
   # to what you've set as the preferred unit in Home Assistant.
   unit = "celsius"
   ```
   In this example, I'm going to use a systemd drop-in to slightly customize the
   default service file:
   ```shell
   (umask 077; sudo systemctl edit r-u-still-there.service)
   ```
   with contents similar to this:
   ```ini
   [Service]
   Environment="RUSTILLTHERE_MQTT_PASSWORD=SooperSeekrit"
   ```

7. Now that we have the config file set up, let's test it:
   ```shell
   r-u-still-there
   ```
   If your user doesn't have access to the I2C device, you'll see an error
   message about permission denied. You can either fix that error (by adding
   yourself to the `i2c` user group), or temporarily run r-u-still-there with
   sudo.

   If r-u-still-there encounters an error, the error message *should* tell you
   why, but if you want more info you can increase the amount of logging like
   so:
   ```shell
   RUST_LOG=debug r-u-still-there
   ```

8. After we've confirmed that the configuration is good, let's start the
   service:
   ```shell
   sudo systemctl start r-u-still-there
   sudo systemctl status r-u-still-there
   ```
   That first command starts the service (it should be stopped as it wasn't
   configured before), and the second shows the status. If the service is
   failing, the logs should be in the lower part of that screen.

And that's it! In Home Assistant (assuming you've already set up MQTT in Home
Assistant) you should see a new device exposing the ambient temperature, a
boolean sensor for whether the space is occupied or not, and a count of how many
people the camera sees. You can also navigate to `http://<raspberry pi
IP>:9000/mjpeg` to see a live video stream from the camera.
