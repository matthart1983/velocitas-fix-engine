/// FIX checksum computation — optimized with loop unrolling.
/// Checksum is the sum of all bytes modulo 256, formatted as 3-digit string.

/// Compute FIX checksum over a byte slice.
/// Uses 4-way accumulator to exploit instruction-level parallelism.
#[inline]
pub fn compute(data: &[u8]) -> u8 {
    let len = data.len();
    let mut sum0: u32 = 0;
    let mut sum1: u32 = 0;
    let mut sum2: u32 = 0;
    let mut sum3: u32 = 0;

    let chunks = len / 4;
    let remainder = len % 4;

    let mut i = 0;
    for _ in 0..chunks {
        sum0 = sum0.wrapping_add(data[i] as u32);
        sum1 = sum1.wrapping_add(data[i + 1] as u32);
        sum2 = sum2.wrapping_add(data[i + 2] as u32);
        sum3 = sum3.wrapping_add(data[i + 3] as u32);
        i += 4;
    }

    for j in 0..remainder {
        sum0 = sum0.wrapping_add(data[i + j] as u32);
    }

    ((sum0
        .wrapping_add(sum1)
        .wrapping_add(sum2)
        .wrapping_add(sum3))
        % 256) as u8
}

/// Format checksum as 3-character zero-padded string into the provided buffer.
/// Returns the 3 bytes written.
#[inline]
pub fn format(checksum: u8, buf: &mut [u8; 3]) {
    buf[0] = b'0' + (checksum / 100);
    buf[1] = b'0' + ((checksum / 10) % 10);
    buf[2] = b'0' + (checksum % 10);
}

/// Validate checksum of a complete FIX message (including the 10=xxx| trailer).
/// The checksum is computed over all bytes up to (but not including) the "10=" tag.
#[inline]
pub fn validate(msg: &[u8]) -> bool {
    // Find the last "10=" which starts the checksum field
    // Search backwards for SOH followed by "10="
    let len = msg.len();
    if len < 7 {
        return false; // Minimum: "10=000" + SOH
    }

    let mut checksum_start = 0;
    for i in (0..len - 4).rev() {
        if msg[i] == 0x01 && msg[i + 1] == b'1' && msg[i + 2] == b'0' && msg[i + 3] == b'=' {
            checksum_start = i + 1;
            break;
        }
    }

    // Also check if message starts with 10= (edge case, shouldn't happen)
    if checksum_start == 0 {
        if msg[0] == b'1' && msg[1] == b'0' && msg[2] == b'=' {
            checksum_start = 0;
        } else {
            return false;
        }
    }

    // Compute checksum up to and including the SOH before "10="
    let computed = compute(&msg[..checksum_start - 1 + 1]); // include the SOH before 10=

    // Extract stated checksum
    let cs_offset = checksum_start + 3; // skip "10="
    if cs_offset + 3 > len {
        return false;
    }

    let stated = (msg[cs_offset] - b'0') as u8 * 100
        + (msg[cs_offset + 1] - b'0') as u8 * 10
        + (msg[cs_offset + 2] - b'0') as u8;

    computed == stated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checksum_compute() {
        // "8=FIX.4.4" + SOH = bytes
        let data = b"8=FIX.4.4\x019=5\x0135=0\x01";
        let cs = compute(data);
        assert_eq!(cs, 163);
    }

    #[test]
    fn test_checksum_format() {
        let mut buf = [0u8; 3];
        format(7, &mut buf);
        assert_eq!(&buf, b"007");

        format(42, &mut buf);
        assert_eq!(&buf, b"042");

        format(255, &mut buf);
        assert_eq!(&buf, b"255");

        format(100, &mut buf);
        assert_eq!(&buf, b"100");
    }

    #[test]
    fn test_checksum_empty() {
        assert_eq!(compute(b""), 0);
    }

    #[test]
    fn test_checksum_known_value() {
        // Sum of ASCII values of "ABC" = 65+66+67 = 198
        assert_eq!(compute(b"ABC"), 198);
    }
}
