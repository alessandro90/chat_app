use std::{collections::HashMap, io, net::SocketAddr, sync::Arc};
use tokio::{
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener,
    },
    spawn,
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex,
    },
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

struct Connections {
    entries: Arc<Mutex<HashMap<SocketAddr, OwnedWriteHalf>>>,
}

impl Default for Connections {
    fn default() -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Connections {
    async fn handle_conn(&self, conn: Connection) {
        match conn {
            Connection::Push {
                sockaddr,
                stream_writer,
            } => {
                println!("added connection: {}", sockaddr);
                let _ = self.entries.lock().await.insert(sockaddr, stream_writer);
            }
            Connection::Pop(ref sockaddr) => {
                println!("removed connection: {}", sockaddr);
                let _ = self.entries.lock().await.remove(sockaddr);
            }
        };
    }

    async fn msg_broadcast(&self, msg: String) {
        for sockaddr in self.entries.lock().await.keys().cloned() {
            let msg = msg.clone();
            let entries = Arc::clone(&self.entries);
            spawn(async move {
                let entries = &entries.lock().await;
                if let Some(stream) = entries.get(&sockaddr) {
                    stream.writable().await.expect("Stream not writable");
                    stream
                        .try_write(msg.as_bytes())
                        .expect("Cannot write to stream");
                }
            });
        }
    }
}

async fn connections_task(mut conn_recv: Receiver<Connection>, mut msg_recv: Receiver<String>) {
    spawn(async move {
        let connections = Connections::default();
        loop {
            tokio::select! {
                conn = conn_recv.recv() => {
                    if let Some(conn) = conn {
                        connections.handle_conn(conn).await;
                    }
                },
                msg = msg_recv.recv() => {
                    if let Some(msg) = msg {
                        connections.msg_broadcast(msg).await;
                    }
                }
            }
        }
    });
}

struct MsgHandler {
    listener: TcpListener,
    conn_sender: Sender<Connection>,
    msg_sender: Sender<String>,
}

impl MsgHandler {
    async fn new(
        ip: &str,
        port: u16,
        conn_sender: Sender<Connection>,
        msg_sender: Sender<String>,
    ) -> Self {
        let listener = TcpListener::bind(format!("{}:{}", ip, port))
            .await
            .expect("No client");
        Self {
            listener,
            conn_sender,
            msg_sender,
        }
    }

    async fn listen_for_conn(&self) -> (OwnedReadHalf, OwnedWriteHalf, SocketAddr) {
        let (stream, sockaddr) = self
            .listener
            .accept()
            .await
            .expect("Cannot accept connection");
        let (reader, writer) = stream.into_split();
        (reader, writer, sockaddr)
    }

    async fn push_conn(&self, sockaddr: SocketAddr, stream_writer: OwnedWriteHalf) {
        self.conn_sender
            .send(Connection::Push {
                sockaddr,
                stream_writer,
            })
            .await
            .expect("Cannot queue new connection");
    }

    async fn spawn_conn_task(&self, stream_reader: OwnedReadHalf, sockaddr: SocketAddr) {
        let msg_sender = self.msg_sender.clone();
        let conn_sender = self.conn_sender.clone();
        spawn(async move {
            if let Err(e) = handle_bytes(stream_reader, msg_sender, sockaddr, conn_sender).await {
                eprintln!("{}", e);
            };
        });
    }
}

async fn msg_task(
    ip: &str,
    port: u16,
    conn_sender: Sender<Connection>,
    msg_sender: Sender<String>,
) {
    let msg_handler = MsgHandler::new(ip, port, conn_sender, msg_sender).await;
    loop {
        let (stream_reader, stream_writer, sockaddr) = msg_handler.listen_for_conn().await;
        msg_handler.push_conn(sockaddr, stream_writer).await;
        msg_handler.spawn_conn_task(stream_reader, sockaddr).await;
    }
}

#[tokio::main]
async fn main() {
    let (conn_sender, conn_recv) = mpsc::channel(MAX_SIMULATANEOUS_INCOMING_CONNECTIONS);
    let (msg_sender, msg_recv) = mpsc::channel::<String>(MAX_CHANNEL_QUEUE_LEN);
    spawn(connections_task(conn_recv, msg_recv));
    msg_task("127.0.0.1", 60_000, conn_sender, msg_sender).await;
}

// TODO: make this function resturn the message or
// if the conn got closed and handle the messages
// outside of it
async fn handle_bytes(
    stream: OwnedReadHalf,
    sender: Sender<String>,
    sockaddr: SocketAddr,
    conn_sender: Sender<Connection>,
) -> std::io::Result<()> {
    let mut buf = [0; MAX_MSG_BYTES];
    loop {
        stream.readable().await?;
        // NOTE: consider using read_buf
        match stream.try_read(&mut buf) {
            Ok(0) => {
                conn_sender
                    .send(Connection::Pop(sockaddr))
                    .await
                    .expect("Cannot send pop conncetion request");
                break;
            }
            Ok(n) => {
                let msg = String::from_utf8_lossy(&buf[..n]);
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
