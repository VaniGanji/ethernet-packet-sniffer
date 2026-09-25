Ethernet Packet Sniffer — from C to Rust, and into Linux systems internals

A personal learning project that starts from an existing C packet sniffer and uses it as the seed for a broader, hands-on exploration of Linux systems programming in Rust: daemons, IPC mechanisms and event-driven I/O.

The original C sniffer captures raw Ethernet traffic. Rather than leave it as a standalone tool, this project reimplements and extends that same core idea — raw packet capture — in Rust, then builds an entire small systems stack around it: a proper daemon architecture, multiple IPC mechanisms (pipes, Unix domain sockets, shared memory, message queues) and event loop.

What's being built
A Rust daemon wrapping raw AF_PACKET capture, with graceful shutdown, signal handling, and capability-based privilege (no sudo required at runtime)
Multiple IPC mechanisms, implemented and compared side by side rather than picked arbitrarily — FIFOs, Unix domain sockets, shared memory, POSIX message queues
An epoll-based event loop, replacing naive polling, handling multiple simultaneous clients
Daemon supervision via systemd — restart policies, watchdogs, and clean shutdown, going beyond what the original C version did
Protocol parsing and test tooling — structured binary protocol handling and an automated Python test harness

Why C → Rust

The port isn't about replacing working C code — it's a deliberate way to learn Rust's ownership model, unsafe/FFI boundaries, and systems-level APIs (nix/libc).

cargo build
sudo setcap cap_net_raw,cap_net_admin=eip target/debug/capd
./target/debug/capd
