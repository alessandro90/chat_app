type Size = u32;

#[derive(Debug)]
pub struct Message {
    size: Size,
    msg_type: MsgType,
    payload: Vec<u8>,
}

impl Message {
    #[must_use]
    const fn size_of_len() -> usize {
        std::mem::size_of::<Size>()
    }

    #[must_use]
    pub fn from_bytes(payload: String) -> Self {
        Self {
            size: (Self::size_of_len() + MsgType::size() + payload.len()) as u32,
            msg_type: MsgType::Text,
            payload: payload.as_bytes().to_vec(),
        }
    }

    #[must_use]
    pub fn from_number(n: u32) -> Self {
        Self {
            size: (Self::size_of_len() + MsgType::size() + std::mem::size_of_val(&n)) as u32,
            msg_type: MsgType::Num,
            payload: u32_to_bytes_iter(n.to_be()).collect(),
        }
    }

    #[must_use]
    pub fn serialize(self) -> Vec<u8> {
        let size = self.size.to_be();
        u32_to_bytes_iter(size)
            .chain([self.msg_type as u8].into_iter())
            .chain(self.payload.into_iter())
            .collect()
    }
}

#[derive(Debug)]
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

pub enum ParsedMsg {
    Num(u32),
    Text(String),
}

impl ParsedMsg {
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        assert!(bytes.len() >= std::mem::size_of::<Size>() + MsgType::size());
        let msg_type: MsgType = bytes[Message::size_of_len()]
            .try_into()
            .expect("Invalid msg type");
        match msg_type {
            MsgType::Num => {
                assert!(
                    bytes.len()
                        == Message::size_of_len() + MsgType::size() + std::mem::size_of::<u32>()
                );
                let mut it = bytes.iter().skip(Message::size_of_len() + MsgType::size());
                // This is guaranteed by the above assert
                let a = *unsafe { it.next().unwrap_unchecked() };
                let b = *unsafe { it.next().unwrap_unchecked() };
                let c = *unsafe { it.next().unwrap_unchecked() };
                let d = *unsafe { it.next().unwrap_unchecked() };
                Self::Num(u32::from_be_bytes([a, b, c, d]))
            }
            MsgType::Text => {
                assert!(bytes.len() > Message::size_of_len() + MsgType::size());
                Self::Text(
                    String::from_utf8_lossy(&bytes[Message::size_of_len() + MsgType::size()..])
                        .to_string(),
                )
            }
        }
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
        let msg = Message::from_bytes(s.clone());
        let bytes = msg.serialize();
        let parsed = ParsedMsg::from_bytes(&bytes);
        match parsed {
            ParsedMsg::Num(_) => assert!(false),
            ParsedMsg::Text(txt) => assert_eq!(txt, s),
        };
    }

    #[test]
    fn num_test() {
        let n = 11u32;
        let msg = Message::from_number(n);
        let bytes = msg.serialize();
        let parsed = ParsedMsg::from_bytes(&bytes);
        match parsed {
            ParsedMsg::Num(m) => assert_eq!(n, m),
            ParsedMsg::Text(_) => assert!(false),
        };
    }
}
