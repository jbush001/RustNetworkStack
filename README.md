This is a toy user space TCP/IP network stack.

It's my first real Rust program (other than simple tutorials). I've
been wanting to dig into Rust more deeply with a non-trival project
for a while, and this one seemed interesting. I have written a network
stack before many years ago but that was in C.

I've also recently gotten access to Github Copilot and have been using it
quite a bit on this, which has been an interesting experience. It seems to
struggle with the reasoning about the borrow checker as much as humans, often
generating obviously incorrect code (and, unfortunately, sometimes subtly
incorrect). Rust's ownership and borrowing semantics require building an
internal mental model and reasoning about it, and it's still unclear if
current LLMs are capable of this. Also, I'd guess these models probably have
much less Rust in their training data. I'm dubious LLMs will replace
programmers in the short term as some have predicted.

This uses the TUN driver on Linux to provide acccess to a network
interface for testing <https://docs.kernel.org/networking/tuntap.html>.
This driver presents a virtual network interface (looking roughly like an
Ethernet card) to the host operating system. This program then emulates a
remote host on that virtual network. This allows this stack to communicate
with programs running on the host.

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

IP bridging (<https://developers.redhat.com/articles/2022/04/06/introduction-linux-bridging-commands-and-features>),
should also allow this stack to communicate with other machines on the
Internet, although I haven't done that.

## Setup

Install Rust: <https://www.rust-lang.org/tools/install>

Install other utilities:

    sudo apt install -y tcpdump gnuplot

## Running

Build

    cargo build

Run unit tests:

    cargo test

Run benchmark:

    cargo bench

The network stack must be run with root privileges, as the TUN device is
not accessible to regular users. It's probably possible to make configuration
changes to avoid that, but I haven't done that.

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

Or IPv6

    $ ping fe80::2%tun0

At the end of this test (As with any), you can kill the server
with kill %1 (or just launch it in another terminal window so
you can use ctrl-C)

### UDP

    sudo ./target/debug/udp_echo &
    python3 scripts/udp_test.py 10.0.0.2

To test IPv6:

    sudo ./target/debug/udp_echo v6 &
    python3 scripts/udp_test.py fe80::2 v6

### TCP Download 1

    python3 scripts/chargen_server.py &
    sudo ./target/debug/tcp_bulk_download

Press a key to start the transfer

To test IPv6:

    python3 scripts/chargen_server.py v6 &
    sudo ./target/debug/tcp_bulk_download v6

### TCP Download 2

    python3 -m http.server 3000 &
    sudo ./target/debug/tcp_bulk_download

(http.server doesn't support IPv6)

### TCP Upload

    python3 scripts/sink_server.py 3000 &
    sudo ./target/debug/tcp_bulk_upload

(add v6 param as above)

