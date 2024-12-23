Install Rust: <https://www.rust-lang.org/tools/install>

Build

    ./build.sh

This must be run using Sudo

    $ sudo ./main

Now try pinging this from another terminal. This will fail because this device does not support ARP:

    $ ping 10.0.0.1
    PING 10.0.0.1 (10.0.0.1) 56(84) bytes of data.
    From 100.115.92.206 icmp_seq=1 Destination Host Unreachable
    From 100.115.92.206 icmp_seq=2 Destination Host Unreachable
    From 100.115.92.206 icmp_seq=3 Destination Host Unreachable

...But, you will see ICMP packets coming into this program (starting with the ethernet header):

    ff ff ff ff ff ff be 6e 72 7d db 21 08 06 00 01 08 00 06 04 00 01 be 6e 72 7d db 21 64 73 5c ce 00 00 00 00 00 00 0a 00 00 01
    ff ff ff ff ff ff be 6e 72 7d db 21 08 06 00 01 08 00 06 04 00 01 be 6e 72 7d db 21 64 73 5c ce 00 00 00 00 00 00 0a 00 00 01
    ff ff ff ff ff ff be 6e 72 7d db 21 08 06 00 01 08 00 06 04 00 01 be 6e 72 7d db 21 64 73 5c ce 00 00 00 00 00 00 0a 00 00 01


