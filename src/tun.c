
// This create a device that will appear in the host as a network interface.
// It will set up an IP route so that any packets sent to 10.0.0.1 will
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

int tun_init(const char *remote_ip_addr) {
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

    sprintf(command_line, "ip link set dev %s up", ifr.ifr_name);
    system(command_line);
    sprintf(command_line, "ip route add dev %s %d.%d.%d.%d", ifr.ifr_name,
        remote_ip_addr[0], remote_ip_addr[1], remote_ip_addr[2], remote_ip_addr[3]);
    system(command_line);

    return 0;
}

int tun_recv(void *buffer, int length) {
    return read(tun_fd, buffer, length);
}

int tun_send(const void *buffer, int length) {
    return write(tun_fd, buffer, length);
}
