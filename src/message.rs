type Size = u32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializedMessage(Vec<u8>);

impl SerializedMessage {
    #[must_use]
    pub const fn size_of_len() -> usize {
        std::mem::size_of::<Size>()
    }

    #[must_use]
    pub fn from_string(payload: &str) -> Self {
        let size = (Self::size_of_len() + MsgType::size() + payload.len()) as u32;
        let msg_type = MsgType::Text;
        Self(serialize(
            size,
            msg_type,
            payload.as_bytes().into_iter().cloned(),
        ))
    }

    #[must_use]
    pub fn from_number(n: u32) -> Self {
        let size = (Self::size_of_len() + MsgType::size() + std::mem::size_of_val(&n)) as u32;
        let msg_type = MsgType::Num;
        Self(serialize(size, msg_type, u32_to_bytes_iter(n.to_be())))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl From<SerializedMessage> for Vec<u8> {
    fn from(value: SerializedMessage) -> Self {
        value.0
    }
}

#[must_use]
fn serialize(size: u32, msg_type: MsgType, payload: impl Iterator<Item = u8>) -> Vec<u8> {
    let size = size.to_be();
    u32_to_bytes_iter(size)
        .chain([msg_type as u8].into_iter())
        .chain(payload)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    Text = 0,
    Num = 1,
}

impl MsgType {
    #[must_use]
    const fn size() -> usize {
        std::mem::size_of::<Self>()
    }
}

impl TryInto<MsgType> for u8 {
    type Error = ();
    fn try_into(self) -> Result<MsgType, Self::Error> {
        match self {
            0 => Ok(MsgType::Text),
            1 => Ok(MsgType::Num),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum Cmd {
    UserCount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum InfoKind {
    MessageTooLong,
    ServerFull,
}

// NOTE: Should I create 2 message types, one for the server and one for the client?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedMsg {
    Num(u32),
    Text(String),
    Command(Cmd),
    Info(InfoKind),
}

impl ParsedMsg {
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let msg_type: MsgType = bytes
            .get(SerializedMessage::size_of_len())?
            .clone()
            .try_into()
            .ok()?;
        match msg_type {
            MsgType::Num => {
                let mut it = bytes
                    .iter()
                    .skip(SerializedMessage::size_of_len() + MsgType::size());
                let a = *it.next()?;
                let b = *it.next()?;
                let c = *it.next()?;
                let d = *it.next()?;
                if it.next().is_some() {
                    return None;
                }
                Some(Self::Num(u32::from_be_bytes([a, b, c, d])))
            }
            MsgType::Text => {
                let text = String::from_utf8_lossy(
                    bytes.get(SerializedMessage::size_of_len() + MsgType::size()..)?,
                );

                match text.as_ref().trim_end() {
                    "/count" => Some(Self::Command(Cmd::UserCount)),
                    _ => Some(Self::Text(text.to_string())),
                }
            }
        }
    }

    #[must_use]
    pub fn from_info(info_kind: InfoKind) -> Self {
        Self::Info(info_kind)
    }
}

#[must_use]
fn u32_to_bytes_iter(n: u32) -> impl Iterator<Item = u8> {
    (0..std::mem::size_of_val(&n)).map(move |i| ((n >> (i * 8)) & 0xFF) as u8)
}

#[cfg(test)]
mod message_tests {
    use super::*;

    #[test]
    fn num_to_bytes_test() {
        let x = 112u32;
        let v: Vec<_> = u32_to_bytes_iter(x).collect();
        assert_eq!(v[0] as u32, x & 0xFF);
        assert_eq!(v[1] as u32, (x >> (1 * 8)) & 0xFF);
        assert_eq!(v[2] as u32, (x >> (2 * 8)) & 0xFF);
        assert_eq!(v[3] as u32, (x >> (3 * 8)) & 0xFF);
    }

    #[test]
    fn text_test() {
        let s = "Hello, World!".to_owned();
        let msg = SerializedMessage::from_string(&s);
        let parsed = ParsedMsg::from_bytes(msg.as_bytes()).unwrap();
        match parsed {
            ParsedMsg::Num(_) => assert!(false),
            ParsedMsg::Text(txt) => assert_eq!(txt, s),
            ParsedMsg::Command(_) => assert!(false),
            ParsedMsg::Info(_) => assert!(false),
        };
    }

    #[test]
    fn num_test() {
        let n = 11u32;
        let msg = SerializedMessage::from_number(n);
        let parsed = ParsedMsg::from_bytes(msg.as_bytes()).unwrap();
        match parsed {
            ParsedMsg::Num(m) => assert_eq!(n, m),
            ParsedMsg::Text(_) => assert!(false),
            ParsedMsg::Command(_) => assert!(false),
            ParsedMsg::Info(_) => assert!(false),
        };
    }

    #[test]
    fn cmd_test() {
        let msg = SerializedMessage::from_string("/count");
        let parsed = ParsedMsg::from_bytes(msg.as_bytes()).unwrap();
        match parsed {
            ParsedMsg::Num(_) => assert!(false),
            ParsedMsg::Text(_) => assert!(false),
            ParsedMsg::Command(cmd) => assert_eq!(cmd, Cmd::UserCount),
            ParsedMsg::Info(_) => assert!(false),
        };
    }
}