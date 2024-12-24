Install Rust: <https://www.rust-lang.org/tools/install>

Build

    cargo build

This must be run using Sudo

    $ sudo ./target/debug/netstack

Now try pinging this from another terminal. This will see 100% loss because
our network stack doesn't respond (yet)

    $ ping 10.0.0.1
    PING 10.0.0.1 (10.0.0.1) 56(84) bytes of data.
    ^C
    --- 10.0.0.1 ping statistics ---
    2 packets transmitted, 0 received, 100% packet loss, time 1025ms

...But, you will see IP packets coming into this program:

    Received packet (84 bytes):
    45 00 00 54 e3 5e 40 00 40 01 8c 08 64 73 5c ce
    0a 00 00 01 08 00 a2 16 7d 4c 00 01 b0 d9 6a 67
    00 00 00 00 f3 87 0b 00 00 00 00 00 10 11 12 13
    14 15 16 17 18 19 1a 1b 1c 1d 1e 1f 20 21 22 23
    24 25 26 27 28 29 2a 2b 2c 2d 2e 2f 30 31 32 33
    34 35 36 37
