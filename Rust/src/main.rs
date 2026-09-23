
//! capd - a packet-capture loop wrapped as a proper Linux daemon.
//!
//! Concepts exercised:
//!   - daemon lifecycle: graceful shutdown on SIGTERM/SIGINT
//!   - Linux IPC: a named pipe (FIFO) used as a control channel
//!   - OS-layer awareness: non-blocking I/O, fd cleanup, FIFO open() semantics
 

use nix::libc;
use nix::sys::socket::{recv, MsgFlags};
use std::os::fd::FromRawFd;
use nix::sys::stat::Mode;
use nix::unistd::{mkfifo, unlink};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
 
const CONTROL_FIFO: &str = "/tmp/capd.ctl";
 
/// Set by the SIGTERM/SIGINT handler. Checked once per loop iteration.
/// Signal handlers must only do async-signal-safe work, so we just flip
/// a flag here and react to it in normal control flow — never do I/O or
/// allocate inside a handler.
static SHUTDOWN: AtomicBool = AtomicBool::new(false);
 
extern "C" fn handle_signal(_sig: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}
 
fn install_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGTERM, handle_signal as *const () as usize);
        libc::signal(libc::SIGINT, handle_signal as *const () as usize);
    }
}
 
/// Open (or create) the control FIFO. Returns a non-blocking read handle.
/// Classic FIFO gotcha: open() on a FIFO blocks until a writer exists,
/// unless you pass O_NONBLOCK.
fn setup_control_fifo() -> std::io::Result<File> {
    let path = Path::new(CONTROL_FIFO);
    if !path.exists() {
        let _ = unlink(CONTROL_FIFO);
        mkfifo(path, Mode::S_IRUSR | Mode::S_IWUSR)
            .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
    }
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
}
 
/// Create a raw AF_PACKET socket capturing all EtherTypes on all interfaces.
///
/// GOTCHA: the `protocol` argument to socket(AF_PACKET, ...) is not optional
/// bookkeeping — it's the EtherType filter the kernel uses to decide which
/// packets to deliver to this socket. Passing 0/None means "deliver nothing".
/// You must pass htons(ETH_P_ALL) to see everything, exactly as a C sniffer
/// using `socket(AF_PACKET, SOCK_RAW, htons(ETH_P_ALL))` would.
/// The `nix` safe wrapper doesn't expose ETH_P_ALL as a `SockProtocol`
/// variant, so this one call goes through raw `libc` — same syscall either way.
fn setup_capture_socket() -> nix::Result<std::os::fd::OwnedFd> {
    const ETH_P_ALL: libc::c_int = 0x0003;
    let proto_be = (ETH_P_ALL as u16).to_be() as libc::c_int;
    let raw_fd = unsafe {
        libc::socket(
            libc::AF_PACKET,
            libc::SOCK_RAW | libc::SOCK_NONBLOCK,
            proto_be,
        )
    };
    if raw_fd < 0 {
        return Err(nix::errno::Errno::last());
    }
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(raw_fd) })
}
 
fn main() -> std::io::Result<()> {
    println!("[capd] starting, pid={}", std::process::id());
    install_signal_handlers();
 
    let mut ctl = setup_control_fifo()?;
    println!("[capd] control channel ready at {CONTROL_FIFO}");
    println!("[capd] try in another shell: echo status > {CONTROL_FIFO}");
 
    let cap_sock = setup_capture_socket();
    if let Err(ref e) = cap_sock {
        eprintln!("[capd] warning: raw capture unavailable ({e}); running with capture disabled");
    }
 
    let packet_count = Arc::new(AtomicU64::new(0));
    let mut buf = [0u8; 65536];
    let mut ctl_buf = [0u8; 256];
 
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            break;
        }
 
        match ctl.read(&mut ctl_buf) {
            Ok(0) => { /* no writer attached right now; normal for a FIFO */ }
            Ok(n) => {
                let cmd = String::from_utf8_lossy(&ctl_buf[..n]);
                handle_command(cmd.trim(), &packet_count);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => eprintln!("[capd] control read error: {e}"),
        }
 
        if let Ok(ref sock) = cap_sock {
            match recv(sock.as_raw_fd(), &mut buf, MsgFlags::MSG_DONTWAIT) {
                Ok(n) => {
                    let total = packet_count.fetch_add(1, Ordering::Relaxed) + 1;
                    if total % 50 == 0 {
                        println!("[capd] captured {total} packets so far (last size={n}B)");
                    }
                }
                Err(nix::errno::Errno::EAGAIN) => {}
                Err(e) => eprintln!("[capd] recv error: {e}"),
            }
        }
 
        std::thread::sleep(Duration::from_millis(20));
    }
 
    println!(
        "[capd] shutdown requested, {} packets captured this run",
        packet_count.load(Ordering::Relaxed)
    );
    let _ = unlink(Path::new(CONTROL_FIFO));
    println!("[capd] control fifo removed, exiting cleanly");
    Ok(())
}
 
fn handle_command(cmd: &str, packet_count: &AtomicU64) {
    match cmd {
        "status" => println!("[capd] STATUS: {} packets captured", packet_count.load(Ordering::Relaxed)),
        "reset" => {
            packet_count.store(0, Ordering::Relaxed);
            println!("[capd] counter reset");
        }
        other => println!("[capd] unknown command: '{other}'"),
    }
}