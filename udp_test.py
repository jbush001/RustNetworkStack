import socket
import sys

sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock.sendto(bytes('Test message', 'UTF-8'), (sys.argv[1], 8000))
