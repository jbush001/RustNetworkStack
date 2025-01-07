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

import socket
import sys

PORT = 8000

v6 = len(sys.argv) > 2 and sys.argv[2] == 'v6'

sock = socket.socket(socket.AF_INET6 if v6 else socket.AF_INET, socket.SOCK_DGRAM)
if v6:
    bindaddr = ('fe80::1', PORT, 0, socket.if_nametoindex('tun0'))
else:
    bindaddr = ('10.0.0.1', PORT)

sock.bind(bindaddr)
for _ in range(20):
    sock.sendto(bytes('Test message ' * 100, 'UTF-8'), (sys.argv[1], PORT))
    data, _ = sock.recvfrom(1024)
    print(data)

