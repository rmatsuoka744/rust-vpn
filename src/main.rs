use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

use log::{debug, error, info};
use nix::libc;

#[derive(Debug)]
struct TunInterface {
    file: File,
    name: String,
}

impl TunInterface {
    fn new(name: &str) -> std::io::Result<TunInterface> {
        info!("Starting TUN interface creation: {}", name);
        let fd = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/net/tun")?;

        #[repr(C)]
        struct Ifreq {
            ifr_name: [u8; libc::IFNAMSIZ],
            ifr_flags: libc::c_short,
            _pad: [u8; 64],
        }

        let mut ifr_name = [0u8; libc::IFNAMSIZ];
        for (i, c) in name.bytes().enumerate() {
            ifr_name[i] = c;
        }

        let flags: libc::c_short = (libc::IFF_TUN | libc::IFF_NO_PI) as i16;

        let mut ifr = Ifreq {
            ifr_name,
            ifr_flags: flags,
            _pad: [0u8; 64],
        };

        let res = unsafe { libc::ioctl(fd.as_raw_fd(), libc::TUNSETIFF, &mut ifr as *mut _) };
        if res < 0 {
            return Err(std::io::Error::last_os_error());
        }

        info!("TUN interface {} created successfully.", name);
        Ok(TunInterface {
            file: fd,
            name: name.to_string(),
        })
    }

    fn set_ip(&self, cidr: &str) -> std::io::Result<()> {
        info!("Setting IP {} on {}", cidr, self.name);
        let status = Command::new("ip")
            .args(&["addr", "add", cidr, "dev", &self.name])
            .status()?;
        if !status.success() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to set IP on TUN",
            ));
        }

        let status = Command::new("ip")
            .args(&["link", "set", "dev", &self.name, "up"])
            .status()?;
        if !status.success() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to set TUN up",
            ));
        }
        info!("TUN interface {} is up with IP {}.", self.name, cidr);
        Ok(())
    }

    fn read_packet(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.file.read(buf)?;
        if n > 0 {
            debug!("Read {} bytes from TUN {}:", n, self.name);
            hexdump(&buf[..n]);
        }
        Ok(n)
    }

    fn write_packet(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        debug!("Writing {} bytes to TUN {}:", buf.len(), self.name);
        hexdump(buf);
        self.file.write(buf)
    }
}

// Simple hex dump function
fn hexdump(data: &[u8]) {
    for chunk in data.chunks(16) {
        debug!("  {:02X?}", chunk.iter().map(|b| *b).collect::<Vec<u8>>());
    }
}

// Utility to read a line from a TCP stream
fn read_line(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line)
}

// Write a line to a TCP stream
fn write_line(stream: &mut TcpStream, line: &str) -> std::io::Result<()> {
    debug!("Sending line to TCP peer: {}", line.trim_end());
    stream.write_all(line.as_bytes())?;
    Ok(())
}

// Send a packet with a 2-byte header containing length (big-endian)
fn send_vpn_packet(stream: &mut TcpStream, packet: &[u8]) -> std::io::Result<()> {
    if packet.len() > 0xFFFF {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Packet too large",
        ));
    }
    info!("Sending VPN packet of {} bytes to TCP peer.", packet.len());
    debug!(
        "VPN header: length = {} (0x{:04X})",
        packet.len(),
        packet.len()
    );
    hexdump(packet);
    let len = (packet.len() as u16).to_be_bytes();
    stream.write_all(&len)?;
    stream.write_all(packet)?;
    info!("Sent VPN packet ({} bytes) successfully.", packet.len());
    Ok(())
}

// Receive a packet with a 2-byte header containing length
fn recv_vpn_packet(stream: &mut TcpStream, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut len_buf = [0u8; 2];
    match stream.read_exact(&mut len_buf) {
        Ok(_) => {}
        Err(e) => {
            info!("No more data or error while reading VPN packet length.");
            return Err(e);
        }
    };
    let length = u16::from_be_bytes(len_buf) as usize;
    info!("Receiving VPN packet: expected length = {} bytes.", length);
    if length > buf.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Packet too large for buffer",
        ));
    }
    stream.read_exact(&mut buf[..length])?;
    debug!("Received {} bytes from TCP:", length);
    hexdump(&buf[..length]);
    info!("Received VPN packet ({} bytes) successfully.", length);
    Ok(length)
}

fn server_mode(bind_addr: &str, port: &str, tun_ip: &str, tun_name: &str) -> std::io::Result<()> {
    info!("Starting server mode.");
    let tun = TunInterface::new(tun_name)?;
    tun.set_ip(tun_ip)?;
    let tun = Arc::new(Mutex::new(tun));

    let listener = TcpListener::bind(format!("{}:{}", bind_addr, port))?;
    info!("Server listening on {}:{}", bind_addr, port);

    let (mut stream, addr) = listener.accept()?;
    info!("Client connected from: {:?}", addr);

    // Handshake
    info!("Starting handshake with client...");
    let mut line = read_line(&mut stream)?;
    line = line.trim_end().to_string();
    let client_ip = line;
    info!("Client requested IP: {}", client_ip);

    write_line(&mut stream, "OK\n")?;
    info!("Handshake complete. Start forwarding packets.");

    // Thread: TUN -> Server -> Client
    let tun_rx = tun.clone();
    let mut stream_tx = stream.try_clone()?;
    let tun_tx_handle = thread::spawn(move || {
        info!("TUN->Client forwarding thread started.");
        let mut buf = [0u8; 1500];
        loop {
            let n = {
                let mut t = tun_rx.lock().unwrap();
                match t.read_packet(&mut buf) {
                    Ok(n) => n,
                    Err(e) => {
                        error!("Error reading from TUN: {}", e);
                        break;
                    }
                }
            };

            if n == 0 {
                info!("No data from TUN. Possibly link down or closed.");
            } else {
                if let Err(e) = send_vpn_packet(&mut stream_tx, &buf[..n]) {
                    error!("Error sending packet to client: {}", e);
                    break;
                }
            }
        }
        info!("TUN->Client forwarding thread ended.");
    });

    // Main: Client -> Server -> TUN
    info!("Client->TUN forwarding loop started.");
    let mut buf = [0u8; 1500];
    loop {
        let n = match recv_vpn_packet(&mut stream, &mut buf) {
            Ok(n) => n,
            Err(e) => {
                error!("Error receiving from client: {}", e);
                break;
            }
        };

        if n == 0 {
            info!("Received zero-length packet. Possibly connection closed.");
            break;
        }

        let mut t = tun.lock().unwrap();
        if let Err(e) = t.write_packet(&buf[..n]) {
            error!("Error writing to TUN: {}", e);
            break;
        }
    }

    info!("Client->TUN forwarding loop ended. Waiting for TUN->Client thread to finish.");
    tun_tx_handle.join().ok();
    info!("Server shutting down.");
    Ok(())
}

fn client_mode(server_addr: &str, port: &str, my_ip: &str, tun_name: &str) -> std::io::Result<()> {
    info!(
        "Starting client mode. Connecting to {}:{}...",
        server_addr, port
    );
    let mut stream = TcpStream::connect(format!("{}:{}", server_addr, port))?;
    info!("Connected to server.");

    info!("Starting handshake with server...");
    write_line(&mut stream, &format!("{}\n", my_ip))?;

    let line = read_line(&mut stream)?;
    info!("Server response: {}", line.trim_end());

    let tun = TunInterface::new(tun_name)?;
    tun.set_ip(my_ip)?;
    let tun = Arc::new(Mutex::new(tun));

    info!("Handshake complete. Start forwarding packets.");

    // Thread: TUN -> Client -> Server
    let tun_rx = tun.clone();
    let mut stream_tx = stream.try_clone()?;
    let tun_tx_handle = thread::spawn(move || {
        info!("TUN->Server forwarding thread started.");
        let mut buf = [0u8; 1500];
        loop {
            let n = {
                let mut t = tun_rx.lock().unwrap();
                match t.read_packet(&mut buf) {
                    Ok(n) => n,
                    Err(e) => {
                        error!("Error reading from TUN: {}", e);
                        break;
                    }
                }
            };

            if n == 0 {
                info!("No data from TUN. Possibly link down or closed.");
            } else {
                if let Err(e) = send_vpn_packet(&mut stream_tx, &buf[..n]) {
                    error!("Error sending packet to server: {}", e);
                    break;
                }
            }
        }
        info!("TUN->Server forwarding thread ended.");
    });

    // Main: Server -> Client -> TUN
    info!("Server->TUN forwarding loop started.");
    let mut buf = [0u8; 1500];
    loop {
        let n = match recv_vpn_packet(&mut stream, &mut buf) {
            Ok(n) => n,
            Err(e) => {
                error!("Error receiving from server: {}", e);
                break;
            }
        };

        if n == 0 {
            info!("Received zero-length packet. Possibly connection closed.");
            break;
        }

        let mut t = tun.lock().unwrap();
        if let Err(e) = t.write_packet(&buf[..n]) {
            error!("Error writing to TUN: {}", e);
            break;
        }
    }

    info!("Server->TUN forwarding loop ended. Waiting for TUN->Server thread to finish.");
    tun_tx_handle.join().ok();
    info!("Client shutting down.");
    Ok(())
}

fn main() {
    env_logger::init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 6 {
        eprintln!("Usage:");
        eprintln!(
            "  Server: {} server <bind_addr> <port> <tun_ip_cidr> <tun_name>",
            args[0]
        );
        eprintln!(
            "  Client: {} client <server_addr> <port> <my_ip_cidr> <tun_name>",
            args[0]
        );
        return;
    }

    let mode = &args[1];
    if mode == "server" {
        let bind_addr = &args[2];
        let port = &args[3];
        let tun_ip = &args[4];
        let tun_name = &args[5];
        if let Err(e) = server_mode(bind_addr, port, tun_ip, tun_name) {
            error!("Server error: {}", e);
        }
    } else if mode == "client" {
        let server_addr = &args[2];
        let port = &args[3];
        let my_ip = &args[4];
        let tun_name = &args[5];
        if let Err(e) = client_mode(server_addr, port, my_ip, tun_name) {
            error!("Client error: {}", e);
        }
    } else {
        error!("Invalid mode: {}", mode);
    }
}
