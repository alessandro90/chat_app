use async_chat::message::{Cmd, InfoKind, ParsedMsg, SerializedMessage};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::{
    io::AsyncReadExt,
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

const RESERVED_MSG_LEN: usize = 512;
const MAX_MSG_LEN: usize = 5 * 1024;
const MAX_CHANNEL_QUEUE_LEN: usize = 256;
const MAX_SIMULATANEOUS_INCOMING_CONNECTIONS: usize = 32;
const SERVER_INFO_HEADER: &str = "SERVER.INFO: ";
const MAX_CONNECTIONS: usize = 100;
const SERVER_PORT: u16 = 60_000;
const SERVER_LISTEN_IP: &str = "127.0.0.1";

enum Connection {
    Push {
        sockaddr: SocketAddr,
        stream_writer: OwnedWriteHalf,
    },
    Pop(SocketAddr),
}

struct Connections {
    // TODO: Encapsulate Arc<Mutex<OwnedWriteHalf>> in own struct
    entries: HashMap<SocketAddr, Arc<Mutex<OwnedWriteHalf>>>,
}

impl Default for Connections {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl Connections {
    fn handle_conn(&mut self, conn: Connection) {
        match conn {
            Connection::Push {
                sockaddr,
                stream_writer,
            } => {
                println!("added connection: {}", sockaddr);
                let _ = self
                    .entries
                    .insert(sockaddr, Arc::new(Mutex::new(stream_writer)));
                if self.entries.len() >= MAX_CONNECTIONS {
                    self.send_info_msg(sockaddr, InfoKind::ServerFull);
                }
            }
            Connection::Pop(ref sockaddr) => {
                println!("removed connection: {}", sockaddr);
                let _ = self.entries.remove(sockaddr);
            }
        };
    }

    fn send_count_to_user(&self, sockaddr: SocketAddr) {
        if let Some(stream) = self.entries.get(&sockaddr).map(Arc::downgrade) {
            let user_count = self.entries.len() as u32;
            spawn(async move {
                if let Some(stream) = stream.upgrade() {
                    let lock_stream = stream.lock().await;
                    if let Ok(()) = lock_stream.writable().await {
                        lock_stream
                            .try_write(SerializedMessage::from_number(user_count).as_bytes())
                            .expect("Cannot write to stream");
                    }
                }
            });
        }
    }

    fn broadcast_msg(&self, txt: String, sockaddr: SocketAddr) {
        for (key, stream) in self.entries.iter().map(|(k, v)| (k, Arc::downgrade(v))) {
            let txt = txt.clone();
            let key = key.clone();
            spawn(async move {
                if let Some(stream) = stream.upgrade() {
                    let lock_stream = stream.lock().await;
                    if let Ok(()) = lock_stream.writable().await {
                        let prefix = if key == sockaddr {
                            "You".to_string()
                        } else {
                            sockaddr.to_string()
                        };
                        lock_stream
                            .try_write(format!("{}: {}", prefix, txt).as_bytes())
                            .expect("Cannot write to stream");
                    }
                }
            });
        }
    }

    fn send_info_msg(&mut self, sockaddr: SocketAddr, info_kind: InfoKind) {
        match info_kind {
            InfoKind::MessageTooLong => {
                if let Some(stream) = self.entries.get(&sockaddr).map(Arc::downgrade) {
                    spawn(async move {
                        if let Some(stream) = stream.upgrade() {
                            let lock_stream = stream.lock().await;
                            if let Ok(()) = lock_stream.writable().await {
                                let msg = format!(
                                        "{}Your message is too long. Maximum allowed lenght in bytes is {}",
                                        SERVER_INFO_HEADER,
                                        MAX_MSG_LEN
                                    );
                                lock_stream
                                    .try_write(SerializedMessage::from_string(&msg).as_bytes())
                                    .expect("Cannot write to stream");
                            }
                        }
                    });
                }
            }
            InfoKind::ServerFull => {
                if let Some(stream) = self.entries.remove(&sockaddr) {
                    spawn(async move {
                        let stream = stream.lock().await;
                        if let Ok(()) = stream.writable().await {
                            let msg = format!(
                                "{}Server has reached max number of connections {}. Refusing the connection.",
                                SERVER_INFO_HEADER, MAX_CONNECTIONS
                            );
                            stream
                                .try_write(SerializedMessage::from_string(&msg).as_bytes())
                                .expect("Cannot write to stream");
                        }
                    });
                }
            }
        }
    }

    fn handle_message(&mut self, conn_msg: ConnMsg) {
        let ConnMsg { msg, sockaddr } = conn_msg;
        match msg {
            ParsedMsg::Num(_) => (),
            ParsedMsg::Command(cmd) => match cmd {
                Cmd::UserCount => self.send_count_to_user(sockaddr),
            },
            ParsedMsg::Text(txt) => self.broadcast_msg(txt, sockaddr),
            ParsedMsg::Info(info_kind) => self.send_info_msg(sockaddr, info_kind),
        };
    }
}

async fn connections_task(
    mut conn_recv: Receiver<Connection>,
    mut msg_recv: Receiver<ConnMsg>,
) -> ! {
    let mut connections = Connections::default();
    loop {
        tokio::select! {
            conn = conn_recv.recv() => {
                if let Some(conn) = conn {
                    connections.handle_conn(conn);
                }
            },
            msg = msg_recv.recv() => {
                if let Some(msg) = msg {
                    connections.handle_message(msg);
                }
            }
        }
    }
}

struct Server {
    listener: TcpListener,
    conn_sender: Sender<Connection>,
    msg_sender: Sender<ConnMsg>,
}

impl Server {
    async fn new(
        ip: &str,
        port: u16,
        conn_sender: Sender<Connection>,
        msg_sender: Sender<ConnMsg>,
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
            if let Err(parse_error) = parse_messages(stream_reader, msg_sender, sockaddr).await {
                match parse_error {
                    ParseError::ConnClosed(conn) => {
                        conn_sender
                            .send(Connection::Pop(conn))
                            .await
                            .expect("Cannot send pop conncetion request");
                    }
                    e @ (ParseError::Unknown(_) | ParseError::InvalidMsg) => eprintln!("{:?}", e),
                }
            };
        });
    }
}

async fn msg_task(
    ip: &str,
    port: u16,
    conn_sender: Sender<Connection>,
    msg_sender: Sender<ConnMsg>,
) -> ! {
    let msg_handler = Server::new(ip, port, conn_sender, msg_sender).await;
    loop {
        let (stream_reader, stream_writer, sockaddr) = msg_handler.listen_for_conn().await;
        msg_handler.push_conn(sockaddr, stream_writer).await;
        msg_handler.spawn_conn_task(stream_reader, sockaddr).await;
    }
}

struct ConnMsg {
    sockaddr: SocketAddr,
    msg: ParsedMsg,
}

async fn run_server(port: u16) {
    let (conn_sender, conn_recv) = mpsc::channel(MAX_SIMULATANEOUS_INCOMING_CONNECTIONS);
    let (msg_sender, msg_recv) = mpsc::channel::<ConnMsg>(MAX_CHANNEL_QUEUE_LEN);
    spawn(connections_task(conn_recv, msg_recv));
    msg_task(SERVER_LISTEN_IP, port, conn_sender, msg_sender).await;
}

#[tokio::main]
async fn main() {
    run_server(SERVER_PORT).await;
}

#[derive(Debug)]
enum ParseError {
    ConnClosed(SocketAddr),
    InvalidMsg,
    #[allow(dead_code)]
    Unknown(String),
}

async fn parse_messages(
    mut stream: OwnedReadHalf,
    sender: Sender<ConnMsg>,
    sockaddr: SocketAddr,
) -> Result<(), ParseError> {
    enum State {
        ReadHeader,
        ReadPayload,
        DiscardMessage(usize),
    }
    let mut state = State::ReadHeader;
    let mut buf = Vec::with_capacity(RESERVED_MSG_LEN);
    let mut size = 0;
    loop {
        stream
            .readable()
            .await
            .map_err(|e| ParseError::Unknown(e.to_string()))?;
        match state {
            State::ReadHeader => {
                size = stream
                    .read_u32()
                    .await
                    .map_err(|e| ParseError::Unknown(e.to_string()))?;
                let msg_type = stream
                    .read_u8()
                    .await
                    .map_err(|e| ParseError::Unknown(e.to_string()))?;
                if size > MAX_MSG_LEN as u32 {
                    sender
                        .send(ConnMsg {
                            sockaddr,
                            msg: ParsedMsg::from_info(InfoKind::MessageTooLong),
                        })
                        .await
                        .expect("Cannot send reply");
                    state = State::DiscardMessage(size as usize - 5);
                    buf.resize(256, 0);
                } else if size > SerializedMessage::size_of_len() as u32 + 1 {
                    size.to_be_bytes().into_iter().for_each(|b| buf.push(b));
                    buf.push(msg_type);
                    buf.resize(size as usize, 0);
                    state = State::ReadPayload;
                }
            }
            State::ReadPayload => match stream.read_exact(&mut buf[5..]).await {
                Ok(_) => {
                    let msg = ParsedMsg::from_bytes(&buf[..size as usize])
                        .ok_or_else(|| ParseError::InvalidMsg)?;
                    if let ParsedMsg::Info(i) = &msg {
                        println!(
                            "Invalid message of type INFO from client: {:?}. Ignoring.",
                            i
                        );
                    } else {
                        buf.clear();
                        size = 0;
                        state = State::ReadHeader;
                        sender
                            .send(ConnMsg { sockaddr, msg })
                            .await
                            .expect("Cannot send reply");
                    }
                }
                Err(_) => {
                    return Err(ParseError::ConnClosed(sockaddr));
                }
            },
            State::DiscardMessage(to_discard) => match stream.read_exact(&mut buf).await {
                Ok(bytes) => {
                    if bytes == to_discard {
                        buf.clear();
                        size = 0;
                        state = State::ReadHeader;
                    } else {
                        state = State::DiscardMessage(to_discard - bytes);
                    }
                }
                Err(_) => {
                    return Err(ParseError::ConnClosed(sockaddr));
                }
            },
        }
    }
}

#[cfg(test)]
mod server_tests {
    use std::time::Duration;
    use tokio::{io::AsyncWriteExt, net::TcpStream, time::sleep};

    use super::*;

    #[tokio::test]
    async fn test_simple_msg() {
        let port = 60_001;
        spawn(run_server(port));
        sleep(Duration::from_millis(500)).await;

        let mut client = TcpStream::connect(format!("{}:{}", SERVER_LISTEN_IP, port))
            .await
            .expect("Cannot connect to server");
        let msg = SerializedMessage::from_string("Hello I am a client!");

        client.writable().await.unwrap();
        client
            .write_all(msg.as_bytes())
            .await
            .expect("Cannot send message");
        let mut v = vec![];
        client.readable().await.unwrap();
        let read_bytes = client.read_buf(&mut v).await.expect("Cannot read bytes");
        println!("Bytes received: {}", read_bytes);
        print!("Message is: ");
        println!("{}", String::from_utf8_lossy(&v));
    }

    #[tokio::test]
    async fn test_message_too_long() {
        let port = 60_003;
        spawn(run_server(port));
        sleep(Duration::from_millis(500)).await;

        let mut client = TcpStream::connect(format!("{}:{}", SERVER_LISTEN_IP, port))
            .await
            .expect("Cannot connect to server");
        let s = (0..MAX_MSG_LEN + 1).map(|_| 'a').collect::<String>();
        let msg = SerializedMessage::from_string(&s);

        client.writable().await.unwrap();
        client
            .write_all(msg.as_bytes())
            .await
            .expect("Cannot send message");
        let mut v = vec![];
        client.readable().await.unwrap();
        let read_bytes = client.read_buf(&mut v).await.expect("Cannot read bytes");
        println!("Bytes received: {}", read_bytes);
        print!("Message is: ");
        println!("{}", String::from_utf8_lossy(&v));
    }

    #[tokio::test]
    async fn test_multi_conn() {
        let port = 60_002;
        spawn(run_server(port));
        sleep(Duration::from_millis(500)).await;

        spawn(async move {
            let mut client = TcpStream::connect(format!("{}:{}", SERVER_LISTEN_IP, port))
                .await
                .expect("Cannot connect to server");
            sleep(Duration::from_millis(1_000)).await;
            let msg = SerializedMessage::from_string("Hello I am a client!");

            client.writable().await.unwrap();
            client
                .write_all(msg.as_bytes())
                .await
                .expect("Cannot send message");
        });
        let mut client = TcpStream::connect(format!("{}:{}", SERVER_LISTEN_IP, port))
            .await
            .expect("Cannot connect to server");
        sleep(Duration::from_millis(500)).await;

        let mut v = vec![];
        client.readable().await.unwrap();
        let read_bytes = client.read_buf(&mut v).await.expect("Cannot read bytes");
        println!("Bytes received: {}", read_bytes);
        print!("Message is: ");
        println!("{}", String::from_utf8_lossy(&v));
    }
}
