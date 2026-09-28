//! POSIX message queue demo: one binary, two roles (send/recv),
//! chosen by argv[1]. Demonstrates message boundaries and priority ordering
//! — two things a byte-stream IPC (pipe, UDS SOCK_STREAM) does NOT give you.

use nix::mqueue::{mq_close, mq_open, mq_receive, mq_send, mq_unlink, MQ_OFlag, MqAttr};
use nix::sys::stat::Mode;
use std::env;

const QUEUE_NAME: &str = "/capd_demo_queue"; // POSIX mq names must start with '/'

fn main() {
    let role = env::args().nth(1).unwrap_or_default();

    match role.as_str() {
        "send" => sender(),
        "recv" => receiver(),
        "cleanup" => {
            let _ = mq_unlink(QUEUE_NAME);
            println!("[mq] queue unlinked");
        }
        _ => eprintln!("usage: mq_demo <send|recv|cleanup>"),
    }
}

fn sender() {
    // O_CREAT: create if it doesn't exist. Both sender and receiver can
    // safely call mq_open with O_CREAT — POSIX mqueues, unlike FIFOs,
    // don't require one side to "go first."
    let attr = MqAttr::new(0, 10, 256, 0); // max 10 messages queued, 256 bytes each
    let mq = mq_open(
        QUEUE_NAME,
        MQ_OFlag::O_CREAT | MQ_OFlag::O_WRONLY,
        Mode::S_IRUSR | Mode::S_IWUSR,
        Some(&attr),
    )
    .expect("mq_open (sender) failed");

    let messages = [
        (1u32, "routine: housekeeping ping"),
        (1u32, "routine: another low-priority message"),
        (9u32, "ALERT: something urgent, sent LAST but marked high priority"),
    ];

    for (priority, text) in messages {
        mq_send(&mq, text.as_bytes(), priority).expect("mq_send failed");
        println!("[mq][send] queued (priority={priority}): {text}");
    }

    mq_close(mq).expect("mq_close failed");
    println!("[mq][send] done");
}

fn receiver() {
    let attr = MqAttr::new(0, 10, 256, 0);
    let mq = mq_open(
        QUEUE_NAME,
        MQ_OFlag::O_CREAT | MQ_OFlag::O_RDONLY,
        Mode::S_IRUSR | Mode::S_IWUSR,
        Some(&attr),
    )
    .expect("mq_open (receiver) failed");

    let mut buf = [0u8; 256];
    // Drain exactly 3 messages for this demo, printing the order they
    // actually arrive in — priority order, NOT send order.
    for _ in 0..3 {
        let mut priority: u32 = 0;
        match mq_receive(&mq, &mut buf, &mut priority) {
            Ok(n) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                println!("[mq][recv] priority={priority}: {text}");
            }
            Err(e) => {
                eprintln!("[mq][recv] error: {e}");
                break;
            }
        }
    }

    mq_close(mq).expect("mq_close failed");
    println!("[mq][recv] done");
}