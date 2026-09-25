
//! capd - a packet-capture loop wrapped as a proper Linux daemon.
//!
//! Concepts exercised:
//!   - daemon lifecycle: graceful shutdown on SIGTERM/SIGINT
//!   - Linux IPC: a named pipe (FIFO) used as a control channel
//!   - OS-layer awareness: non-blocking I/O, fd cleanup, FIFO open() semantics
 

use nix::libc;
use nix::sys::socket::{recv, MsgFlags, accept, bind, listen, socket, AddressFamily, Backlog, SockFlag, SockType, UnixAddr};
use std::os::fd::{FromRawFd, AsRawFd, OwnedFd};
use nix::sys::stat::Mode;
use nix::unistd::{mkfifo, unlink};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags};
use std::collections::HashMap;

 
const CONTROL_FIFO: &str = "/tmp/capd.ctl";
const CONTROL_SOCK: &str = "/tmp/capd.sock";
 
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
    let _ = unlink(path); // defensive: clear a stale FIFO left by a kill -9
    mkfifo(path, Mode::S_IRUSR | Mode::S_IWUSR)
        .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
}

/// A Unix domain socket, SOCK_STREAM, listening for control connections.
/// Unlike the FIFO, this supports multiple simultaneous clients and gives
/// each one its own connection (its own fd) rather than one shared channel.
fn setup_control_socket() -> nix::Result<OwnedFd> {
    let path = Path::new(CONTROL_SOCK);
    let _ = unlink(path); // stale socket file from a previous crashed run
    let listener = socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::SOCK_NONBLOCK,
        None,
    )?;
    let addr = UnixAddr::new(path)?;
    bind(listener.as_raw_fd(), &addr)?;
    listen(&listener, Backlog::new(8).unwrap())?;
    Ok(listener)
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

fn handle_command(cmd: &str, packet_count: &AtomicU64) -> String {
    match cmd {
        "status" => format!("STATUS: {} packets captured\n", packet_count.load(Ordering::Relaxed)),
        "reset" => {
            packet_count.store(0, Ordering::Relaxed);
            "OK: counter reset\n".to_string()
        }
        other => format!("ERR: unknown command '{other}'\n"),
    }
}
 
fn main() -> std::io::Result<()> {
    println!("[capd] starting, pid={}", std::process::id());
    install_signal_handlers();
 
    let mut ctl_fifo = setup_control_fifo()?;
    println!("[capd] FIFO control channel ready at {CONTROL_FIFO}");
 
    let ctl_sock =
        setup_control_socket().map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
    println!("[capd] UDS control channel ready at {CONTROL_SOCK}");
    println!("[capd] try: echo status > {CONTROL_FIFO}");
    println!("[capd] or:  echo status | nc -U {CONTROL_SOCK}");
 
    let cap_sock = setup_capture_socket();
    if let Err(ref e) = cap_sock {
        eprintln!("[capd] warning: raw capture unavailable ({e}); running with capture disabled");
    }
 
    let packet_count = Arc::new(AtomicU64::new(0));

        // ---- epoll setup ----
    // Create the epoll instance itself — a kernel-side "interest list" of
    // fds we want to be notified about.
    let epoll = Epoll::new(EpollCreateFlags::empty())?;
 
    // Register each fd we care about, tagging it with a small integer "token"
    // (via EpollEvent's u64 data field) so that when epoll_wait tells us
    // "fd X is ready," we know which of our fds that corresponds to.
    const TOKEN_FIFO: u64 = 1;
    const TOKEN_LISTENER: u64 = 2;
    const TOKEN_CAPTURE: u64 = 3;
    const TOKEN_CLIENT_BASE: u64 = 1000; // client fds get tokens >= this
 
    epoll.add(&ctl_fifo, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_FIFO))?;
    epoll.add(&ctl_sock, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_LISTENER))?;
    if let Ok(ref sock) = cap_sock {
        epoll.add(sock, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_CAPTURE))?;
    }
 
    // Track connected UDS clients: token -> owned fd, so we can read from
    // and eventually deregister/close them.
    let mut clients: HashMap<u64, OwnedFd> = HashMap::new();
    let mut next_client_token: u64 = TOKEN_CLIENT_BASE;
    let mut events = vec![EpollEvent::empty(); 16];
    let mut buf = [0u8; 65536];
    let mut ctl_buf = [0u8; 256];
 
    println!("[capd] entering epoll event loop (blocking, zero busy-wait)");
 
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            break;
        }
 
        // epoll_wait blocks here — genuinely zero CPU used — until at least
        // one registered fd is ready, or the timeout (100ms) elapses (the
        // timeout exists mainly so we re-check SHUTDOWN periodically even
        // if nothing is happening; a signal also interrupts this early).
        let n_ready = match epoll.wait(&mut events, 100u16) {
            Ok(n) => n,
            Err(nix::errno::Errno::EINTR) => continue, // interrupted by our signal handler
            Err(e) => {
                eprintln!("[capd] epoll_wait error: {e}");
                continue;
            }
        };
 
        for ev in &events[..n_ready] {
            let token = ev.data();
            match token {
                TOKEN_FIFO => {
                    loop {
                        match ctl_fifo.read(&mut ctl_buf) {
                            Ok(0) => break,                                   // FIFO: 0 means genuinely no writer/no data
                            Ok(n) => {
                                let cmd = String::from_utf8_lossy(&ctl_buf[..n]);
                                let reply = handle_command(cmd.trim(), &packet_count);
                                print!("[capd][fifo] {reply}");
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                            Err(e) => { eprintln!("[capd] fifo read error: {e}"); break; }
                        }
                    }
                }
                TOKEN_LISTENER => {
                    // A new client is connecting to the UDS listener.
                    match accept(ctl_sock.as_raw_fd()) {
                        Ok(client_fd) => {
                            let client = unsafe { OwnedFd::from_raw_fd(client_fd) };
                            let token = next_client_token;
                            next_client_token += 1;
                            epoll.add(&client, EpollEvent::new(EpollFlags::EPOLLIN, token))?;
                            clients.insert(token, client);
                            println!("[capd] client connected (token {token})");
                        }
                        Err(nix::errno::Errno::EAGAIN) => {}
                        Err(e) => eprintln!("[capd] accept error: {e}"),
                    }
                }
                TOKEN_CAPTURE => {
                    if let Ok(ref sock) = cap_sock {
                        loop {
                            match recv(sock.as_raw_fd(), &mut buf, MsgFlags::MSG_DONTWAIT) {
                                Ok(n) => {
                                    let total = packet_count.fetch_add(1, Ordering::Relaxed) + 1;
                                    if total % 50 == 0 {
                                        println!(
                                            "[capd] captured {total} packets so far (last size={n}B)"
                                        );
                                    }
                                }
                                Err(nix::errno::Errno::EAGAIN) => break,
                                Err(e) => {
                                    eprintln!("[capd] recv error: {e}");
                                    break;
                                }
                            }
                        }
                    }
                }
                client_token => {
                    if let Some(fd) = clients.get(&client_token) {
                        loop {
                            match recv(fd.as_raw_fd(), &mut ctl_buf, MsgFlags::MSG_DONTWAIT) {
                                Ok(0) => {
                                    let fd = clients.remove(&client_token).unwrap();
                                    let _ = epoll.delete(&fd);
                                    println!("[capd] client disconnected (token {client_token})");
                                    break;
                                }
                                Ok(n) => {
                                    let cmd = String::from_utf8_lossy(&ctl_buf[..n]);
                                    let reply = handle_command(cmd.trim(), &packet_count);
                                    print!("[capd][uds token={client_token}] {reply}");
                                    let _ = nix::sys::socket::send(fd.as_raw_fd(), reply.as_bytes(), MsgFlags::empty());
                                }
                                Err(nix::errno::Errno::EAGAIN) => break,
                                Err(e) => { eprintln!("[capd] client recv error: {e}"); break; }
                            }
                        }
                    }
                }
            }
        }
    }
 
    println!(
        "[capd] shutdown requested, {} packets captured this run",
        packet_count.load(Ordering::Relaxed)
    );
    let _ = unlink(Path::new(CONTROL_FIFO));
    let _ = unlink(Path::new(CONTROL_SOCK));
    println!("[capd] control fifo and socket removed, exiting cleanly");
    Ok(())
}