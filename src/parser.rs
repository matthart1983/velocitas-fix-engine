/// Zero-copy FIX message parser.
///
/// Parses FIX messages directly from wire buffers using the flyweight pattern.
/// No heap allocations. Field values are referenced as slices into the original buffer.
///
/// Parsing strategy:
/// 1. Scan for SOH (0x01) delimiters
/// 2. Extract tag number via fast integer parsing
/// 3. Record offset/length of value in the field index
/// 4. Validate BeginString, BodyLength, and Checksum
use crate::checksum;
use crate::message::MessageView;
use crate::tags::{self, EQUALS, SOH};

/// Parse error types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Buffer is too short to contain a valid FIX message.
    BufferTooShort,
    /// Missing BeginString (tag 8) as first field.
    MissingBeginString,
    /// Missing BodyLength (tag 9) as second field.
    MissingBodyLength,
    /// BodyLength value doesn't match actual content.
    InvalidBodyLength,
    /// Missing MsgType (tag 35) as third field.
    MissingMsgType,
    /// Checksum validation failed.
    InvalidChecksum,
    /// Malformed tag=value pair (missing '=' or SOH).
    MalformedField,
    /// Tag number is zero or unparseable.
    InvalidTag,
    /// Message exceeds maximum supported size.
    MessageTooLarge,
}

/// FIX message parser — stateless, zero-allocation.
pub struct FixParser {
    /// Whether to validate checksums (can be disabled for max throughput).
    validate_checksum: bool,
    /// Whether to validate body length.
    validate_body_length: bool,
    /// Maximum message size in bytes.
    max_message_size: usize,
}

impl FixParser {
    /// Create a new parser with default settings (all validation enabled).
    pub fn new() -> Self {
        FixParser {
            validate_checksum: true,
            validate_body_length: true,
            max_message_size: 65_536,
        }
    }

    /// Create a parser optimized for maximum throughput (validation disabled).
    pub fn new_unchecked() -> Self {
        FixParser {
            validate_checksum: false,
            validate_body_length: false,
            max_message_size: 65_536,
        }
    }

    /// Set whether to validate checksums.
    pub fn validate_checksum(mut self, validate: bool) -> Self {
        self.validate_checksum = validate;
        self
    }

    /// Set whether to validate body length.
    pub fn validate_body_length(mut self, validate: bool) -> Self {
        self.validate_body_length = validate;
        self
    }

    /// Set maximum message size.
    pub fn max_message_size(mut self, size: usize) -> Self {
        self.max_message_size = size;
        self
    }

    /// Parse a FIX message from the given buffer.
    ///
    /// Returns a `MessageView` that provides zero-copy access to all fields,
    /// and the number of bytes consumed from the buffer.
    ///
    /// The buffer may contain trailing data (e.g., the start of the next message);
    /// only the first complete message is parsed.
    #[inline]
    pub fn parse<'a>(&self, buffer: &'a [u8]) -> Result<(MessageView<'a>, usize), ParseError> {
        match self.parse_prefix_inner(buffer, false)? {
            Some(parsed) => Ok(parsed),
            None => {
                if buffer.len() < 20 {
                    Err(ParseError::BufferTooShort)
                } else {
                    Err(ParseError::MalformedField)
                }
            }
        }
    }

    /// Attempt to parse the first complete FIX message prefix from a buffer.
    ///
    /// Returns `Ok(None)` when the current buffer is not yet large enough to
    /// contain a full message, which lets stream processors avoid a separate
    /// framing scan before parsing.
    #[inline]
    pub fn try_parse_prefix<'a>(
        &self,
        buffer: &'a [u8],
    ) -> Result<Option<(MessageView<'a>, usize)>, ParseError> {
        self.parse_prefix_inner(buffer, true)
    }

    fn parse_prefix_inner<'a>(
        &self,
        buffer: &'a [u8],
        allow_incomplete: bool,
    ) -> Result<Option<(MessageView<'a>, usize)>, ParseError> {
        let len = buffer.len();
        if len < 20 {
            return if allow_incomplete {
                Ok(None)
            } else {
                Err(ParseError::BufferTooShort)
            };
        }
        if len > self.max_message_size {
            return Err(ParseError::MessageTooLarge);
        }

        let mut view = MessageView::new(buffer);
        let mut pos = 0;
        let mut field_num = 0;
        let mut body_start = 0;
        let mut stated_body_length: usize = 0;
        let mut checksum_tag_start = None;
        let mut checksum_value = None;

        // Parse all tag=value\x01 fields
        while pos < len {
            // Parse tag number
            let tag_start = pos;
            let mut tag: u32 = 0;
            while pos < len && buffer[pos] != EQUALS {
                let b = buffer[pos];
                if b < b'0' || b > b'9' {
                    return Err(ParseError::InvalidTag);
                }
                tag = tag * 10 + (b - b'0') as u32;
                pos += 1;
            }

            if pos >= len {
                return if allow_incomplete {
                    Ok(None)
                } else {
                    Err(ParseError::MalformedField)
                };
            }
            pos += 1; // skip '='

            // Record value start
            let value_start = pos;

            // Scan for SOH delimiter
            while pos < len && buffer[pos] != SOH {
                pos += 1;
            }

            if pos >= len {
                return if allow_incomplete {
                    Ok(None)
                } else {
                    Err(ParseError::MalformedField)
                };
            }

            let value_len = pos - value_start;
            pos += 1; // skip SOH

            // Validate structural fields
            match field_num {
                0 => {
                    if tag != tags::BEGIN_STRING {
                        return Err(ParseError::MissingBeginString);
                    }
                }
                1 => {
                    if tag != tags::BODY_LENGTH {
                        return Err(ParseError::MissingBodyLength);
                    }
                    // Parse body length value
                    stated_body_length =
                        parse_usize_ascii(&buffer[value_start..value_start + value_len])
                            .ok_or(ParseError::InvalidBodyLength)?;
                    body_start = pos;
                }
                2 => {
                    if tag != tags::MSG_TYPE {
                        return Err(ParseError::MissingMsgType);
                    }
                }
                _ => {}
            }

            // Add field to the view
            view.add_field(tag, value_start as u32, value_len as u16);

            // If this is the checksum field, we're done
            if tag == tags::CHECKSUM {
                checksum_tag_start = Some(tag_start);
                checksum_value = Some((value_start, value_len));
                break;
            }

            field_num += 1;
        }

        let Some(checksum_tag_start) = checksum_tag_start else {
            return if allow_incomplete {
                Ok(None)
            } else {
                Err(ParseError::MalformedField)
            };
        };
        let Some((checksum_value_start, checksum_value_len)) = checksum_value else {
            return Err(ParseError::InvalidChecksum);
        };

        let consumed = pos;

        // Validate body length
        if self.validate_body_length && body_start > 0 {
            // Body length is from after "9=NNN\x01" to before "10=XXX\x01".
            let actual_body_length = checksum_tag_start - body_start;
            if actual_body_length != stated_body_length {
                return Err(ParseError::InvalidBodyLength);
            }
        }

        // Validate checksum
        if self.validate_checksum {
            let valid = if checksum_value_len == 3 {
                let stated_bytes = &buffer[checksum_value_start..checksum_value_start + 3];
                if stated_bytes.iter().all(|b| b.is_ascii_digit()) {
                    let computed = checksum::compute(&buffer[..checksum_tag_start]);
                    let stated = (stated_bytes[0] - b'0') * 100
                        + (stated_bytes[1] - b'0') * 10
                        + (stated_bytes[2] - b'0');
                    computed == stated
                } else {
                    false
                }
            } else {
                false
            };
            view.set_checksum_valid(valid);
            if !valid {
                return Err(ParseError::InvalidChecksum);
            }
        } else {
            view.set_checksum_valid(true);
        }

        Ok(Some((view, consumed)))
    }

    /// Attempt to find a complete message in the buffer.
    /// Returns `Some(length)` if a complete message is found, `None` otherwise.
    /// Useful for framing on a TCP stream.
    #[inline]
    pub fn find_message_boundary(&self, buffer: &[u8]) -> Option<usize> {
        // Look for "10=XXX\x01" pattern (checksum trailer)
        let len = buffer.len();
        if len < 7 {
            return None;
        }

        // Scan for "\x0110=XXX\x01" pattern (SOH + "10=" + 3 digits + SOH = 8 bytes)
        let mut i = 0;
        while i + 8 <= len {
            if buffer[i] == SOH
                && buffer[i + 1] == b'1'
                && buffer[i + 2] == b'0'
                && buffer[i + 3] == b'='
                && buffer[i + 7] == SOH
            {
                return Some(i + 8);
            }
            i += 1;
        }

        None
    }
}

impl Default for FixParser {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn parse_usize_ascii(bytes: &[u8]) -> Option<usize> {
    if bytes.is_empty() {
        return None;
    }

    let mut value = 0usize;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value * 10 + (b - b'0') as usize;
    }

    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_fix_msg(body_fields: &str) -> Vec<u8> {
        let soh = '\x01';
        let begin = format!("8=FIX.4.4{soh}");
        let body = format!("{body_fields}");
        let body_len = body.len();
        let header = format!("{begin}9={body_len}{soh}");
        let pre_checksum = format!("{header}{body}");
        let cs = checksum::compute(pre_checksum.as_bytes());
        let mut buf = [0u8; 3];
        checksum::format(cs, &mut buf);
        let cs_str = std::str::from_utf8(&buf).unwrap();
        let full = format!("{pre_checksum}10={cs_str}{soh}");
        full.into_bytes()
    }

    #[test]
    fn test_parse_heartbeat() {
        let msg = build_fix_msg(&format!(
            "35=0\x0149=SENDER\x0156=TARGET\x0134=1\x0152=20260321-10:00:00\x01"
        ));

        let parser = FixParser::new();
        let (view, consumed) = parser.parse(&msg).expect("should parse heartbeat");

        assert_eq!(consumed, msg.len());
        assert_eq!(view.msg_type(), Some(b"0".as_slice()));
        assert_eq!(view.begin_string(), Some("FIX.4.4"));
        assert_eq!(view.sender_comp_id(), Some("SENDER"));
        assert_eq!(view.target_comp_id(), Some("TARGET"));
        assert_eq!(view.msg_seq_num(), Some(1));
        assert!(view.is_checksum_valid());
    }

    #[test]
    fn test_parse_new_order_single() {
        let msg = build_fix_msg(&format!(
            "35=D\x0149=BANK\x0156=NYSE\x0134=42\x0152=20260321-10:00:00\x01\
             11=ORD001\x0155=AAPL\x0154=1\x0138=1000\x0140=2\x0144=150.50\x0159=0\x01\
             60=20260321-10:00:00\x01"
        ));

        let parser = FixParser::new();
        let (view, _consumed) = parser.parse(&msg).expect("should parse NOS");

        assert_eq!(view.msg_type(), Some(b"D".as_slice()));
        assert_eq!(view.get_field_str(tags::CL_ORD_ID), Some("ORD001"));
        assert_eq!(view.get_field_str(tags::SYMBOL), Some("AAPL"));
        assert_eq!(view.get_field_i64(tags::ORDER_QTY), Some(1000));
        assert_eq!(view.get_field_str(tags::PRICE), Some("150.50"));
    }

    #[test]
    fn test_parse_unchecked() {
        let msg = build_fix_msg(&format!(
            "35=0\x0149=S\x0156=T\x0134=1\x0152=20260321-10:00:00\x01"
        ));

        let parser = FixParser::new_unchecked();
        let (view, _) = parser.parse(&msg).expect("should parse without validation");
        assert_eq!(view.msg_type(), Some(b"0".as_slice()));
    }

    #[test]
    fn test_parse_invalid_checksum() {
        let mut msg = build_fix_msg(&format!(
            "35=0\x0149=S\x0156=T\x0134=1\x0152=20260321-10:00:00\x01"
        ));

        // Corrupt the checksum
        let len = msg.len();
        msg[len - 4] = b'0';
        msg[len - 3] = b'0';
        msg[len - 2] = b'0';

        let parser = FixParser::new();
        let result = parser.parse(&msg);
        assert_eq!(result.unwrap_err(), ParseError::InvalidChecksum);
    }

    #[test]
    fn test_parse_missing_begin_string() {
        let msg = b"9=5\x0135=0\x0110=000\x01xxxxxxxxxxxxxxx";
        let parser = FixParser::new();
        let result = parser.parse(msg);
        assert_eq!(result.unwrap_err(), ParseError::MissingBeginString);
    }

    #[test]
    fn test_parse_buffer_too_short() {
        let parser = FixParser::new();
        assert_eq!(
            parser.parse(b"8=FIX").unwrap_err(),
            ParseError::BufferTooShort
        );
    }

    #[test]
    fn test_find_message_boundary() {
        let msg = build_fix_msg(&format!(
            "35=0\x0149=S\x0156=T\x0134=1\x0152=20260321-10:00:00\x01"
        ));

        let parser = FixParser::new();
        let boundary = parser.find_message_boundary(&msg);
        assert_eq!(boundary, Some(msg.len()));
    }

    #[test]
    fn test_try_parse_prefix_incomplete_returns_none() {
        let parser = FixParser::new();
        assert!(parser.try_parse_prefix(b"8=FIX.4.4\x01").unwrap().is_none());
    }

    #[test]
    fn test_try_parse_prefix_consumes_only_first_message() {
        let first = build_fix_msg(&format!(
            "35=0\x0149=S\x0156=T\x0134=1\x0152=20260321-10:00:00\x01"
        ));
        let second = build_fix_msg(&format!(
            "35=0\x0149=S\x0156=T\x0134=2\x0152=20260321-10:00:01\x01"
        ));
        let mut buffer = first.clone();
        buffer.extend_from_slice(&second);

        let parser = FixParser::new();
        let (view, consumed) = parser.try_parse_prefix(&buffer).unwrap().unwrap();

        assert_eq!(consumed, first.len());
        assert_eq!(view.msg_seq_num(), Some(1));
    }

    #[test]
    fn test_parse_execution_report() {
        let msg = build_fix_msg(&format!(
            "35=8\x0149=NYSE\x0156=BANK\x0134=100\x0152=20260321-10:00:00.123\x01\
             37=EXORD001\x0117=EXEC001\x01150=0\x0139=0\x0155=AAPL\x0154=1\x01\
             38=1000\x0132=0\x0131=0\x01151=1000\x0114=0\x016=0\x01"
        ));

        let parser = FixParser::new();
        let (view, _) = parser.parse(&msg).expect("should parse ExecRpt");

        assert_eq!(view.msg_type(), Some(b"8".as_slice()));
        assert_eq!(view.get_field_str(tags::ORDER_ID), Some("EXORD001"));
        assert_eq!(view.get_field_str(tags::EXEC_ID), Some("EXEC001"));
        assert_eq!(view.get_field_i64(tags::ORDER_QTY), Some(1000));
        assert!(view.field_count() >= 15);
    }
}
