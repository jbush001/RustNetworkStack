This is a user space TCP/IP network stack.

It's my first real Rust program (other than simple tutorials). I've
been wanting to dig into Rust more deeply with a large project for a while,
and this one seemed like an interesting challenge. I have written a network
stack before: I coded the one on the T-Mobile Sidekick at Danger from scratch
many years ago, but that was in C.

This uses the TUN/TAP driver on Linux to provide a network interface. These
drivers present a virtual network interface (akin to plugging in an Ethernet
card) to the host operating system. We then simulate this stack as a remote
host on that virtual network. This allows this stack to communicate with
programs running on the host. With bridging, it should also allow this stack
to communicate with other machines on the Internet (although I'm not that
far along yet and haven't configured that).

    +----------------------+              +--------------------+
    |       netstack       |              |  Host test program |
    |     (this program)   |              |                    |
    +----------------------+              +--------------------+
    +------------------------------+-----------------------------------+
    |     /dev/net/tun             |                                   |
    |          ^                   |           Network Stack           |
    |          |                   |                                   |
    |          |                   +-----------------------------------+
    |          |                                   ^                   |
    |          |             Host Kernel           |                   |
    |          |                                   |                   |
    |          +-----------------------------------+                   |
    |                                                                  |
    +------------------------------------------------------------------+


## Setup

Install Rust: <https://www.rust-lang.org/tools/install>

Install other utilities:

    sudo apt install tcpdump

## Running

Build

    cargo build

Run unit tests:

    cargo test

The network stack must be run with root privileges, as the TUN device is
not accessible to regular users. It's probably possible to make configuration
changes to avoid that, but I haven't bothered.

    $ sudo ./target/debug/netstack

Now try pinging this from another terminal.

    $ ping 10.0.0.2
    PING 10.0.0.2 (10.0.0.2) 56(84) bytes of data.
    64 bytes from 10.0.0.2: icmp_seq=1 ttl=64 time=2.95 ms
    64 bytes from 10.0.0.2: icmp_seq=2 ttl=64 time=1.40 ms
    64 bytes from 10.0.0.2: icmp_seq=3 ttl=64 time=1.01 ms
    64 bytes from 10.0.0.2: icmp_seq=4 ttl=64 time=2.10 ms

Can also run tcpdump in another window to monitor traffic (this has to be
invoked after netstack is running, otherwise the interface will not exist).

    sudo tcpdump -i tun0 -v

TCP is only a stub, but the beginnings of the 3-way handshake can be
demonstrated by:

    wget http://10.0.0.2

The UDP stack can be tested with the included udp_test.py, which uses the
host's network stack. After starting netstack, run this program to
send UDP packets to it:

    python3 udp_test.py 10.0.0.2
