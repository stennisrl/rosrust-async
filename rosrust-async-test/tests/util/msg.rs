// Normally these would be generated using rosrust_msg or the rosmsg_include macro, but these are hard-coded to simplify testing.

use rosrust::{Message, RosMsg, ServicePair};

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RosString {
    pub data: String,
}

impl Message for RosString {
    #[inline]
    fn msg_definition() -> String {
        "string data\n".into()
    }
    #[inline]
    fn md5sum() -> String {
        "992ce8a1687cec8c8bd883ec73ca41d1".into()
    }
    #[inline]
    fn msg_type() -> String {
        "std_msgs/String".into()
    }
}
impl RosMsg for RosString {
    fn encode<W: std::io::Write>(&self, mut w: W) -> std::io::Result<()> {
        self.data.encode(w.by_ref())?;
        Ok(())
    }
    fn decode<R: std::io::Read>(mut r: R) -> std::io::Result<Self> {
        Ok(Self {
            data: RosMsg::decode(r.by_ref())?,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TwoIntsRes {
    pub sum: i64,
}

impl Message for TwoIntsRes {
    #[inline]
    fn msg_definition() -> String {
        "int64 sum\n".into()
    }
    #[inline]
    fn md5sum() -> String {
        "b88405221c77b1878a3cbbfff53428d7".into()
    }
    #[inline]
    fn msg_type() -> String {
        "test_msgs/TwoIntsRes".into()
    }
}
impl RosMsg for TwoIntsRes {
    fn encode<W: std::io::Write>(&self, mut w: W) -> std::io::Result<()> {
        self.sum.encode(w.by_ref())?;
        Ok(())
    }
    fn decode<R: std::io::Read>(mut r: R) -> std::io::Result<Self> {
        Ok(Self {
            sum: RosMsg::decode(r.by_ref())?,
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TwoIntsReq {
    pub a: i64,
    pub b: i64,
}

impl Message for TwoIntsReq {
    #[inline]
    fn msg_definition() -> String {
        "int64 a\nint64 b\n".into()
    }
    #[inline]
    fn md5sum() -> String {
        "36d09b846be0b371c5f190354dd3153e".into()
    }
    #[inline]
    fn msg_type() -> String {
        "test_msgs/TwoIntsReq".into()
    }
}

impl RosMsg for TwoIntsReq {
    fn encode<W: std::io::Write>(&self, mut w: W) -> std::io::Result<()> {
        self.a.encode(w.by_ref())?;
        self.b.encode(w.by_ref())?;
        Ok(())
    }
    fn decode<R: std::io::Read>(mut r: R) -> std::io::Result<Self> {
        Ok(Self {
            a: RosMsg::decode(r.by_ref())?,
            b: RosMsg::decode(r.by_ref())?,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TwoInts;

impl Message for TwoInts {
    #[inline]
    fn msg_definition() -> String {
        String::new()
    }
    #[inline]
    fn md5sum() -> String {
        "6a2e34150c00229791cc89ff309fff21".into()
    }
    #[inline]
    fn msg_type() -> String {
        "test_msgs/TwoInts".into()
    }
}

impl RosMsg for TwoInts {
    fn encode<W: std::io::Write>(&self, _w: W) -> std::io::Result<()> {
        Ok(())
    }
    fn decode<R: std::io::Read>(_r: R) -> std::io::Result<Self> {
        Ok(Self {})
    }
}

impl rosrust::ServicePair for TwoInts {
    type Request = TwoIntsReq;
    type Response = TwoIntsRes;
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DudService;

impl Message for DudService {
    #[inline]
    fn msg_definition() -> String {
        String::new()
    }
    #[inline]
    fn md5sum() -> String {
        "12345".into()
    }
    #[inline]
    fn msg_type() -> String {
        "some_msg".into()
    }
}

impl RosMsg for DudService {
    fn encode<W: std::io::Write>(&self, _w: W) -> std::io::Result<()> {
        Ok(())
    }
    fn decode<R: std::io::Read>(_r: R) -> std::io::Result<Self> {
        Ok(Self {})
    }
}

impl ServicePair for DudService {
    type Request = i8;
    type Response = i8;
}
