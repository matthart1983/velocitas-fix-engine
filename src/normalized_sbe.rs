use std::io;

use velocitas_fix_sbe::message_header_codec::{
    MessageHeaderDecoder, ENCODED_LENGTH as SBE_HEADER_LENGTH,
};
use velocitas_fix_sbe::normalized_execution_report_codec::{
    NormalizedExecutionReportDecoder, NormalizedExecutionReportEncoder,
};
use velocitas_fix_sbe::normalized_order_codec::{NormalizedOrderDecoder, NormalizedOrderEncoder};
use velocitas_fix_sbe::{Encoder, ReadBuf, WriteBuf, SBE_SCHEMA_ID};

use crate::message::MessageView;
use crate::tags;

const VAR_DATA_LENGTH_PREFIX: usize = 4;
const NORMALIZED_ORDER_VAR_DATA_FIELDS: usize = 5;
const NORMALIZED_EXECUTION_REPORT_VAR_DATA_FIELDS: usize = 6;

pub const NORMALIZED_ORDER_SBE_OVERHEAD: usize = SBE_HEADER_LENGTH
    + velocitas_fix_sbe::normalized_order_codec::SBE_BLOCK_LENGTH as usize
    + (NORMALIZED_ORDER_VAR_DATA_FIELDS * VAR_DATA_LENGTH_PREFIX);
pub const NORMALIZED_EXECUTION_REPORT_SBE_OVERHEAD: usize = SBE_HEADER_LENGTH
    + velocitas_fix_sbe::normalized_execution_report_codec::SBE_BLOCK_LENGTH as usize
    + (NORMALIZED_EXECUTION_REPORT_VAR_DATA_FIELDS * VAR_DATA_LENGTH_PREFIX);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedOrder {
    pub cl_ord_id: String,
    pub symbol: String,
    pub side: u8,
    pub qty: i64,
    pub price: String,
    pub sender_comp_id: String,
    pub target_comp_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedExecutionReport {
    pub cl_ord_id: String,
    pub symbol: String,
    pub side: u8,
    pub order_qty: i64,
    pub last_qty: i64,
    pub last_px: String,
    pub leaves_qty: i64,
    pub cum_qty: i64,
    pub avg_px: String,
    pub order_id: String,
    pub exec_id: String,
    pub exec_type: u8,
    pub ord_status: u8,
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn ensure_var_data_len(value: &str) -> io::Result<()> {
    if value.len() > (u32::MAX - 1) as usize {
        return Err(invalid_input(
            "normalized field exceeds SBE varData capacity",
        ));
    }

    Ok(())
}

fn decode_utf8(bytes: &[u8]) -> io::Result<String> {
    String::from_utf8(bytes.to_vec()).map_err(|error| invalid_data(error.to_string()))
}

fn validate_header(
    frame: &[u8],
    expected_template_id: u16,
) -> io::Result<MessageHeaderDecoder<ReadBuf<'_>>> {
    if frame.len() < SBE_HEADER_LENGTH {
        return Err(invalid_data(
            "frame is too small to contain an SBE message header",
        ));
    }

    let header = MessageHeaderDecoder::default().wrap(ReadBuf::new(frame), 0);
    let template_id = header.template_id();
    let schema_id = header.schema_id();
    if template_id != expected_template_id || schema_id != SBE_SCHEMA_ID {
        return Err(invalid_data(format!(
            "unexpected SBE message header: template_id={template_id}, schema_id={schema_id}",
        )));
    }

    Ok(header)
}

pub fn encoded_normalized_order_len(order: &NormalizedOrder) -> io::Result<usize> {
    for value in [
        order.cl_ord_id.as_str(),
        order.symbol.as_str(),
        order.price.as_str(),
        order.sender_comp_id.as_str(),
        order.target_comp_id.as_str(),
    ] {
        ensure_var_data_len(value)?;
    }

    Ok(NORMALIZED_ORDER_SBE_OVERHEAD
        + order.cl_ord_id.len()
        + order.symbol.len()
        + order.price.len()
        + order.sender_comp_id.len()
        + order.target_comp_id.len())
}

pub fn encode_normalized_order_as_sbe(
    frame_buf: &mut Vec<u8>,
    order: &NormalizedOrder,
) -> io::Result<usize> {
    let encoded_len = encoded_normalized_order_len(order)?;
    if frame_buf.len() < encoded_len {
        frame_buf.resize(encoded_len, 0);
    }

    let mut encoder = NormalizedOrderEncoder::default();
    encoder = encoder.wrap(WriteBuf::new(frame_buf.as_mut_slice()), SBE_HEADER_LENGTH);
    encoder = encoder
        .header(0)
        .parent()
        .map_err(|err| invalid_data(format!("failed to finalize SBE header: {err}")))?;
    encoder.side(order.side);
    encoder.qty(order.qty);
    encoder.cl_ord_id(&order.cl_ord_id);
    encoder.symbol(&order.symbol);
    encoder.price(&order.price);
    encoder.sender_comp_id(&order.sender_comp_id);
    encoder.target_comp_id(&order.target_comp_id);

    let actual_len = encoder.get_limit();
    if actual_len != encoded_len {
        return Err(invalid_data(format!(
            "unexpected normalized order SBE length: expected {encoded_len}, got {actual_len}",
        )));
    }

    Ok(actual_len)
}

pub fn decode_normalized_order_from_sbe(frame: &[u8]) -> io::Result<NormalizedOrder> {
    let header = validate_header(
        frame,
        velocitas_fix_sbe::normalized_order_codec::SBE_TEMPLATE_ID,
    )?;
    let mut decoder = NormalizedOrderDecoder::default().header(header, 0);

    let cl_ord_id = {
        let coordinates = decoder.cl_ord_id_decoder();
        decode_utf8(decoder.cl_ord_id_slice(coordinates))?
    };
    let symbol = {
        let coordinates = decoder.symbol_decoder();
        decode_utf8(decoder.symbol_slice(coordinates))?
    };
    let price = {
        let coordinates = decoder.price_decoder();
        decode_utf8(decoder.price_slice(coordinates))?
    };
    let sender_comp_id = {
        let coordinates = decoder.sender_comp_id_decoder();
        decode_utf8(decoder.sender_comp_id_slice(coordinates))?
    };
    let target_comp_id = {
        let coordinates = decoder.target_comp_id_decoder();
        decode_utf8(decoder.target_comp_id_slice(coordinates))?
    };

    Ok(NormalizedOrder {
        cl_ord_id,
        symbol,
        side: decoder.side(),
        qty: decoder.qty(),
        price,
        sender_comp_id,
        target_comp_id,
    })
}

pub fn encoded_normalized_execution_report_len(
    report: &NormalizedExecutionReport,
) -> io::Result<usize> {
    for value in [
        report.cl_ord_id.as_str(),
        report.symbol.as_str(),
        report.last_px.as_str(),
        report.avg_px.as_str(),
        report.order_id.as_str(),
        report.exec_id.as_str(),
    ] {
        ensure_var_data_len(value)?;
    }

    Ok(NORMALIZED_EXECUTION_REPORT_SBE_OVERHEAD
        + report.cl_ord_id.len()
        + report.symbol.len()
        + report.last_px.len()
        + report.avg_px.len()
        + report.order_id.len()
        + report.exec_id.len())
}

pub fn encode_normalized_execution_report_as_sbe(
    frame_buf: &mut Vec<u8>,
    report: &NormalizedExecutionReport,
) -> io::Result<usize> {
    let encoded_len = encoded_normalized_execution_report_len(report)?;
    if frame_buf.len() < encoded_len {
        frame_buf.resize(encoded_len, 0);
    }

    let mut encoder = NormalizedExecutionReportEncoder::default();
    encoder = encoder.wrap(WriteBuf::new(frame_buf.as_mut_slice()), SBE_HEADER_LENGTH);
    encoder = encoder
        .header(0)
        .parent()
        .map_err(|err| invalid_data(format!("failed to finalize SBE header: {err}")))?;
    encoder.side(report.side);
    encoder.order_qty(report.order_qty);
    encoder.last_qty(report.last_qty);
    encoder.leaves_qty(report.leaves_qty);
    encoder.cum_qty(report.cum_qty);
    encoder.exec_type(report.exec_type);
    encoder.ord_status(report.ord_status);
    encoder.cl_ord_id(&report.cl_ord_id);
    encoder.symbol(&report.symbol);
    encoder.last_px(&report.last_px);
    encoder.avg_px(&report.avg_px);
    encoder.order_id(&report.order_id);
    encoder.exec_id(&report.exec_id);

    let actual_len = encoder.get_limit();
    if actual_len != encoded_len {
        return Err(invalid_data(format!(
            "unexpected normalized execution report SBE length: expected {encoded_len}, got {actual_len}",
        )));
    }

    Ok(actual_len)
}

pub fn decode_normalized_execution_report_from_sbe(
    frame: &[u8],
) -> io::Result<NormalizedExecutionReport> {
    let header = validate_header(
        frame,
        velocitas_fix_sbe::normalized_execution_report_codec::SBE_TEMPLATE_ID,
    )?;
    let mut decoder = NormalizedExecutionReportDecoder::default().header(header, 0);

    let cl_ord_id = {
        let coordinates = decoder.cl_ord_id_decoder();
        decode_utf8(decoder.cl_ord_id_slice(coordinates))?
    };
    let symbol = {
        let coordinates = decoder.symbol_decoder();
        decode_utf8(decoder.symbol_slice(coordinates))?
    };
    let last_px = {
        let coordinates = decoder.last_px_decoder();
        decode_utf8(decoder.last_px_slice(coordinates))?
    };
    let avg_px = {
        let coordinates = decoder.avg_px_decoder();
        decode_utf8(decoder.avg_px_slice(coordinates))?
    };
    let order_id = {
        let coordinates = decoder.order_id_decoder();
        decode_utf8(decoder.order_id_slice(coordinates))?
    };
    let exec_id = {
        let coordinates = decoder.exec_id_decoder();
        decode_utf8(decoder.exec_id_slice(coordinates))?
    };

    Ok(NormalizedExecutionReport {
        cl_ord_id,
        symbol,
        side: decoder.side(),
        order_qty: decoder.order_qty(),
        last_qty: decoder.last_qty(),
        last_px,
        leaves_qty: decoder.leaves_qty(),
        cum_qty: decoder.cum_qty(),
        avg_px,
        order_id,
        exec_id,
        exec_type: decoder.exec_type(),
        ord_status: decoder.ord_status(),
    })
}

impl NormalizedOrder {
    pub fn from_fix(msg: &MessageView<'_>) -> io::Result<Self> {
        Ok(Self {
            cl_ord_id: msg
                .get_field_str(tags::CL_ORD_ID)
                .unwrap_or("?")
                .to_string(),
            symbol: msg.get_field_str(tags::SYMBOL).unwrap_or("?").to_string(),
            side: msg
                .get_field_str(tags::SIDE)
                .and_then(|value| value.as_bytes().first().copied())
                .unwrap_or(b'?'),
            qty: msg.get_field_i64(tags::ORDER_QTY).unwrap_or(0),
            price: msg.get_field_str(tags::PRICE).unwrap_or("0").to_string(),
            sender_comp_id: msg.sender_comp_id().unwrap_or("?").to_string(),
            target_comp_id: msg.target_comp_id().unwrap_or("?").to_string(),
        })
    }
}

impl NormalizedExecutionReport {
    pub fn from_order(order: &NormalizedOrder) -> Self {
        Self {
            cl_ord_id: order.cl_ord_id.clone(),
            symbol: order.symbol.clone(),
            side: order.side,
            order_qty: order.qty,
            last_qty: order.qty,
            last_px: order.price.clone(),
            leaves_qty: 0,
            cum_qty: order.qty,
            avg_px: order.price.clone(),
            order_id: format!("VENUE-{}", order.cl_ord_id),
            exec_id: format!("EXEC-{}", order.cl_ord_id),
            exec_type: b'F',
            ord_status: b'2',
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_order_sbe_round_trip() {
        let order = NormalizedOrder {
            cl_ord_id: "BPIPE-ORD-0001".into(),
            symbol: "EUR/USD".into(),
            side: b'1',
            qty: 5_000_000,
            price: "1.08255".into(),
            sender_comp_id: "BLOOMBERG_FX".into(),
            target_comp_id: "VELOCITAS_GATEWAY".into(),
        };

        let mut buffer = Vec::new();
        let len = encode_normalized_order_as_sbe(&mut buffer, &order).unwrap();
        let decoded = decode_normalized_order_from_sbe(&buffer[..len]).unwrap();

        assert_eq!(decoded, order);
    }

    #[test]
    fn normalized_execution_report_sbe_round_trip() {
        let report = NormalizedExecutionReport {
            cl_ord_id: "BPIPE-ORD-0001".into(),
            symbol: "EUR/USD".into(),
            side: b'1',
            order_qty: 5_000_000,
            last_qty: 5_000_000,
            last_px: "1.08255".into(),
            leaves_qty: 0,
            cum_qty: 5_000_000,
            avg_px: "1.08255".into(),
            order_id: "VENUE-BPIPE-ORD-0001".into(),
            exec_id: "EXEC-BPIPE-ORD-0001".into(),
            exec_type: b'F',
            ord_status: b'2',
        };

        let mut buffer = Vec::new();
        let len = encode_normalized_execution_report_as_sbe(&mut buffer, &report).unwrap();
        let decoded = decode_normalized_execution_report_from_sbe(&buffer[..len]).unwrap();

        assert_eq!(decoded, report);
    }
}
