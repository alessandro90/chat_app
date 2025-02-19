type Size = u32;

pub const MAX_MSG_LEN: usize = 5 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializedMessage(Vec<u8>);

impl SerializedMessage {
    #[must_use]
    pub const fn size_of_len() -> usize {
        std::mem::size_of::<Size>()
    }

    #[must_use]
    pub const fn size_of_header() -> usize {
        Self::size_of_len() + MsgType::size()
    }

    #[must_use]
    pub fn from_string_generic(payload: &str, msg_type: MsgType) -> Self {
        let size = (Self::size_of_header() + payload.len()) as u32;
        Self(serialize(
            size,
            msg_type,
            payload.as_bytes().into_iter().cloned(),
        ))
    }

    #[must_use]
    pub fn from_string(payload: &str) -> Self {
        Self::from_string_generic(payload, MsgType::Text)
    }

    #[must_use]
    pub fn from_help_string(payload: &str) -> Self {
        Self::from_string_generic(payload, MsgType::Help)
    }

    #[must_use]
    pub fn from_user_count(n: u32) -> Self {
        let size = (Self::size_of_header() + std::mem::size_of_val(&n)) as u32;
        let msg_type = MsgType::UserCount;
        Self(serialize(size, msg_type, n.to_be_bytes().into_iter()))
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
    size.to_be_bytes()
        .into_iter()
        .chain([msg_type as u8].into_iter())
        .chain(payload)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    Text = 0,
    UserCount = 1,
    Help = 2,
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
            1 => Ok(MsgType::UserCount),
            2 => Ok(MsgType::Help),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum Cmd {
    UserCount,
    Help,
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
    UserCount(u32),
    Text(String),
    Command(Cmd),
    Info(InfoKind),
    Help(String),
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
            MsgType::Help => {
                let text =
                    String::from_utf8_lossy(bytes.get(SerializedMessage::size_of_header()..)?);
                Some(Self::Help(text.to_string()))
            }
            MsgType::UserCount => {
                let mut it = bytes.iter().skip(SerializedMessage::size_of_header());
                let a = *it.next()?;
                let b = *it.next()?;
                let c = *it.next()?;
                let d = *it.next()?;
                if it.next().is_some() {
                    return None;
                }
                Some(Self::UserCount(u32::from_be_bytes([a, b, c, d])))
            }
            MsgType::Text => {
                let text =
                    String::from_utf8_lossy(bytes.get(SerializedMessage::size_of_header()..)?);

                match text.as_ref().trim_end() {
                    "/count" => Some(Self::Command(Cmd::UserCount)),
                    "/help" => Some(Self::Command(Cmd::Help)),
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

#[cfg(test)]
mod message_tests {
    use super::*;

    #[test]
    fn text_test() {
        let s = "Hello, World!".to_owned();
        let msg = SerializedMessage::from_string(&s);
        let parsed = ParsedMsg::from_bytes(msg.as_bytes()).unwrap();
        match parsed {
            ParsedMsg::UserCount(_) => assert!(false),
            ParsedMsg::Text(txt) => assert_eq!(txt, s),
            ParsedMsg::Command(_) => assert!(false),
            ParsedMsg::Info(_) => assert!(false),
            ParsedMsg::Help(_) => assert!(false),
        };
    }

    #[test]
    fn num_test() {
        let n = 11u32;
        let msg = SerializedMessage::from_user_count(n);
        let parsed = ParsedMsg::from_bytes(msg.as_bytes()).unwrap();
        match parsed {
            ParsedMsg::UserCount(m) => assert_eq!(n, m),
            ParsedMsg::Text(_) => assert!(false),
            ParsedMsg::Command(_) => assert!(false),
            ParsedMsg::Info(_) => assert!(false),
            ParsedMsg::Help(_) => assert!(false),
        };
    }

    #[test]
    fn cmd_test() {
        let msg = SerializedMessage::from_string("/count");
        let parsed = ParsedMsg::from_bytes(msg.as_bytes()).unwrap();
        match parsed {
            ParsedMsg::UserCount(_) => assert!(false),
            ParsedMsg::Text(_) => assert!(false),
            ParsedMsg::Command(cmd) => assert_eq!(cmd, Cmd::UserCount),
            ParsedMsg::Info(_) => assert!(false),
            ParsedMsg::Help(_) => assert!(false),
        };
    }
}
