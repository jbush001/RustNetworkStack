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
    struct ifreq ifr;
    int err;
    char command_line[256];

    tun_fd = open("/dev/net/tun", O_RDWR);
    if (tun_fd < 0 ) {
        return -1;
    }

    memset(&ifr, 0, sizeof(ifr));
    ifr.ifr_flags = IFF_TUN | IFF_NO_PI;
    err = ioctl(tun_fd, TUNSETIFF, (void*) &ifr);
    if (err < 0) {
        printf("ioctl error: %d\n", err);
        close(tun_fd);
        return -1;
    }

    // Mark the interface as being ready.
    sprintf(command_line, "ip link set dev %s up", ifr.ifr_name);
    printf("%s\n", command_line);
    system(command_line);

    // Ensure anything sent to the 10.0.0.x subnet gets routed to our TUN driver.
    // Our address is hardcoded in netif.rs as 10.0.0.2.
    sprintf(command_line, "ip route add dev %s 10.0.0.0/24", ifr.ifr_name);
    printf("%s\n", command_line);
    system(command_line);

    // Local address of this interface as seen on the virtual network
    // This is the address our stack will see packets from the host as coming from.
    sprintf(command_line, "ip addr add dev %s local 10.0.0.1", ifr.ifr_name);
    printf("%s\n", command_line);
    system(command_line);

    return 0;
}

int tun_recv(void *buffer, int length) {
    return read(tun_fd, buffer, length);
}

int tun_send(const void *buffer, int length) {
    return write(tun_fd, buffer, length);
}
