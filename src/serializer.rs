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

/// Convenience function to build a Logon message with ResetSeqNumFlag=Y.
pub fn build_logon_with_reset(
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
        .add_int(tags::HEARTBT_INT, heartbeat_interval)
        .add_str(tags::RESET_SEQ_NUM_FLAG, b"Y");
    ser.finalize()
}

/// Convenience function to build a Logout message.
pub fn build_logout(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
) -> usize {
    let mut ser = FixSerializer::new(buf);
    ser.begin(begin_string, b"5")
        .add_str(tags::SENDER_COMP_ID, sender)
        .add_str(tags::TARGET_COMP_ID, target)
        .add_u64(tags::MSG_SEQ_NUM, seq_num)
        .add_str(tags::SENDING_TIME, sending_time);
    ser.finalize()
}

/// Convenience function to build a SequenceReset-GapFill message (35=4, 123=Y).
pub fn build_sequence_reset_gap_fill(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
    new_seq_no: u64,
) -> usize {
    let mut ser = FixSerializer::new(buf);
    ser.begin(begin_string, b"4")
        .add_str(tags::SENDER_COMP_ID, sender)
        .add_str(tags::TARGET_COMP_ID, target)
        .add_u64(tags::MSG_SEQ_NUM, seq_num)
        .add_str(tags::SENDING_TIME, sending_time)
        .add_str(tags::GAP_FILL_FLAG, b"Y")
        .add_u64(tags::NEW_SEQ_NO, new_seq_no)
        .add_str(tags::POSS_DUP_FLAG, b"Y");
    ser.finalize()
}

/// Convenience function to build a Heartbeat message with an optional TestReqID.
pub fn build_heartbeat_with_test_req_id(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
    test_req_id: Option<&[u8]>,
) -> usize {
    let mut ser = FixSerializer::new(buf);
    ser.begin(begin_string, b"0")
        .add_str(tags::SENDER_COMP_ID, sender)
        .add_str(tags::TARGET_COMP_ID, target)
        .add_u64(tags::MSG_SEQ_NUM, seq_num)
        .add_str(tags::SENDING_TIME, sending_time);
    if let Some(id) = test_req_id {
        ser.add_str(tags::TEST_REQ_ID, id);
    }
    ser.finalize()
}

/// Convenience function to build a ResendRequest message.
pub fn build_resend_request(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
    begin_seq_no: u64,
    end_seq_no: u64,
) -> usize {
    let mut ser = FixSerializer::new(buf);
    ser.begin(begin_string, b"2")
        .add_str(tags::SENDER_COMP_ID, sender)
        .add_str(tags::TARGET_COMP_ID, target)
        .add_u64(tags::MSG_SEQ_NUM, seq_num)
        .add_str(tags::SENDING_TIME, sending_time)
        .add_u64(tags::BEGIN_SEQ_NO, begin_seq_no)
        .add_u64(tags::END_SEQ_NO, end_seq_no);
    ser.finalize()
}

/// Convenience function to build a Reject message (35=3).
pub fn build_reject(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
    ref_seq_num: u64,
    reason: u32,
    text: Option<&[u8]>,
) -> usize {
    let mut ser = FixSerializer::new(buf);
    ser.begin(begin_string, b"3")
        .add_str(tags::SENDER_COMP_ID, sender)
        .add_str(tags::TARGET_COMP_ID, target)
        .add_u64(tags::MSG_SEQ_NUM, seq_num)
        .add_str(tags::SENDING_TIME, sending_time)
        .add_u64(tags::REF_SEQ_NUM, ref_seq_num)
        .add_u64(tags::SESSION_REJECT_REASON, reason as u64);
    if let Some(text) = text {
        ser.add_str(tags::TEXT, text);
    }
    ser.finalize()
}

/// Convenience function to build a Logout message with optional text.
pub fn build_logout_with_text(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
    text: Option<&[u8]>,
) -> usize {
    let mut ser = FixSerializer::new(buf);
    ser.begin(begin_string, b"5")
        .add_str(tags::SENDER_COMP_ID, sender)
        .add_str(tags::TARGET_COMP_ID, target)
        .add_u64(tags::MSG_SEQ_NUM, seq_num)
        .add_str(tags::SENDING_TIME, sending_time);
    if let Some(text) = text {
        ser.add_str(tags::TEXT, text);
    }
    ser.finalize()
}

/// Convenience function to build a TestRequest message.
pub fn build_test_request(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
    test_req_id: &[u8],
) -> usize {
    let mut ser = FixSerializer::new(buf);
    ser.begin(begin_string, b"1")
        .add_str(tags::SENDER_COMP_ID, sender)
        .add_str(tags::TARGET_COMP_ID, target)
        .add_u64(tags::MSG_SEQ_NUM, seq_num)
        .add_str(tags::SENDING_TIME, sending_time)
        .add_str(tags::TEST_REQ_ID, test_req_id);
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

/// Convenience function to build an OrderCancelRequest message.
pub fn build_order_cancel_request(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
    orig_cl_ord_id: &[u8],
    cl_ord_id: &[u8],
    symbol: &[u8],
    side: u8,
    qty: i64,
) -> usize {
    let mut ser = FixSerializer::new(buf);
    ser.begin(begin_string, b"F")
        .add_str(tags::SENDER_COMP_ID, sender)
        .add_str(tags::TARGET_COMP_ID, target)
        .add_u64(tags::MSG_SEQ_NUM, seq_num)
        .add_str(tags::SENDING_TIME, sending_time)
        .add_str(tags::ORIG_CL_ORD_ID, orig_cl_ord_id)
        .add_str(tags::CL_ORD_ID, cl_ord_id)
        .add_str(tags::SYMBOL, symbol)
        .add_str(tags::SIDE, &[side])
        .add_int(tags::ORDER_QTY, qty)
        .add_str(tags::TRANSACT_TIME, sending_time);
    ser.finalize()
}

/// Convenience function to build an OrderCancelReplaceRequest message.
pub fn build_order_cancel_replace_request(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
    orig_cl_ord_id: &[u8],
    cl_ord_id: &[u8],
    symbol: &[u8],
    side: u8,
    qty: i64,
    ord_type: u8,
    price: &[u8],
) -> usize {
    let mut ser = FixSerializer::new(buf);
    ser.begin(begin_string, b"G")
        .add_str(tags::SENDER_COMP_ID, sender)
        .add_str(tags::TARGET_COMP_ID, target)
        .add_u64(tags::MSG_SEQ_NUM, seq_num)
        .add_str(tags::SENDING_TIME, sending_time)
        .add_str(tags::ORIG_CL_ORD_ID, orig_cl_ord_id)
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

/// Convenience function to build an OrderStatusRequest message.
pub fn build_order_status_request(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
    cl_ord_id: &[u8],
    symbol: &[u8],
    side: u8,
) -> usize {
    let mut ser = FixSerializer::new(buf);
    ser.begin(begin_string, b"H")
        .add_str(tags::SENDER_COMP_ID, sender)
        .add_str(tags::TARGET_COMP_ID, target)
        .add_u64(tags::MSG_SEQ_NUM, seq_num)
        .add_str(tags::SENDING_TIME, sending_time)
        .add_str(tags::CL_ORD_ID, cl_ord_id)
        .add_str(tags::SYMBOL, symbol)
        .add_str(tags::SIDE, &[side]);
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
    build_execution_report_with_orig_cl_ord_id(
        buf,
        begin_string,
        sender,
        target,
        seq_num,
        sending_time,
        order_id,
        exec_id,
        cl_ord_id,
        None,
        symbol,
        side,
        ord_qty,
        last_qty,
        last_px,
        leaves_qty,
        cum_qty,
        avg_px,
        exec_type,
        ord_status,
    )
}

/// Convenience function to build an ExecutionReport message with optional OrigClOrdID.
pub fn build_execution_report_with_orig_cl_ord_id(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
    order_id: &[u8],
    exec_id: &[u8],
    cl_ord_id: &[u8],
    orig_cl_ord_id: Option<&[u8]>,
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
        .add_str(tags::CL_ORD_ID, cl_ord_id);
    if let Some(orig_cl_ord_id) = orig_cl_ord_id {
        ser.add_str(tags::ORIG_CL_ORD_ID, orig_cl_ord_id);
    }
    ser.add_str(tags::EXEC_TYPE, &[exec_type])
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

/// Convenience function to build an OrderCancelReject message.
pub fn build_order_cancel_reject(
    buf: &mut [u8],
    begin_string: &[u8],
    sender: &[u8],
    target: &[u8],
    seq_num: u64,
    sending_time: &[u8],
    order_id: &[u8],
    cl_ord_id: &[u8],
    orig_cl_ord_id: &[u8],
    ord_status: u8,
    cxl_rej_response_to: u8,
    cxl_rej_reason: Option<i64>,
    text: Option<&[u8]>,
) -> usize {
    let mut ser = FixSerializer::new(buf);
    ser.begin(begin_string, b"9")
        .add_str(tags::SENDER_COMP_ID, sender)
        .add_str(tags::TARGET_COMP_ID, target)
        .add_u64(tags::MSG_SEQ_NUM, seq_num)
        .add_str(tags::SENDING_TIME, sending_time)
        .add_str(tags::ORDER_ID, order_id)
        .add_str(tags::CL_ORD_ID, cl_ord_id)
        .add_str(tags::ORIG_CL_ORD_ID, orig_cl_ord_id)
        .add_str(tags::ORD_STATUS, &[ord_status])
        .add_str(tags::CXL_REJ_RESPONSE_TO, &[cxl_rej_response_to]);
    if let Some(cxl_rej_reason) = cxl_rej_reason {
        ser.add_int(tags::CXL_REJ_REASON, cxl_rej_reason);
    }
    if let Some(text) = text {
        ser.add_str(tags::TEXT, text);
    }
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
        let (view, consumed) = parser
            .parse(msg)
            .expect("should parse serialized heartbeat");
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
    fn test_serialize_order_lifecycle_requests() {
        let parser = FixParser::new();
        let mut buf = [0u8; 1024];

        let len = build_order_cancel_request(
            &mut buf,
            b"FIX.4.4",
            b"BANK_OMS",
            b"NYSE",
            43,
            b"20260321-10:00:00.123",
            b"ORD-00001",
            b"CXL-00001",
            b"AAPL",
            b'1',
            1000,
        );
        let (view, _) = parser.parse(&buf[..len]).expect("should parse cancel req");
        assert_eq!(view.msg_type(), Some(b"F".as_slice()));
        assert_eq!(view.get_field_str(tags::ORIG_CL_ORD_ID), Some("ORD-00001"));
        assert_eq!(view.get_field_str(tags::CL_ORD_ID), Some("CXL-00001"));

        let len = build_order_cancel_replace_request(
            &mut buf,
            b"FIX.4.4",
            b"BANK_OMS",
            b"NYSE",
            44,
            b"20260321-10:00:00.124",
            b"ORD-00001",
            b"RPL-00001",
            b"AAPL",
            b'1',
            1200,
            b'2',
            b"150.75",
        );
        let (view, _) = parser.parse(&buf[..len]).expect("should parse replace req");
        assert_eq!(view.msg_type(), Some(b"G".as_slice()));
        assert_eq!(view.get_field_str(tags::ORIG_CL_ORD_ID), Some("ORD-00001"));
        assert_eq!(view.get_field_str(tags::CL_ORD_ID), Some("RPL-00001"));
        assert_eq!(view.get_field_str(tags::PRICE), Some("150.75"));

        let len = build_order_status_request(
            &mut buf,
            b"FIX.4.4",
            b"BANK_OMS",
            b"NYSE",
            45,
            b"20260321-10:00:00.125",
            b"RPL-00001",
            b"AAPL",
            b'1',
        );
        let (view, _) = parser.parse(&buf[..len]).expect("should parse status req");
        assert_eq!(view.msg_type(), Some(b"H".as_slice()));
        assert_eq!(view.get_field_str(tags::CL_ORD_ID), Some("RPL-00001"));
        assert_eq!(view.get_field_str(tags::SYMBOL), Some("AAPL"));
    }

    #[test]
    fn test_serialize_order_cancel_reject_and_execution_report_with_orig_cl_ord_id() {
        let parser = FixParser::new();
        let mut buf = [0u8; 2048];

        let len = build_execution_report_with_orig_cl_ord_id(
            &mut buf,
            b"FIX.4.4",
            b"NYSE",
            b"BANK_OMS",
            101,
            b"20260321-10:00:00.457",
            b"NYSE-ORD-001",
            b"NYSE-EXEC-002",
            b"RPL-00001",
            Some(b"ORD-00001"),
            b"AAPL",
            b'1',
            1200,
            0,
            b"0",
            1200,
            0,
            b"0",
            b'5',
            b'5',
        );
        let (view, _) = parser
            .parse(&buf[..len])
            .expect("should parse replace execution report");
        assert_eq!(view.msg_type(), Some(b"8".as_slice()));
        assert_eq!(view.get_field_str(tags::ORIG_CL_ORD_ID), Some("ORD-00001"));

        let len = build_order_cancel_reject(
            &mut buf,
            b"FIX.4.4",
            b"NYSE",
            b"BANK_OMS",
            102,
            b"20260321-10:00:00.458",
            b"NYSE-ORD-001",
            b"CXL-00002",
            b"RPL-00001",
            b'4',
            b'1',
            Some(1),
            Some(b"order already canceled"),
        );
        let (view, _) = parser
            .parse(&buf[..len])
            .expect("should parse cancel reject");
        assert_eq!(view.msg_type(), Some(b"9".as_slice()));
        assert_eq!(view.get_field_str(tags::ORDER_ID), Some("NYSE-ORD-001"));
        assert_eq!(view.get_field_str(tags::CL_ORD_ID), Some("CXL-00002"));
        assert_eq!(view.get_field_str(tags::ORIG_CL_ORD_ID), Some("RPL-00001"));
        assert_eq!(
            view.get_field_str(tags::TEXT),
            Some("order already canceled")
        );
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
