#
# Copyright 2024 Jeff Bush
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#

# Create a TCP server that can accept incoming connections and generates
# a continuous stream of characters from the ASCII character set.

import socket
import sys

v6 = len(sys.argv) > 1 and sys.argv[1] == 'v6'
if v6:
    print('Using IPv6')
else:
    print('Using IPv4')

PORT = 3000

PATTERN = ''.join(chr(i) for i in range(32, 126)) * 2
LINE_LEN = 72

listen_sock = socket.socket(socket.AF_INET6 if v6 else socket.AF_INET, socket.SOCK_STREAM)
listen_sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listen_sock.bind(('', PORT))
listen_sock.listen(1)

print('Listening on port', PORT)
while True:
    client_conn, addr = listen_sock.accept()
    print(f'Accepted socket from {addr}')
    try:
        while True:
            for offset in range(0, len(PATTERN) // 2):
                client_conn.send(bytes(PATTERN[offset:offset + LINE_LEN] + '\r\n', 'UTF-8'))
    except BrokenPipeError:
        client_conn.close()



