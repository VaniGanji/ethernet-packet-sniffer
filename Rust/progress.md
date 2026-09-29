# Progress Log

## Daemon fundamentals: FIFO + signals + raw capture
- Built: daemon skeleton, FIFO control channel, SIGTERM/SIGINT → graceful shutdown
- Verified: kill -9 leaves the FIFO orphaned; Ctrl+C cleans it up
- Environment: had to move from macOS to a Linux VM "multipass" (AF_PACKET/epoll are Linux-only)

## epoll + Unix domain sockets
- Built: epoll event loop (replacing sleep-based polling), UDS listener
  alongside the FIFO, token-based dispatch for epoll_wait() results
- Measured: ~0% idle CPU with epoll vs. constant wakeups with polling
- Verified: FIFO and UDS both read/mutate the same shared packet counter —
  confirms control channels are independent of the data path
- Verified drain-loop empirically: sent a 350-byte FIFO message (> the
  256-byte read buffer) in one write; confirmed the loop performed two
  reads (256B + 94B) in the same epoll event, fully draining the message
  rather than leaving bytes stranded in the kernel buffer

## POSIX message queues
- Built: mq_demo (src/bin/mq_demo.rs) — sender and receiver as two
  independently-launched processes, communicating only through a named POSIX
  message queue (/capd_demo_queue)
- Verified real concurrency: started the receiver first, confirmed via
  `ps` that it was alive and blocked *before* the sender process even
  existed, then started the sender independently
- Demonstrated priority ordering: a message sent LAST but marked high
  priority was received FIRST when all sends completed before any
  receive; when the receiver was already blocked and messages arrived
  one at a time, ordering correctly applied only among messages present
  in the queue at each receive call — not a strict global reordering guarantee across the whole exchange

## Shared-memory SPSC ring buffer
- Built: shm_demo (src/bin/shm_demo.rs), a producer and a consumer as two separate processes sharing a POSIX shared-memory segment
- Verified: 12 messages through the ring, in order, with correct wraparound (slot 7 back to slot 0)
- Memory ordering: producer publishes `tail` with Release, consumer reads it
  with Acquire, so the slot write is visible before the new tail is seen.
- Observed backpressure: with the consumer started late, the producer filled
  7 of 8 slots and blocked until the consumer freed one. Usable capacity is
  N-1 by design, so a full ring can be told apart from an empty one
- Stale state: the segment in /dev/shm outlives both processes, so the
  `cleanup` subcommand is needed

## Daemons and systemd
- Wrote capd.service: Type=simple, RuntimeDirectory=capd, Restart=on-failure,
  AmbientCapabilities=CAP_NET_RAW etc.

- Running as a systemd service:
  cargo build --release
  sudo install -m 755 target/release/capd /usr/local/bin/capd
  sudo systemctl daemon-reload
  sudo systemctl start capd
  systemctl status capd
  journalctl -u capd -f

- Changed capd to read $RUNTIME_DIRECTORY for its FIFO/socket paths,
  falling back to /tmp when run manually (no systemd) — verified both
  modes work correctly
- Verified the actual payoff: repeated kill -9 on the running process
  resulted in systemd relaunching it every time, each with a new PID,
  confirmed via `systemctl show capd -p NRestarts` and journalctl showing
  "Main process exited, code=killed, status=9/KILL" followed by
  "Scheduled restart job" for each occurrence
- Verified the contrast case: a clean `systemctl stop`, and a `kill -TERM`
  (which capd's existing signal handler turns into a graceful, code-0
  exit), do NOT trigger a restart.

## systemd - Type=notify and watchdog integration
- Implemented sd_notify: writes plain text (READY=1,
  WATCHDOG=1, STOPPING=1) to a Unix datagram socket at the path systemd
  provides in $NOTIFY_SOCKET
- READY=1 sent only after FIFO, UDS and epoll setup all succeed
- Watchdog pings sent at HALF of WATCHDOG_USEC (systemd's own
  recommendation), so one delayed tick doesn't cause a false-positive kill
- Unit file changed: Type=simple -> Type=notify, added WatchdogSec=10
- Verified the watchdog mechanism definitively: added a deliberate
  sleep(30) right after sd_notify("READY=1") to simulate a hang, rebuilt,
  redeployed, restarted. Confirmed via journalctl:
    - systemd logged "Watchdog timeout (limit 10s)!" exactly 10s after
      start, matching WatchdogSec=10 precisely
    - systemd killed the hung process with SIGABRT (not SIGTERM/SIGKILL) —
      deliberately chosen by systemd to produce a core dump for debugging
    - systemd categorized this as Result=watchdog, distinct from a plain
      crash or clean stop
    - Restart=on-failure then relaunched capd with a new PID, confirming
      full detect-and-recover behavior with zero manual intervention.