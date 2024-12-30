# Create a TCP server that can accept incoming connections and generates
# a continuous stream of characters from the ASCII character set.

import socket

PORT = 3000

PATTERN = ''.join(chr(i) for i in range(32, 126)) * 2
LINE_LEN = 72

listen_sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
listen_sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listen_sock.bind(('', PORT))
listen_sock.listen(1)

print('Listening on port', PORT)
while True:
    client_conn, addr = listen_sock.accept()
    print(f'Accepted socket from {addr}')
    sock_active = True
    try:
        while True:
            for offset in range(0, len(PATTERN) // 2):
                client_conn.send(bytes(PATTERN[offset:offset + LINE_LEN] + '\r\n', 'UTF-8'))
    except BrokenPipeError:
        client_conn.close()



