use std::env;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tun::{AbstractDevice, Configuration, Device};

use env_logger::Env;
use log::{debug, info};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // ログの初期化
    env_logger::Builder::from_env(Env::default().default_filter_or("debug")).init();

    // コマンドライン引数の解析
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} [server|client]", args[0]);
        return Ok(());
    }

    match args[1].as_str() {
        "server" => run_server().await,
        "client" => run_client().await,
        _ => {
            eprintln!("Invalid argument: {}", args[1]);
            eprintln!("Usage: {} [server|client]", args[0]);
            Ok(())
        }
    }
}

async fn run_server() -> std::io::Result<()> {
    // TUNデバイスの設定
    let mut config = Configuration::default();
    config
        .tun_name("tun0")
        .address("10.0.0.1")
        .netmask("255.255.255.0")
        .mtu(1500)
        .up();

    let tun = Device::new(&config).expect("Failed to create TUN device");
    info!(
        "Server TUN device created: {}",
        tun.tun_name().unwrap_or_default()
    );

    // TUNデバイスを共有可能にする
    let tun = Arc::new(Mutex::new(tun));

    // クライアントからの接続を待ち受ける
    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    info!("Server listening on 0.0.0.0:8080");

    let (socket, addr) = listener.accept().await?;
    info!("Client connected: {}", addr);

    // TcpStreamを分割
    let (mut socket_reader, mut socket_writer) = socket.into_split();

    let tun_clone = Arc::clone(&tun);

    // TUN -> ソケットへのデータ転送タスク
    let tun_to_socket = tokio::spawn(async move {
        let mut buf = [0u8; 1504];
        loop {
            // TUNデバイスから読み取り
            let n = tokio::task::block_in_place(|| {
                let mut tun = tun_clone.lock().unwrap();
                tun.read(&mut buf)
            })
            .expect("Failed to read from TUN");

            debug!("Server read {} bytes from TUN: {:?}", n, &buf[..n]);

            // ソケットに書き込み
            if let Err(e) = socket_writer.write_all(&buf[..n]).await {
                eprintln!("Error writing to socket: {}", e);
                break;
            }
            debug!("Server wrote {} bytes to socket", n);
        }
    });

    let tun_clone = Arc::clone(&tun);

    // ソケット -> TUNへのデータ転送タスク
    let socket_to_tun = tokio::spawn(async move {
        let mut buf = [0u8; 1504];
        loop {
            // ソケットから読み取り
            let n = match socket_reader.read(&mut buf).await {
                Ok(n) if n == 0 => {
                    info!("Client disconnected");
                    break;
                }
                Ok(n) => n,
                Err(e) => {
                    eprintln!("Error reading from socket: {}", e);
                    break;
                }
            };
            debug!("Server read {} bytes from socket: {:?}", n, &buf[..n]);

            // TUNデバイスに書き込み
            tokio::task::block_in_place(|| {
                let mut tun = tun_clone.lock().unwrap();
                tun.write_all(&buf[..n]).expect("Failed to write to TUN");
            });
            debug!("Server wrote {} bytes to TUN", n);
        }
    });

    // 両方のタスクが完了するのを待つ
    let _ = tokio::try_join!(tun_to_socket, socket_to_tun);

    Ok(())
}

async fn run_client() -> std::io::Result<()> {
    // TUNデバイスの設定
    let mut config = Configuration::default();
    config
        .tun_name("tun1")
        .address("10.0.0.2")
        .netmask("255.255.255.0")
        .mtu(1500)
        .up();

    let tun = Device::new(&config).expect("Failed to create TUN device");
    info!(
        "Client TUN device created: {}",
        tun.tun_name().unwrap_or_default()
    );

    // TUNデバイスを共有可能にする
    let tun = Arc::new(Mutex::new(tun));

    // サーバーに接続
    let socket = TcpStream::connect("127.0.0.1:8080").await?;
    info!("Connected to server at 127.0.0.1:8080");

    // TcpStreamを分割
    let (mut socket_reader, mut socket_writer) = socket.into_split();

    let tun_clone = Arc::clone(&tun);

    // TUN -> ソケットへのデータ転送タスク
    let tun_to_socket = tokio::spawn(async move {
        let mut buf = [0u8; 1504];
        loop {
            // TUNデバイスから読み取り
            let n = tokio::task::block_in_place(|| {
                let mut tun = tun_clone.lock().unwrap();
                tun.read(&mut buf)
            })
            .expect("Failed to read from TUN");

            debug!("Client read {} bytes from TUN: {:?}", n, &buf[..n]);

            // ソケットに書き込み
            if let Err(e) = socket_writer.write_all(&buf[..n]).await {
                eprintln!("Error writing to socket: {}", e);
                break;
            }
            debug!("Client wrote {} bytes to socket", n);
        }
    });

    let tun_clone = Arc::clone(&tun);

    // ソケット -> TUNへのデータ転送タスク
    let socket_to_tun = tokio::spawn(async move {
        let mut buf = [0u8; 1504];
        loop {
            // ソケットから読み取り
            let n = match socket_reader.read(&mut buf).await {
                Ok(n) if n == 0 => {
                    info!("Server disconnected");
                    break;
                }
                Ok(n) => n,
                Err(e) => {
                    eprintln!("Error reading from socket: {}", e);
                    break;
                }
            };
            debug!("Client read {} bytes from socket: {:?}", n, &buf[..n]);

            // TUNデバイスに書き込み
            tokio::task::block_in_place(|| {
                let mut tun = tun_clone.lock().unwrap();
                tun.write_all(&buf[..n]).expect("Failed to write to TUN");
            });
            debug!("Client wrote {} bytes to TUN", n);
        }
    });

    // 両方のタスクが完了するのを待つ
    let _ = tokio::try_join!(tun_to_socket, socket_to_tun);

    Ok(())
}
