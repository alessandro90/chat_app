use async_chat::message::{ParsedMsg, SerializedMessage, MAX_MSG_LEN};
use std::{
    io::{self, ErrorKind, Read, Write},
    net::TcpStream,
    sync::mpsc::{channel, Receiver},
    thread::spawn,
    time::Duration,
};

pub struct Connection {
    stream: TcpStream,
    msg_receiver: Receiver<io::Result<ParsedMsg>>,
}

impl Connection {
    #[must_use]
    pub fn new(ip: &str, port: u16) -> io::Result<Self> {
        let (msg_sender, msg_receiver) = channel();
        let mut stream = TcpStream::connect(format!("{}:{}", ip, port))?;
        stream
            .set_write_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let stream_clone = stream.try_clone()?;
        enum State {
            ReadHeader,
            ReadPayload,
        }
        spawn(move || {
            let mut state = State::ReadHeader;
            let mut payload = vec![0; 256];
            loop {
                match state {
                    State::ReadHeader => {
                        let mut buf = [0; SerializedMessage::size_of_len()];
                        if let Err(e) = stream.read_exact(&mut buf) {
                            if let Err(_) = msg_sender.send(Err(e)) {}
                            break;
                        }
                        let size = u32::from_be_bytes(buf);
                        assert!(size <= SerializedMessage::size_of_len() as u32);
                        payload.resize(size as usize, 0);
                        buf.into_iter()
                            .enumerate()
                            .for_each(|(i, b)| payload[i] = b);
                        state = State::ReadPayload;
                    }
                    State::ReadPayload => {
                        // The message type
                        if let Err(e) = stream.read_exact(
                            &mut payload[SerializedMessage::size_of_len()
                                ..SerializedMessage::size_of_header()],
                        ) {
                            if let Err(_) = msg_sender.send(Err(e)) {}
                            break;
                        }
                        if let Err(e) =
                            stream.read_exact(&mut payload[SerializedMessage::size_of_header()..])
                        {
                            if let Err(_) = msg_sender.send(Err(e)) {}
                            break;
                        }
                        if let Some(msg) = ParsedMsg::from_bytes(&payload) {
                            if let Err(_) = msg_sender.send(Ok(msg)) {
                                break;
                            }
                            state = State::ReadHeader;
                            payload.clear();
                        } else {
                            break;
                        }
                    }
                };
            }
        });
        Ok(Self {
            stream: stream_clone,
            msg_receiver,
        })
    }

    #[must_use]
    pub fn split(self) -> (Writer, Reader) {
        (
            Writer {
                stream: self.stream,
            },
            Reader {
                msg_receiver: self.msg_receiver,
            },
        )
    }
}

pub struct Writer {
    stream: TcpStream,
}

impl Writer {
    // TODO: use a channel to queue several messages
    #[must_use]
    pub fn try_send_msg(&mut self, msg: &str) -> io::Result<()> {
        if msg.as_bytes().len() > MAX_MSG_LEN {
            return Err(io::Error::new(
                ErrorKind::Other,
                format!("Message too long. Max lenght in bytes is {}", MAX_MSG_LEN),
            ));
        }
        self.stream
            .write_all(SerializedMessage::from_string(msg).as_bytes())?;
        self.stream.flush()
    }
}

pub struct Reader {
    msg_receiver: Receiver<io::Result<ParsedMsg>>,
}

impl Reader {
    #[must_use]
    pub fn try_read_msg(&self) -> io::Result<ParsedMsg> {
        match self.msg_receiver.recv_timeout(Duration::from_millis(0)) {
            Ok(msg) => msg,
            Err(_) => Err(io::Error::new(
                ErrorKind::Other,
                "Cannot recv message from thread",
            )),
        }
    }
}
