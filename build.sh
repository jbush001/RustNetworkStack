gcc -c -o netif.o netif.c
ar rcs libnetif.a netif.o
rustc main.rs -l netif -L .
