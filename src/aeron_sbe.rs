use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use velocitas_fix_sbe::fix_aeron_envelope_codec::{
    FixAeronEnvelopeDecoder, FixAeronEnvelopeEncoder, SBE_BLOCK_LENGTH, SBE_TEMPLATE_ID,
};
use velocitas_fix_sbe::message_header_codec::{
    MessageHeaderDecoder, ENCODED_LENGTH as SBE_HEADER_LENGTH,
};
use velocitas_fix_sbe::{Encoder, ReadBuf, WriteBuf, SBE_SCHEMA_ID};

const VAR_DATA_LENGTH_PREFIX: usize = 4;
pub(crate) const FIX_AERON_ENVELOPE_OVERHEAD: usize =
    SBE_HEADER_LENGTH + SBE_BLOCK_LENGTH as usize + VAR_DATA_LENGTH_PREFIX;

#[derive(Debug)]
pub(crate) struct DecodedFixEnvelope<'a> {
    pub base_stream_id: i32,
    pub payload: &'a [u8],
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn now_unix_timestamp_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

pub(crate) fn encoded_envelope_len(payload_len: usize) -> io::Result<usize> {
    if payload_len > (u32::MAX - 1) as usize {
        return Err(invalid_input(
            "FIX payload is too large for SBE varData encoding",
        ));
    }

    Ok(FIX_AERON_ENVELOPE_OVERHEAD + payload_len)
}

pub(crate) fn encode_fix_payload_as_sbe(
    frame_buf: &mut Vec<u8>,
    base_stream_id: i32,
    payload: &[u8],
) -> io::Result<usize> {
    let encoded_len = encoded_envelope_len(payload.len())?;
    if frame_buf.len() < encoded_len {
        frame_buf.resize(encoded_len, 0);
    }

    let mut encoder = FixAeronEnvelopeEncoder::default();
    encoder = encoder.wrap(WriteBuf::new(frame_buf.as_mut_slice()), SBE_HEADER_LENGTH);
    encoder = encoder
        .header(0)
        .parent()
        .map_err(|err| invalid_data(format!("failed to finalize SBE header: {err}")))?;
    encoder.published_at(now_unix_timestamp_ns());
    encoder.base_stream_id(base_stream_id);
    encoder.payload(payload);

    let actual_len = encoder.get_limit();
    if actual_len != encoded_len {
        return Err(invalid_data(format!(
            "unexpected SBE envelope length: expected {encoded_len}, got {actual_len}"
        )));
    }

    Ok(actual_len)
}

pub(crate) fn decode_fix_payload_from_sbe(frame: &[u8]) -> io::Result<DecodedFixEnvelope<'_>> {
    if frame.len() < FIX_AERON_ENVELOPE_OVERHEAD {
        return Err(invalid_data(
            "Aeron frame is too small to contain an SBE envelope",
        ));
    }

    let header = MessageHeaderDecoder::default().wrap(ReadBuf::new(frame), 0);
    let template_id = header.template_id();
    let schema_id = header.schema_id();
    if template_id != SBE_TEMPLATE_ID || schema_id != SBE_SCHEMA_ID {
        return Err(invalid_data(format!(
            "unexpected SBE envelope header: template_id={template_id}, schema_id={schema_id}"
        )));
    }

    let mut decoder = FixAeronEnvelopeDecoder::default().header(header, 0);
    let base_stream_id = decoder.base_stream_id();
    let payload_coordinates = decoder.payload_decoder();
    let payload_end = payload_coordinates.0 + payload_coordinates.1;
    if payload_end > frame.len() {
        return Err(invalid_data(
            "SBE envelope payload length exceeds Aeron frame size",
        ));
    }

    let payload = &frame[payload_coordinates.0..payload_end];
    Ok(DecodedFixEnvelope {
        base_stream_id,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fix_payload_sbe_round_trip() {
        let mut buffer = Vec::new();
        let payload = b"8=FIX.4.4\x019=5\x0135=0\x0110=000\x01";

        let len = encode_fix_payload_as_sbe(&mut buffer, 42_001, payload).unwrap();
        let decoded = decode_fix_payload_from_sbe(&buffer[..len]).unwrap();

        assert_eq!(decoded.base_stream_id, 42_001);
        assert_eq!(decoded.payload, payload);
    }

    #[test]
    fn test_fix_payload_sbe_rejects_short_frames() {
        let err = decode_fix_payload_from_sbe(&[0u8; 4]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
