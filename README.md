# Ethernet Packet Sniffer

A lightweight multithreaded Ethernet packet sniffer implemented in C using `libpcap` and POSIX threads.

The sniffer captures Ethernet frames from a selected network interface, parses Layer 2/3/4 headers, and displays protocol information for supported packet types.

---

# Features

* Ethernet frame capture using `libpcap`
* Layer 2: Ethernet header parsing
* Layer 2: VLAN (802.1Q) header parsing
* Layer 3: IPv4 packet parsing
* Layer 4: TCP and UDP header parsing

---

# Architecture Overview

The sniffer uses a producer-consumer multithreaded architecture.

+---------------------+
| libpcap Capture     |
| Main Thread         |
| (Producer)          |
+----------+----------+
           |
           v
+---------------------+
| Packet Queue        |
| Thread-safe Ring    |
| Buffer              |
+----------+----------+
           |
           v
+---------------------+
| Decoder Thread      |
| Protocol Parsing    |
| CLI output          |
| (Consumer)          |
+---------------------+

---

# Execution flow

NIC Hardware
   ↓
Kernel RX buffer
   ↓
libpcap internal buffer
   ↓
pcap_dispatch()
   ↓
packet_handler() callback
   ↓
enqueue_packet()
   ↓
Queue
   ↓
Consumer thread
   ↓
dequeue_packet()
   ↓
Parser + Printing

---

# Build Instructions
## macOS
Install libpcap (if needed): brew install libpcap
make

---

# Run
## macOS
sudo ./packet_sniffer en0

---

# Known Limitations

* Only IPv4 payloads are processed
* Other Layer 3 protocols such as IPv6/ARP are ignored
* TCP/UDP payload decoding not implemented
* No packet timestamp logging
* No BPF filter configuration interface

---