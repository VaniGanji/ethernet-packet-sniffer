//! Shared-memory SPSC (single-producer, single-consumer) ring
//! buffer. Two processes, no syscalls per message once mapped — the
//! fastest of the four IPC mechanisms, and the one where correctness
//! depends on memory ORDERING, not just the syscalls themselves.
//!
//! Layout in shared memory:
//!   [ RingHeader { head: AtomicUsize, tail: AtomicUsize } ]
//!   [ Slot 0 ][ Slot 1 ] ... [ Slot N-1 ]
//! Each Slot is a fixed-size length-prefixed byte buffer.

use nix::fcntl::OFlag;
use nix::sys::mman::{mmap, shm_open, shm_unlink, MapFlags, ProtFlags};
use nix::sys::stat::Mode;
use nix::unistd::ftruncate;
use std::env;
use std::num::NonZeroUsize;
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicUsize, Ordering};

const SHM_NAME: &str = "/capd_demo_ring";
const RING_CAPACITY: usize = 8; // number of slots (power of two keeps the modulo cheap)
const SLOT_SIZE: usize = 64; // max bytes per message

#[repr(C)]
struct RingHeader {
    head: AtomicUsize, // consumer's next slot to read
    tail: AtomicUsize, // producer's next slot to write
}

#[repr(C)]
struct Slot {
    len: AtomicUsize,
    data: [u8; SLOT_SIZE],
}

const HEADER_SIZE: usize = std::mem::size_of::<RingHeader>();
const SLOT_STRIDE: usize = std::mem::size_of::<Slot>();
const TOTAL_SIZE: usize = HEADER_SIZE + RING_CAPACITY * SLOT_STRIDE;

fn main() {
    let role = env::args().nth(1).unwrap_or_default();
    match role.as_str() {
        "producer" => producer(),
        "consumer" => consumer(),
        "cleanup" => {
            let _ = shm_unlink(SHM_NAME);
            println!("[shm] segment unlinked");
        }
        _ => eprintln!("usage: shm_demo <producer|consumer|cleanup>"),
    }
}

/// Open (creating if needed) and map the shared memory segment. Returns a
/// raw pointer to the start — both header and slots are reached via
/// pointer arithmetic from here, since ordinary Rust references can't
/// span two independent processes' address spaces.
fn map_shared_memory(create: bool) -> *mut u8 {
    let oflag = if create {
        OFlag::O_CREAT | OFlag::O_RDWR
    } else {
        OFlag::O_RDWR
    };

    /* shm_open   → create a named, empty (0-byte) object in /dev/shm
    ftruncate  → give it a size (592 bytes) (16 bytes header + 8 slots * 72 bytes each)
    mmap       → map those bytes into this process's address space */

    let fd = shm_open(SHM_NAME, oflag, Mode::S_IRUSR | Mode::S_IWUSR)
        .expect("shm_open failed");

    if create {
        ftruncate(fd.as_fd(), TOTAL_SIZE as i64).expect("ftruncate failed");
    }

    let addr = unsafe {
        mmap(
            None,
            NonZeroUsize::new(TOTAL_SIZE).unwrap(),
            ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
            MapFlags::MAP_SHARED,
            fd.as_fd(),
            0,
        )
        .expect("mmap failed")
    };

    addr.as_ptr() as *mut u8
}

unsafe fn header_ref(base: *mut u8) -> &'static RingHeader {
    unsafe { &*(base as *const RingHeader) }
}

unsafe fn slot_ptr(base: *mut u8, index: usize) -> *mut Slot {
    unsafe { base.add(HEADER_SIZE + index * SLOT_STRIDE) as *mut Slot }
}

fn producer() {
    let base = map_shared_memory(true);
    let header = unsafe { header_ref(base) };
    // Fresh segment: initialize head/tail to 0 (safe here since producer
    // runs first in this demo; a real system would use a separate
    // "initialized" flag or a named semaphore to avoid this race).
    header.head.store(0, Ordering::Relaxed);
    header.tail.store(0, Ordering::Relaxed);

    println!("[shm][producer] ring ready, capacity={RING_CAPACITY} slots x {SLOT_SIZE}B");

    for i in 0..12 {
        let msg = format!("packet #{i}");
        loop {
            let tail = header.tail.load(Ordering::Relaxed);
            let head = header.head.load(Ordering::Acquire); // see consumer's latest progress
            let next_tail = (tail + 1) % RING_CAPACITY;
            if next_tail == head {
                // ring is full — a real system would back off or drop here
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            }

            unsafe {
                let slot = slot_ptr(base, tail);
                let bytes = msg.as_bytes();
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*slot).data.as_mut_ptr(), bytes.len());
                (*slot).len.store(bytes.len(), Ordering::Relaxed);
            }

            // RELEASE: publishes both the slot data AND the new tail value
            // together. Anything the consumer reads via an ACQUIRE load of
            // `tail` that sees this new value is guaranteed to also see
            // the slot write above — that ordering guarantee is the whole
            // reason this isn't just a Relaxed store.
            header.tail.store(next_tail, Ordering::Release);
            println!("[shm][producer] wrote slot {tail}: {msg}");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    println!("[shm][producer] done");
}

fn consumer() {
    let base = map_shared_memory(false);
    let header = unsafe { header_ref(base) };

    let mut received = 0;
    while received < 12 {
        let head = header.head.load(Ordering::Relaxed);
        // ACQUIRE: pairs with the producer's Release store above. Seeing
        // a new `tail` value here guarantees we also see the slot bytes
        // the producer wrote just before publishing it.
        let tail = header.tail.load(Ordering::Acquire);
        if head == tail {
            std::thread::sleep(std::time::Duration::from_millis(5));
            continue;
        }

        let msg = unsafe {
            let slot = slot_ptr(base, head);
            let len = (*slot).len.load(Ordering::Relaxed);
            let bytes = std::slice::from_raw_parts((*slot).data.as_ptr(), len);
            String::from_utf8_lossy(bytes).to_string()
        };
        println!("[shm][consumer] read slot {head}: {msg}");

        let next_head = (head + 1) % RING_CAPACITY;
        header.head.store(next_head, Ordering::Release); // tells producer this slot is free again
        received += 1;
    }
    println!("[shm][consumer] done");
}