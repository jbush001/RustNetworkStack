//
// Copyright 2024 Jeff Bush
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//

// This create a device that will appear in the host as a network interface.
// It will set up an IP route so that any packets sent to 10.0.0.2 will
// be routed to this program and readable via the tun_recv function.
// Likewise, any packets sent from this will be received byt he host network
// stack as if they came from a remote machine.
// https://www.kernel.org/doc/Documentation/networking/tuntap.txt

#include <fcntl.h>
#include <linux/if.h>
#include <linux/if_tun.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>

static int tun_fd;

int tun_init() {
    tun_fd = open("/dev/net/tun", O_RDWR);
    if (tun_fd < 0 ) {
        printf("Error %d opening TUN device\n", tun_fd);
        return -1;
    }

    struct ifreq ifr;
    memset(&ifr, 0, sizeof(ifr));
    ifr.ifr_flags = IFF_TUN | IFF_NO_PI;
    int err = ioctl(tun_fd, TUNSETIFF, (void*) &ifr);
    if (err < 0) {
        printf("TUNSETIFF error: %d\n", err);
        close(tun_fd);
        return -1;
    }

    char command_line[256];

    // Indicate the interface is up.
    sprintf(command_line, "ip link set dev %s up", ifr.ifr_name);
    system(command_line);

    // Configure so anything sent from the host to the 10.0.0.x subnet gets
    // routed to our TUN driver. Our address is hardcoded in netif.rs as
    // 10.0.0.2.
    sprintf(command_line, "ip route add dev %s 10.0.0.0/24", ifr.ifr_name);
    system(command_line);

    // Address of the host on the virtual network.
    // This is the address our stack will see packets from the host will come
    // from.
    sprintf(command_line, "ip addr add dev %s local 10.0.0.1", ifr.ifr_name);
    system(command_line);

    return 0;
}

int tun_recv(void *buffer, unsigned int length) {
    return read(tun_fd, buffer, length);
}

#define MAX_VECS 32

int tun_sendv(struct iovec *vecs, size_t count) {
    return writev(tun_fd, vecs, count);
}
