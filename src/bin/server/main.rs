use std::{collections::HashMap, io, net::SocketAddr};
use tokio::{
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener,
    },
    spawn,
    sync::mpsc::{self, Sender},
};

const MAX_MSG_BYTES: usize = 1024;
const MAX_CHANNEL_QUEUE_LEN: usize = 256;
const MAX_SIMULATANEOUS_INCOMING_CONNECTIONS: usize = 32;

enum Connection {
    Push {
        sockaddr: SocketAddr,
        stream_writer: OwnedWriteHalf,
    },
    Pop(SocketAddr),
}

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("127.0.0.1:60000")
        .await
        .expect("No client");

    let (incomng_connetions_sender, mut incoming_connections_receiver) =
        mpsc::channel(MAX_SIMULATANEOUS_INCOMING_CONNECTIONS);
    let (incomng_msg_sender, mut incoming_msg_receiver) =
        mpsc::channel::<String>(MAX_CHANNEL_QUEUE_LEN);
    spawn(async move {
        let mut connections = HashMap::new();
        loop {
            tokio::select! {
                val = incoming_connections_receiver.recv() => {
                    if let Some(val) = val {
                        match val {
                            Connection::Push{sockaddr, stream_writer} => {
                                let _ = connections.insert(sockaddr, stream_writer);
                            },
                            Connection::Pop(ref sockaddr) => {
                                let _ = connections.remove(sockaddr);
                            }
                        };
                    }
                },
                msg = incoming_msg_receiver.recv() => {
                    if let Some(msg) = msg {
                        for stream in connections.values() {
                            stream.writable().await.expect("Stream not writable");
                            stream
                                .try_write(msg.as_bytes())
                                .expect("Cannot write to stream");
                        }
                    }
                }
            }
        }
    });

    loop {
        let (socket, sock_addr) = listener.accept().await.expect("Cannot accept connection");
        let (stream_reader, stream_writer) = socket.into_split();
        incomng_connetions_sender
            .send(Connection::Push {
                sockaddr: sock_addr,
                stream_writer,
            })
            .await
            .expect("Cannot queue new connection");
        let incomng_msg_sender_cloned = incomng_msg_sender.clone();
        let incomng_connections_sender_cloned = incomng_connetions_sender.clone();
        spawn(async move {
            if let Err(e) = handle_bytes(
                stream_reader,
                incomng_msg_sender_cloned,
                sock_addr,
                incomng_connections_sender_cloned,
            )
            .await
            {
                eprintln!("{}", e);
            };
        });
    }
}

async fn handle_bytes(
    stream: OwnedReadHalf,
    sender: Sender<String>,
    sock_addr: SocketAddr,
    connection_sender: Sender<Connection>,
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
                connection_sender
                    .send(Connection::Pop(sock_addr))
                    .await
                    .expect("Cannot send pop conncetion request");
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
