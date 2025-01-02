This is a toy user space TCP/IP network stack.

It's my first real Rust program (other than simple tutorials). I've
been wanting to dig into Rust more deeply with a non-trival project
for a while, and this one seemed interesting. I have written a network
stack before many years ago but that was in C.

I've also recently gotten access to Github Copilot and have been using
it heavily on this, which has been an interesting experience. It seems
to struggle with the reasoning about the borrow checker as much as humans,
often generating obviously incorrect code (and, unfortunately, sometimes
subtly incorrect), but it is also surprisingly good at times. I'm dubious
it will replace programmers in the short term as some have predicted, but
I'm intrigued by it as a teaching tool.

This uses the TUN/TAP driver on Linux to provide a network interface. This
driver presents a virtual network interface (akin to plugging in an Ethernet
card) to the host operating system. This program then emulates a remote
host on that virtual network. This allows this stack to communicate with
programs running on the host.

IP bridging (<https://developers.redhat.com/articles/2022/04/06/introduction-linux-bridging-commands-and-features>),
should also allow this stack to communicate with other machines on the
Internet, although I haven't done that.

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

You can also run tcpdump in another window to monitor traffic (this has to be
invoked after netstack is running, otherwise the interface will not exist).

    sudo tcpdump -i tun0 -v

### Ping

    sudo ./target/debug/udp_echo &

Now try pinging:

    $ ping 10.0.0.2
    PING 10.0.0.2 (10.0.0.2) 56(84) bytes of data.
    64 bytes from 10.0.0.2: icmp_seq=1 ttl=64 time=2.95 ms
    64 bytes from 10.0.0.2: icmp_seq=2 ttl=64 time=1.40 ms
    64 bytes from 10.0.0.2: icmp_seq=3 ttl=64 time=1.01 ms
    64 bytes from 10.0.0.2: icmp_seq=4 ttl=64 time=2.10 ms

At the end of this test (As with any), you can kill the server
with kill %1 (or just launch it in another terminal window so
you can use ctrl-C)

### UDP

    sudo ./target/debug/udp_echo &
    python3 scripts/udp_test.py 10.0.0.2

### TCP Download 1

    python3 scripts/chargen_server.py &
    sudo ./target/debug/tcp_bulk_download

Press a key to start the transfer

### TCP Download 2

    python3 -m http.server 3000 &
    sudo ./target/debug/tcp_bulk_download

### TCP Upload

    python3 scripts/sink_server.py 3000 &
    sudo ./target/debug/tcp_bulk_upload


