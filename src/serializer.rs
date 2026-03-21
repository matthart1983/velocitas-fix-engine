/// Zero-copy FIX message serializer.
///
/// Builds FIX messages directly into pre-allocated byte buffers.
/// No heap allocations. Uses lookup tables for fast integer-to-ASCII conversion.

use crate::checksum;
use crate::tags::{self, EQUALS, SOH};

/// Pre-computed lookup table for 2-digit numbers (00–99).
/// Used for fast integer-to-ASCII conversion.
static DIGIT_PAIRS: &[u8; 200] = b"\
    0001020304050607080910111213141516171819\
    2021222324252627282930313233343536373839\
    4041424344454647484950515253545556575859\
    6061626364656667686970717273747576777879\
    8081828384858687888990919293949596979899";

/// FIX message serializer. Writes directly into a caller-provided buffer.
pub struct FixSerializer<'a> {
    buffer: &'a mut [u8],
    pos: usize,
    body_start: usize,
}

impl<'a> FixSerializer<'a> {
    /// Create a new serializer writing into the provided buffer.
    #[inline]
    pub fn new(buffer: &'a mut [u8]) -> Self {
        FixSerializer {
            buffer,
            pos: 0,
            body_start: 0,
        }
    }

    /// Begin a new FIX message with the given BeginString and MsgType.
    /// Reserves space for BodyLength (tag 9) to be filled in at finalize.
    #[inline]
    pub fn begin(&mut self, begin_string: &[u8], msg_type: &[u8]) -> &mut Self {
        // Write "8=FIX.4.4\x01"
        self.write_tag_value(tags::BEGIN_STRING, begin_string);

        // Write "9=" and reserve 6 bytes for body length (e.g., "000123") + SOH
        self.write_bytes(b"9=000000\x01");
        self.body_start = self.pos;

        // Write "35=X\x01"
        self.write_tag_value(tags::MSG_TYPE, msg_type);

        self
    }

    /// Add a string field (tag=value).
    #[inline]
    pub fn add_str(&mut self, tag: u32, value: &[u8]) -> &mut Self {
        self.write_tag_value(tag, value);
        self
    }

    /// Add an integer field.
    #[inline]
    pub fn add_int(&mut self, tag: u32, value: i64) -> &mut Self {
        self.write_tag(tag);
        self.write_i64(value);
        self.buffer[self.pos] = SOH;
        self.pos += 1;
        self
    }

    /// Add a u64 field.
    #[inline]
    pub fn add_u64(&mut self, tag: u32, value: u64) -> &mut Self {
        self.write_tag(tag);
        self.write_u64(value);
        self.buffer[self.pos] = SOH;
        self.pos += 1;
        self
    }

    /// Finalize the message: compute and write BodyLength and Checksum.
    /// Returns the total message length.
    #[inline]
    pub fn finalize(&mut self) -> usize {
        let body_end = self.pos;
        let body_length = body_end - self.body_start;

        // Write body length back into the reserved space
        // The reserved space is at body_start - 7 (the "000000\x01" we wrote)
        let bl_start = self.body_start - 7;
        let mut bl_buf = [0u8; 6];
        let bl_len = write_u64_to_buf(body_length as u64, &mut bl_buf);
        // Right-justify in the 6-byte field
        let bl_offset = bl_start + (6 - bl_len);
        // First, zero-fill the reserved space
        for i in 0..6 {
            self.buffer[bl_start + i] = b'0';
        }
        self.buffer[bl_offset..bl_offset + bl_len].copy_from_slice(&bl_buf[..bl_len]);

        // Compute checksum over everything up to this point
        let cs = checksum::compute(&self.buffer[..body_end]);
        let mut cs_buf = [0u8; 3];
        checksum::format(cs, &mut cs_buf);

        // Write "10=XXX\x01"
        self.buffer[self.pos] = b'1';
        self.buffer[self.pos + 1] = b'0';
        self.buffer[self.pos + 2] = b'=';
        self.buffer[self.pos + 3] = cs_buf[0];
        self.buffer[self.pos + 4] = cs_buf[1];
        self.buffer[self.pos + 5] = cs_buf[2];
        self.buffer[self.pos + 6] = SOH;
        self.pos += 7;

        self.pos
    }

    /// Get the serialized message as a byte slice.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buffer[..self.pos]
    }

    /// Current write position.
    #[inline]
    pub fn position(&self) -> usize {
        self.pos
    }

    // --- Internal helpers ---

    #[inline]
    fn write_tag_value(&mut self, tag: u32, value: &[u8]) {
        self.write_tag(tag);
        self.write_bytes(value);
        self.buffer[self.pos] = SOH;
        self.pos += 1;
    }

    #[inline]
    fn write_tag(&mut self, tag: u32) {
        self.write_u32(tag);
        self.buffer[self.pos] = EQUALS;
        self.pos += 1;
    }

    #[inline]
    fn write_bytes(&mut self, data: &[u8]) {
        let end = self.pos + data.len();
        self.buffer[self.pos..end].copy_from_slice(data);
        self.pos = end;
    }

    #[inline]
    fn write_u32(&mut self, mut val: u32) {
        if val == 0 {
            self.buffer[self.pos] = b'0';
            self.pos += 1;
            return;
        }

        let mut buf = [0u8; 10];
        let mut i = 10;

        while val >= 100 {
            let r = (val % 100) as usize;
            val /= 100;
            i -= 2;
            buf[i] = DIGIT_PAIRS[r * 2];
            buf[i + 1] = DIGIT_PAIRS[r * 2 + 1];
        }

        if val >= 10 {
            let r = val as usize;
            i -= 2;
            buf[i] = DIGIT_PAIRS[r * 2];
            buf[i + 1] = DIGIT_PAIRS[r * 2 + 1];
        } else {
            i -= 1;
            buf[i] = b'0' + val as u8;
        }

        let len = 10 - i;
        self.buffer[self.pos..self.pos + len].copy_from_slice(&buf[i..10]);
        self.pos += len;
    }

    #[inline]
    fn write_i64(&mut self, val: i64) {
        if val < 0 {
            self.buffer[self.pos] = b'-';
            self.pos += 1;
            self.write_u64((-val) as u64);
        } else {
            self.write_u64(val as u64);
        }
    }

    #[inline]
    fn write_u64(&mut self, mut val: u64) {
        if val == 0 {
            self.buffer[self.pos] = b'0';
            self.pos += 1;
            return;
        }

        let mut buf = [0u8; 20];
        let mut i = 20;

        while val >= 100 {
            let r = (val % 100) as usize;
            val /= 100;
            i -= 2;
            buf[i] = DIGIT_PAIRS[r * 2];
            buf[i + 1] = DIGIT_PAIRS[r * 2 + 1];
        }

        if val >= 10 {
            let r = val as usize;
            i -= 2;
            buf[i] = DIGIT_PAIRS[r * 2];
            buf[i + 1] = DIGIT_PAIRS[r * 2 + 1];
        } else {
            i -= 1;
            buf[i] = b'0' + val as u8;
        }

        let len = 20 - i;
        self.buffer[self.pos..self.pos + len].copy_from_slice(&buf[i..20]);
        self.pos += len;
    }
}

/// Write a u64 into a buffer, returning the number of digits written.
fn write_u64_to_buf(mut val: u64, buf: &mut [u8; 6]) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }

    let mut tmp = [0u8; 6];
    let mut i = 6;

    while val > 0 && i > 0 {
        i -= 1;
        tmp[i] = b'0' + (val % 10) as u8;
        val /= 10;
    }

    let len = 6 - i;
    buf[..len].copy_from_slice(&tmp[i..6]);
    len
}

/// Convenience function to build a Heartbeat message.
pub fn build_heartbeat(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
) -> usize {
    let mut ser = FixSerializer::new(buf);
    ser.begin(begin_string, b"0")
        .add_str(tags::SENDER_COMP_ID, sender)
        .add_str(tags::TARGET_COMP_ID, target)
        .add_u64(tags::MSG_SEQ_NUM, seq_num)
        .add_str(tags::SENDING_TIME, sending_time);
    ser.finalize()
}

/// Convenience function to build a Logon message.
pub fn build_logon(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
    heartbeat_interval: i64,
) -> usize {
    let mut ser = FixSerializer::new(buf);
    ser.begin(begin_string, b"A")
        .add_str(tags::SENDER_COMP_ID, sender)
        .add_str(tags::TARGET_COMP_ID, target)
        .add_u64(tags::MSG_SEQ_NUM, seq_num)
        .add_str(tags::SENDING_TIME, sending_time)
        .add_int(tags::ENCRYPT_METHOD, 0)
        .add_int(tags::HEARTBT_INT, heartbeat_interval);
    ser.finalize()
}

/// Convenience function to build a NewOrderSingle message.
pub fn build_new_order_single(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
    cl_ord_id: &[u8],
    symbol: &[u8],
    side: u8,
    qty: i64,
    ord_type: u8,
    price: &[u8],
) -> usize {
    let mut ser = FixSerializer::new(buf);
    ser.begin(begin_string, b"D")
        .add_str(tags::SENDER_COMP_ID, sender)
        .add_str(tags::TARGET_COMP_ID, target)
        .add_u64(tags::MSG_SEQ_NUM, seq_num)
        .add_str(tags::SENDING_TIME, sending_time)
        .add_str(tags::CL_ORD_ID, cl_ord_id)
        .add_str(tags::SYMBOL, symbol)
        .add_str(tags::SIDE, &[side])
        .add_int(tags::ORDER_QTY, qty)
        .add_str(tags::ORD_TYPE, &[ord_type])
        .add_str(tags::PRICE, price)
        .add_str(tags::TRANSACT_TIME, sending_time)
        .add_str(tags::HANDL_INST, b"1");
    ser.finalize()
}

/// Convenience function to build an ExecutionReport message.
pub fn build_execution_report(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
    order_id: &[u8],
    exec_id: &[u8],
    cl_ord_id: &[u8],
    symbol: &[u8],
    side: u8,
    ord_qty: i64,
    last_qty: i64,
    last_px: &[u8],
    leaves_qty: i64,
    cum_qty: i64,
    avg_px: &[u8],
    exec_type: u8,
    ord_status: u8,
) -> usize {
    let mut ser = FixSerializer::new(buf);
    ser.begin(begin_string, b"8")
        .add_str(tags::SENDER_COMP_ID, sender)
        .add_str(tags::TARGET_COMP_ID, target)
        .add_u64(tags::MSG_SEQ_NUM, seq_num)
        .add_str(tags::SENDING_TIME, sending_time)
        .add_str(tags::ORDER_ID, order_id)
        .add_str(tags::EXEC_ID, exec_id)
        .add_str(tags::CL_ORD_ID, cl_ord_id)
        .add_str(tags::EXEC_TYPE, &[exec_type])
        .add_str(tags::ORD_STATUS, &[ord_status])
        .add_str(tags::SYMBOL, symbol)
        .add_str(tags::SIDE, &[side])
        .add_int(tags::ORDER_QTY, ord_qty)
        .add_int(tags::LAST_QTY, last_qty)
        .add_str(tags::LAST_PX, last_px)
        .add_int(tags::LEAVES_QTY, leaves_qty)
        .add_int(tags::CUM_QTY, cum_qty)
        .add_str(tags::AVG_PX, avg_px)
        .add_str(tags::TRANSACT_TIME, sending_time);
    ser.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::FixParser;

    #[test]
    fn test_serialize_heartbeat() {
        let mut buf = [0u8; 1024];
        let len = build_heartbeat(
            &mut buf,
            b"FIX.4.4",
            b"SENDER",
            b"TARGET",
            1,
            b"20260321-10:00:00",
        );

        let msg = &buf[..len];
        // Verify it parses back
        let parser = FixParser::new();
        let (view, consumed) = parser.parse(msg).expect("should parse serialized heartbeat");
        assert_eq!(consumed, len);
        assert_eq!(view.msg_type(), Some(b"0".as_slice()));
        assert_eq!(view.sender_comp_id(), Some("SENDER"));
        assert_eq!(view.target_comp_id(), Some("TARGET"));
        assert_eq!(view.msg_seq_num(), Some(1));
    }

    #[test]
    fn test_serialize_new_order_single() {
        let mut buf = [0u8; 1024];
        let len = build_new_order_single(
            &mut buf,
            b"FIX.4.4",
            b"BANK_OMS",
            b"NYSE",
            42,
            b"20260321-10:00:00.123",
            b"ORD-00001",
            b"AAPL",
            b'1',
            1000,
            b'2',
            b"150.50",
        );

        let msg = &buf[..len];
        let parser = FixParser::new();
        let (view, _) = parser.parse(msg).expect("should parse serialized NOS");
        assert_eq!(view.msg_type(), Some(b"D".as_slice()));
        assert_eq!(view.get_field_str(tags::CL_ORD_ID), Some("ORD-00001"));
        assert_eq!(view.get_field_str(tags::SYMBOL), Some("AAPL"));
        assert_eq!(view.get_field_i64(tags::ORDER_QTY), Some(1000));
    }

    #[test]
    fn test_serialize_execution_report() {
        let mut buf = [0u8; 2048];
        let len = build_execution_report(
            &mut buf,
            b"FIX.4.4",
            b"NYSE",
            b"BANK_OMS",
            100,
            b"20260321-10:00:00.456",
            b"NYSE-ORD-001",
            b"NYSE-EXEC-001",
            b"ORD-00001",
            b"AAPL",
            b'1',
            1000,
            500,
            b"150.50",
            500,
            500,
            b"150.50",
            b'F',
            b'1',
        );

        let msg = &buf[..len];
        let parser = FixParser::new();
        let (view, _) = parser.parse(msg).expect("should parse serialized ExecRpt");
        assert_eq!(view.msg_type(), Some(b"8".as_slice()));
        assert_eq!(view.get_field_str(tags::ORDER_ID), Some("NYSE-ORD-001"));
        assert_eq!(view.get_field_i64(tags::LAST_QTY), Some(500));
    }

    #[test]
    fn test_roundtrip_large_seq_num() {
        let mut buf = [0u8; 1024];
        let len = build_heartbeat(
            &mut buf,
            b"FIX.4.4",
            b"S",
            b"T",
            999_999,
            b"20260321-10:00:00",
        );

        let parser = FixParser::new();
        let (view, _) = parser.parse(&buf[..len]).expect("should parse");
        assert_eq!(view.msg_seq_num(), Some(999_999));
    }

    #[test]
    fn test_serialize_logon() {
        let mut buf = [0u8; 1024];
        let len = build_logon(
            &mut buf,
            b"FIX.4.4",
            b"CLIENT",
            b"SERVER",
            1,
            b"20260321-10:00:00",
            30,
        );

        let parser = FixParser::new();
        let (view, _) = parser.parse(&buf[..len]).expect("should parse logon");
        assert_eq!(view.msg_type(), Some(b"A".as_slice()));
        assert_eq!(view.get_field_i64(tags::HEARTBT_INT), Some(30));
    }
}
