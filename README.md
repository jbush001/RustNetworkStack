Install Rust: <https://www.rust-lang.org/tools/install>

Optionally also install:

    sudo apt install tcpdump

Build

    cargo build

This must be run using Sudo

    $ sudo ./target/debug/netstack

Now try pinging this from another terminal.

    $ ping 10.0.0.2
    PING 10.0.0.2 (10.0.0.2) 56(84) bytes of data.
    64 bytes from 10.0.0.2: icmp_seq=1 ttl=64 time=2.95 ms
    64 bytes from 10.0.0.2: icmp_seq=2 ttl=64 time=1.40 ms
    64 bytes from 10.0.0.2: icmp_seq=3 ttl=64 time=1.01 ms
    64 bytes from 10.0.0.2: icmp_seq=4 ttl=64 time=2.10 ms

Can also run tcpdump in another window to monitor traffic (this has to be invoked after netstack is
running, otherwise the interface will not exist)

    sudo tcpdump -i tun0 -v
