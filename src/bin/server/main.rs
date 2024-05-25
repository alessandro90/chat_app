use std::{io, net::SocketAddr};

use tokio::{
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener, TcpStream,
    },
    spawn,
    sync::mpsc::{self, Receiver, Sender},
};

const MAX_MSG_BYTES: usize = 1024;
const MAX_CHANNEL_QUEUE_LEN: usize = 256;

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("127.0.0.1:60000")
        .await
        .expect("No client");

    loop {
        let (socket, sock_addr) = listener.accept().await.expect("Cannot accept connection");
        let (reader, writer) = socket.into_split();
        let (sender, receiver) = mpsc::channel(MAX_CHANNEL_QUEUE_LEN);
        spawn(async move {
            if let Err(e) = handle_bytes(reader, sender, sock_addr).await {
                eprintln!("{}", e);
            };
        });
        spawn(async move {
            if let Err(e) = send_bytes(writer, receiver).await {
                eprintln!("{}", e);
            };
        });
    }
}

async fn send_bytes(stream: OwnedWriteHalf, mut receiver: Receiver<String>) -> std::io::Result<()> {
    'main_loop: loop {
        match receiver.recv().await {
            Some(msg) => {
                stream.writable().await?;
                stream
                    .try_write(msg.as_bytes())
                    .expect("Cannot write to stream");
            }
            None => break 'main_loop,
        }
    }
    Ok(())
}

async fn handle_bytes(
    stream: OwnedReadHalf,
    sender: Sender<String>,
    sock_addr: SocketAddr,
) -> std::io::Result<()> {
    println!(
        "Incoming connection. Port: {}, address: {}",
        sock_addr.port(),
        sock_addr.ip()
    );

    let mut buf = [0; MAX_MSG_BYTES];
    loop {
        stream.readable().await?;
        match stream.try_read(&mut buf) {
            Ok(0) => {
                println!("Bye {}", sock_addr.port());
                break;
            }
            Ok(n) => {
                let msg = String::from_utf8_lossy(&buf[..n]);
                println!("{}", msg);
                sender
                    .send(msg.to_string())
                    .await
                    .expect("Cannot send reply");
            }
            // False wake
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                continue;
            }
            Err(e) => {
                return Err(e.into());
            }
        };
    }
    Ok(())
}
