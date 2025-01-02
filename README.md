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

The UDP stack can be tested with the included udp_test.py, which uses the
host's network stack. After starting netstack, run this program to
send UDP packets to it:

    python3 udp_test.py 10.0.0.2

To test TCP continuous transfer, before launching app (need to uncomment
test_tcp_download in main.rs)

    python3 chargen_server.py

Then launch the app as above and hit any key to begin the download.

To test the full socket lifecycle, instead:

    python3 -m http.server 3000

To test bulk upload, uncomment test_tcp_upload in main.rs and start this first:

    python3 sink_server 3000

