use async_chat::message::{Cmd, InfoKind, ParsedMsg, SerializedMessage, MAX_MSG_LEN};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
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
const MAX_CHANNEL_QUEUE_LEN: usize = 256;
const MAX_SIMULATANEOUS_INCOMING_CONNECTIONS: usize = 32;
const SERVER_INFO_HEADER: &str = "SERVER.INFO: ";
const MAX_CONNECTIONS: usize = 100;
const SERVER_PORT: u16 = 60_000;
const SERVER_LISTEN_IP: &str = "0.0.0.0";
const READ_TIMEOUT_MS: Duration = Duration::from_millis(1_000);

const HELP_STRING: &str = //
    r"1. /help -> Get this message
    2. /count -> Current number of connectet users";

enum Connection {
    Push {
        sockaddr: SocketAddr,
        stream_writer: OwnedWriteHalf,
    },
    Pop(SocketAddr),
}

struct Entry {
    writer_stream: Arc<Mutex<OwnedWriteHalf>>,
}

impl Entry {
    fn new(stream: OwnedWriteHalf) -> Self {
        Self {
            writer_stream: Arc::new(Mutex::new(stream)),
        }
    }

    async fn close(&mut self) {
        let mut stream = self.writer_stream.lock().await;
        if let Err(e) = stream.shutdown().await {
            println!("Cannot shutdown stream: {}", e);
        }
    }

    fn get_weak_stream(&self) -> WeakEntry {
        WeakEntry {
            stream: Arc::downgrade(&self.writer_stream),
        }
    }

    async fn write_all<F>(&self, f: F)
    where
        F: FnOnce() -> SerializedMessage,
    {
        write_all(&self.writer_stream, f).await;
    }
}

struct WeakEntry {
    stream: Weak<Mutex<OwnedWriteHalf>>,
}

impl WeakEntry {
    async fn write_all<F>(&self, f: F)
    where
        F: FnOnce() -> SerializedMessage,
    {
        if let Some(stream) = self.stream.upgrade() {
            write_all(&stream, f).await;
        }
    }
}

async fn write_all<F>(stream: &Mutex<OwnedWriteHalf>, f: F)
where
    F: FnOnce() -> SerializedMessage,
{
    let mut lock_stream = stream.lock().await;
    if let Ok(()) = lock_stream.writable().await {
        lock_stream
            .write_all(f().as_bytes())
            .await
            .expect("Cannot write to stream");
    }
}

#[derive(Default)]
struct Connections {
    // TODO: Encapsulate Arc<Mutex<OwnedWriteHalf>> in own struct
    entries: HashMap<SocketAddr, Entry>,
}

impl Connections {
    async fn handle_conn(&mut self, conn: Connection) {
        match conn {
            Connection::Push {
                sockaddr,
                stream_writer,
            } => {
                println!("added connection: {}", sockaddr);
                let _ = self.entries.insert(sockaddr, Entry::new(stream_writer));
                if self.entries.len() >= MAX_CONNECTIONS {
                    self.send_info_msg(sockaddr, InfoKind::ServerFull);
                }
            }
            Connection::Pop(sockaddr) => {
                println!("removed connection: {}", sockaddr);
                let stream = self.entries.remove(&sockaddr);
                if let Some(mut stream) = stream {
                    stream.close().await;
                }
            }
        };
    }

    fn send_count_to_user(&self, sockaddr: SocketAddr) {
        if let Some(entry) = self.entries.get(&sockaddr).map(Entry::get_weak_stream) {
            let user_count = self.entries.len() as u32;
            spawn(async move {
                entry
                    .write_all(|| SerializedMessage::from_user_count(user_count))
                    .await;
            });
        }
    }

    fn send_help_to_user(&self, sockaddr: SocketAddr) {
        if let Some(entry) = self.entries.get(&sockaddr).map(Entry::get_weak_stream) {
            spawn(async move {
                entry
                    .write_all(|| SerializedMessage::from_help_string(HELP_STRING))
                    .await;
            });
        }
    }

    fn broadcast_msg(&self, txt: String, sockaddr: SocketAddr) {
        for (key, entry) in self.entries.iter().map(|(k, v)| (k, v.get_weak_stream())) {
            let txt = txt.clone();
            let key = key.clone();
            spawn(async move {
                entry
                    .write_all(|| {
                        let prefix = if key == sockaddr {
                            "You".to_string()
                        } else {
                            sockaddr.to_string()
                        };
                        SerializedMessage::from_string(&format!("{}: {}", prefix, txt))
                    })
                    .await;
            });
        }
    }

    fn send_info_msg(&mut self, sockaddr: SocketAddr, info_kind: InfoKind) {
        match info_kind {
            InfoKind::MessageTooLong => {
                if let Some(entry) = self.entries.get(&sockaddr).map(Entry::get_weak_stream) {
                    spawn(async move {
                        entry.write_all(|| {
                                let msg = format!(
                                    "{}Your message is too long. Maximum allowed lenght in bytes is {}",
                                    SERVER_INFO_HEADER, MAX_MSG_LEN
                                );
                                SerializedMessage::from_string(&msg)
                            })
                            .await;
                    });
                }
            }
            InfoKind::ServerFull => {
                if let Some(entry) = self.entries.remove(&sockaddr) {
                    spawn(async move {
                        entry.write_all(|| {
                            let msg = format!(
                                "{}Server has reached max number of connections {}. Refusing the connection.",
                                SERVER_INFO_HEADER,
                                MAX_CONNECTIONS
                            );
                            SerializedMessage::from_string(&msg)
                        }).await;
                    });
                }
            }
        }
    }

    fn handle_message(&mut self, conn_msg: ConnMsg) {
        let ConnMsg { msg, sockaddr } = conn_msg;
        match msg {
            ParsedMsg::UserCount(_) | ParsedMsg::Help(_) => (), // Clients cannot send these
            ParsedMsg::Command(cmd) => match cmd {
                Cmd::UserCount => self.send_count_to_user(sockaddr),
                Cmd::Help => self.send_help_to_user(sockaddr),
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
                    connections.handle_conn(conn).await;
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
                    ParseError::InvalidMsg => eprintln!("Invalid Msg: {:?}", parse_error),
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
}

macro_rules! or_close {
    ($stream:expr, $sockaddr:expr, $method:ident, with_timeout) => {
        match tokio::time::timeout(READ_TIMEOUT_MS, $stream.$method()).await {
            Ok(res) => res.map_err(|_| ParseError::ConnClosed($sockaddr)),
            Err(_) => Err(ParseError::ConnClosed($sockaddr)),
        }
    };
    ($stream:expr, $sockaddr:expr, $method:ident, $arg:expr, with_timeout) => {
        match tokio::time::timeout(READ_TIMEOUT_MS, $stream.$method($arg)).await {
            Ok(res) => res.map_err(|_| ParseError::ConnClosed($sockaddr)),
            Err(_) => Err(ParseError::ConnClosed($sockaddr)),
        }
    };
    ($stream:expr, $sockaddr:expr, $method:ident) => {
        $stream
            .$method()
            .await
            .map_err(|_| ParseError::ConnClosed($sockaddr))
    };
    ($stream:expr, $sockaddr:expr, $method:ident, $arg:expr) => {
        $stream
            .$method($arg)
            .await
            .map_err(|_| ParseError::ConnClosed($sockaddr))
    };
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
        or_close!(stream, sockaddr, readable)?;
        match state {
            State::ReadHeader => {
                size = or_close!(stream, sockaddr, read_u32)?;
                let msg_type = or_close!(stream, sockaddr, read_u8, with_timeout)?;
                if size > MAX_MSG_LEN as u32 {
                    sender
                        .send(ConnMsg {
                            sockaddr,
                            msg: ParsedMsg::from_info(InfoKind::MessageTooLong),
                        })
                        .await
                        .expect("Cannot send reply");
                    state =
                        State::DiscardMessage(size as usize - SerializedMessage::size_of_header());
                    buf.resize(256, 0);
                } else if size <= SerializedMessage::size_of_header() as u32 {
                    // This message is malformed for some reason
                    // TODO: log it
                    buf.clear();
                    size = 0;
                    state = State::ReadHeader;
                } else {
                    size.to_be_bytes().into_iter().for_each(|b| buf.push(b));
                    buf.push(msg_type);
                    buf.resize(size as usize, 0);
                    state = State::ReadPayload;
                }
            }
            State::ReadPayload => {
                let _ = or_close!(
                    stream,
                    sockaddr,
                    read_exact,
                    &mut buf[SerializedMessage::size_of_header()..],
                    with_timeout
                )?;
                let msg = ParsedMsg::from_bytes(&buf[..size as usize])
                    .ok_or_else(|| ParseError::InvalidMsg)?;
                if let ParsedMsg::Info(ref i) = msg {
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

    const SERVER_IP: &str = "127.0.0.1";

    use super::*;

    #[tokio::test]
    async fn test_simple_msg() {
        let port = 60_001;
        spawn(run_server(port));
        sleep(Duration::from_millis(500)).await;

        let mut client = TcpStream::connect(format!("{}:{}", SERVER_IP, port))
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

        let mut client = TcpStream::connect(format!("{}:{}", SERVER_IP, port))
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
            let mut client = TcpStream::connect(format!("{}:{}", SERVER_IP, port))
                .await
                .expect("Cannot connect to server");
            sleep(Duration::from_millis(1000)).await;
            let msg = SerializedMessage::from_string("Hello I am a client!");

            client.writable().await.unwrap();
            client
                .write_all(msg.as_bytes())
                .await
                .expect("Cannot send message");
        });
        let mut client = TcpStream::connect(format!("{}:{}", SERVER_IP, port))
            .await
            .expect("Cannot connect to server");

        let mut v = vec![];
        client.readable().await.unwrap();
        println!("Readable");
        let read_bytes = client.read_buf(&mut v).await.expect("Cannot read bytes");
        println!("Bytes received: {}", read_bytes);
        print!("Message is: ");
        println!("{}", String::from_utf8_lossy(&v));
    }

    #[tokio::test]
    async fn test_ask_count() {
        let port = 60_004;
        spawn(run_server(port));
        sleep(Duration::from_millis(500)).await;

        let mut client = TcpStream::connect(format!("{}:{}", SERVER_IP, port))
            .await
            .expect("Cannot connect to server");
        let msg = SerializedMessage::from_string("/count");

        client.writable().await.unwrap();
        client
            .write_all(msg.as_bytes())
            .await
            .expect("Cannot send message");
        let mut v = vec![];
        client.readable().await.unwrap();
        let read_bytes = client.read_buf(&mut v).await.expect("Cannot read bytes");
        println!("Bytes received: {}", read_bytes);
        let msg = ParsedMsg::from_bytes(&v).expect("Fail to parse message");
        let ParsedMsg::UserCount(n) = msg else {
            panic!("Invalid msg");
        };
        assert_eq!(1, n);
    }
}
