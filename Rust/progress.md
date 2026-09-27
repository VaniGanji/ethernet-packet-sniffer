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