import socket
import sys

PORT = 8000

sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock.bind(('10.0.0.1', PORT))
sock.sendto(bytes('Test message', 'UTF-8'), (sys.argv[1], PORT))
data, _ = sock.recvfrom(1024)
print(data)

