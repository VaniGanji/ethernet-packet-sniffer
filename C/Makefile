CC=gcc
CFLAGS=-Wall -Wextra -O2 -pthread
LIBS=-lpcap

INCLUDES=-Iinclude

SRC := $(wildcard src/*.c)

TARGET=ethernet_packet_sniffer

all:
	$(CC) $(CFLAGS) $(INCLUDES) $(SRC) -o $(TARGET) $(LIBS)

clean:
	rm -f $(TARGET)
