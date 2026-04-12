/// FIX message representation using the flyweight pattern.
///
/// `MessageView` provides zero-copy access to a FIX message — field values
/// are slices into the original wire buffer, with no heap allocations.
use crate::tags;

/// Maximum number of fields in a single FIX message.
pub const MAX_FIELDS: usize = 512;

/// A single field entry: tag number and the byte range of its value in the buffer.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C, align(16))]
pub struct FieldEntry {
    pub tag: u32,
    pub offset: u32,
    pub length: u16,
    _pad: u16,
}

/// Zero-copy flyweight view over a FIX message buffer.
///
/// Does not own the buffer — the buffer must outlive this view.
/// No heap allocations. All field lookups are O(n) scans over a small
/// pre-allocated array (typically ≤ 50 entries for most messages).
pub struct MessageView<'a> {
    buffer: &'a [u8],
    field_count: u16,
    fields: [FieldEntry; MAX_FIELDS],
    msg_type_idx: u16,
    checksum_valid: bool,
}

impl<'a> std::fmt::Debug for MessageView<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageView")
            .field("field_count", &self.field_count)
            .field("checksum_valid", &self.checksum_valid)
            .finish()
    }
}

impl<'a> MessageView<'a> {
    /// Create a new empty MessageView backed by the given buffer.
    #[inline]
    pub fn new(buffer: &'a [u8]) -> Self {
        MessageView {
            buffer,
            field_count: 0,
            fields: [FieldEntry::default(); MAX_FIELDS],
            msg_type_idx: 0,
            checksum_valid: false,
        }
    }

    /// Add a field entry. Called by the parser during message parsing.
    #[inline]
    pub fn add_field(&mut self, tag: u32, offset: u32, length: u16) {
        if (self.field_count as usize) < MAX_FIELDS {
            let idx = self.field_count as usize;
            self.fields[idx] = FieldEntry {
                tag,
                offset,
                length,
                _pad: 0,
            };
            if tag == tags::MSG_TYPE {
                self.msg_type_idx = self.field_count;
            }
            self.field_count += 1;
        }
    }

    /// Set the checksum validation result.
    #[inline]
    pub fn set_checksum_valid(&mut self, valid: bool) {
        self.checksum_valid = valid;
    }

    /// Returns whether the checksum was valid.
    #[inline]
    pub fn is_checksum_valid(&self) -> bool {
        self.checksum_valid
    }

    /// Get the number of fields in this message.
    #[inline]
    pub fn field_count(&self) -> usize {
        self.field_count as usize
    }

    /// Get the raw byte value of a field by tag number.
    /// Returns the first occurrence of the tag.
    #[inline]
    pub fn get_field(&self, tag: u32) -> Option<&'a [u8]> {
        for i in 0..self.field_count as usize {
            if self.fields[i].tag == tag {
                let offset = self.fields[i].offset as usize;
                let length = self.fields[i].length as usize;
                return Some(&self.buffer[offset..offset + length]);
            }
        }
        None
    }

    /// Get field value as a string slice (no allocation).
    #[inline]
    pub fn get_field_str(&self, tag: u32) -> Option<&'a str> {
        self.get_field(tag)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
    }

    /// Get field value as i64 (parsed inline, no allocation).
    #[inline]
    pub fn get_field_i64(&self, tag: u32) -> Option<i64> {
        self.get_field(tag).and_then(|bytes| parse_i64(bytes))
    }

    /// Get field value as u64.
    #[inline]
    pub fn get_field_u64(&self, tag: u32) -> Option<u64> {
        self.get_field(tag).and_then(|bytes| parse_u64(bytes))
    }

    /// Get the MsgType (tag 35) as a byte slice.
    #[inline]
    pub fn msg_type(&self) -> Option<&'a [u8]> {
        let idx = self.msg_type_idx as usize;
        if idx < self.field_count as usize && self.fields[idx].tag == tags::MSG_TYPE {
            let offset = self.fields[idx].offset as usize;
            let length = self.fields[idx].length as usize;
            Some(&self.buffer[offset..offset + length])
        } else {
            self.get_field(tags::MSG_TYPE)
        }
    }

    /// Get the MsgType as the enum variant.
    #[inline]
    pub fn msg_type_enum(&self) -> Option<MsgType> {
        self.msg_type().and_then(MsgType::from_bytes)
    }

    /// Get the underlying buffer.
    #[inline]
    pub fn buffer(&self) -> &'a [u8] {
        self.buffer
    }

    /// Get the field entries slice.
    #[inline]
    pub fn fields(&self) -> &[FieldEntry] {
        &self.fields[..self.field_count as usize]
    }

    /// Get BeginString (tag 8).
    #[inline]
    pub fn begin_string(&self) -> Option<&'a str> {
        self.get_field_str(tags::BEGIN_STRING)
    }

    /// Get SenderCompID (tag 49).
    #[inline]
    pub fn sender_comp_id(&self) -> Option<&'a str> {
        self.get_field_str(tags::SENDER_COMP_ID)
    }

    /// Get TargetCompID (tag 56).
    #[inline]
    pub fn target_comp_id(&self) -> Option<&'a str> {
        self.get_field_str(tags::TARGET_COMP_ID)
    }

    /// Get MsgSeqNum (tag 34).
    #[inline]
    pub fn msg_seq_num(&self) -> Option<u64> {
        self.get_field_u64(tags::MSG_SEQ_NUM)
    }
}

/// Parse i64 from ASCII bytes without allocation.
#[inline]
fn parse_i64(bytes: &[u8]) -> Option<i64> {
    if bytes.is_empty() {
        return None;
    }

    let (negative, start) = if bytes[0] == b'-' {
        (true, 1)
    } else {
        (false, 0)
    };

    let mut result: i64 = 0;
    for &b in &bytes[start..] {
        if b < b'0' || b > b'9' {
            return None;
        }
        result = result * 10 + (b - b'0') as i64;
    }

    Some(if negative { -result } else { result })
}

/// Parse u64 from ASCII bytes without allocation.
#[inline]
fn parse_u64(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }

    let mut result: u64 = 0;
    for &b in bytes {
        if b < b'0' || b > b'9' {
            return None;
        }
        result = result * 10 + (b - b'0') as u64;
    }

    Some(result)
}

/// FIX MsgType enum for fast dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgType {
    Heartbeat,                 // 0
    TestRequest,               // 1
    ResendRequest,             // 2
    Reject,                    // 3
    SequenceReset,             // 4
    Logout,                    // 5
    Logon,                     // A
    NewOrderSingle,            // D
    OrderCancelRequest,        // F
    OrderCancelReplaceRequest, // G
    OrderStatusRequest,        // H
    ExecutionReport,           // 8
    OrderCancelReject,         // 9
    MarketDataRequest,         // V
    MarketDataSnapshot,        // W
    MarketDataIncremental,     // X
    QuoteRequest,              // R
    Quote,                     // S
    TradeCaptureReport,        // AE
    Unknown,
}

impl MsgType {
    #[inline]
    pub fn from_bytes(bytes: &[u8]) -> Option<MsgType> {
        match bytes {
            b"0" => Some(MsgType::Heartbeat),
            b"1" => Some(MsgType::TestRequest),
            b"2" => Some(MsgType::ResendRequest),
            b"3" => Some(MsgType::Reject),
            b"4" => Some(MsgType::SequenceReset),
            b"5" => Some(MsgType::Logout),
            b"A" => Some(MsgType::Logon),
            b"D" => Some(MsgType::NewOrderSingle),
            b"F" => Some(MsgType::OrderCancelRequest),
            b"G" => Some(MsgType::OrderCancelReplaceRequest),
            b"H" => Some(MsgType::OrderStatusRequest),
            b"8" => Some(MsgType::ExecutionReport),
            b"9" => Some(MsgType::OrderCancelReject),
            b"V" => Some(MsgType::MarketDataRequest),
            b"W" => Some(MsgType::MarketDataSnapshot),
            b"X" => Some(MsgType::MarketDataIncremental),
            b"R" => Some(MsgType::QuoteRequest),
            b"S" => Some(MsgType::Quote),
            b"AE" => Some(MsgType::TradeCaptureReport),
            _ => Some(MsgType::Unknown),
        }
    }

    #[inline]
    pub fn as_bytes(&self) -> &'static [u8] {
        match self {
            MsgType::Heartbeat => b"0",
            MsgType::TestRequest => b"1",
            MsgType::ResendRequest => b"2",
            MsgType::Reject => b"3",
            MsgType::SequenceReset => b"4",
            MsgType::Logout => b"5",
            MsgType::Logon => b"A",
            MsgType::NewOrderSingle => b"D",
            MsgType::OrderCancelRequest => b"F",
            MsgType::OrderCancelReplaceRequest => b"G",
            MsgType::OrderStatusRequest => b"H",
            MsgType::ExecutionReport => b"8",
            MsgType::OrderCancelReject => b"9",
            MsgType::MarketDataRequest => b"V",
            MsgType::MarketDataSnapshot => b"W",
            MsgType::MarketDataIncremental => b"X",
            MsgType::QuoteRequest => b"R",
            MsgType::Quote => b"S",
            MsgType::TradeCaptureReport => b"AE",
            MsgType::Unknown => b"?",
        }
    }

    pub fn is_session_level(&self) -> bool {
        matches!(
            self,
            MsgType::Heartbeat
                | MsgType::TestRequest
                | MsgType::ResendRequest
                | MsgType::Reject
                | MsgType::SequenceReset
                | MsgType::Logout
                | MsgType::Logon
        )
    }

    pub fn is_admin(&self) -> bool {
        self.is_session_level()
    }
}

/// FIX Side (tag 54) values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy = 1,
    Sell = 2,
    BuyMinus = 3,
    SellPlus = 4,
    SellShort = 5,
    SellShortExempt = 6,
    Cross = 8,
}

impl Side {
    pub fn from_byte(b: u8) -> Option<Side> {
        match b {
            b'1' => Some(Side::Buy),
            b'2' => Some(Side::Sell),
            b'3' => Some(Side::BuyMinus),
            b'4' => Some(Side::SellPlus),
            b'5' => Some(Side::SellShort),
            b'6' => Some(Side::SellShortExempt),
            b'8' => Some(Side::Cross),
            _ => None,
        }
    }
}

/// FIX OrdType (tag 40) values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrdType {
    Market = 1,
    Limit = 2,
    Stop = 3,
    StopLimit = 4,
}

impl OrdType {
    pub fn from_byte(b: u8) -> Option<OrdType> {
        match b {
            b'1' => Some(OrdType::Market),
            b'2' => Some(OrdType::Limit),
            b'3' => Some(OrdType::Stop),
            b'4' => Some(OrdType::StopLimit),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_i64() {
        assert_eq!(parse_i64(b"12345"), Some(12345));
        assert_eq!(parse_i64(b"-42"), Some(-42));
        assert_eq!(parse_i64(b"0"), Some(0));
        assert_eq!(parse_i64(b""), None);
        assert_eq!(parse_i64(b"abc"), None);
    }

    #[test]
    fn test_parse_u64() {
        assert_eq!(parse_u64(b"12345"), Some(12345));
        assert_eq!(parse_u64(b"0"), Some(0));
        assert_eq!(parse_u64(b""), None);
        assert_eq!(parse_u64(b"-1"), None);
    }

    #[test]
    fn test_msg_type_roundtrip() {
        for mt in &[
            MsgType::Heartbeat,
            MsgType::Logon,
            MsgType::NewOrderSingle,
            MsgType::ExecutionReport,
            MsgType::TradeCaptureReport,
        ] {
            let bytes = mt.as_bytes();
            let parsed = MsgType::from_bytes(bytes).unwrap();
            assert_eq!(*mt, parsed);
        }
    }

    #[test]
    fn test_message_view_field_access() {
        // Construct a minimal FIX message buffer
        let msg = b"8=FIX.4.4\x019=70\x0135=D\x0149=SENDER\x0156=TARGET\x0134=1\x0152=20260321-10:00:00\x0111=ORD001\x0155=AAPL\x0154=1\x0138=100\x0140=2\x0144=150.50\x0110=000\x01";

        let mut view = MessageView::new(msg);

        // Manually add fields (normally done by parser)
        // 8=FIX.4.4 starts at 2, length 7
        view.add_field(8, 2, 7);
        // 35=D starts at offset: find it
        // For this test, just verify the API works
        assert_eq!(view.field_count(), 1);
        assert_eq!(view.get_field_str(8), Some("FIX.4.4"));
    }

    #[test]
    fn test_side_from_byte() {
        assert_eq!(Side::from_byte(b'1'), Some(Side::Buy));
        assert_eq!(Side::from_byte(b'2'), Some(Side::Sell));
        assert_eq!(Side::from_byte(b'X'), None);
    }
}
