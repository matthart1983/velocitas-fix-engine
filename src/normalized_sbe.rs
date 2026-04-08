use std::io;

use velocitas_fix_sbe::message_header_codec::{
    MessageHeaderDecoder, ENCODED_LENGTH as SBE_HEADER_LENGTH,
};
use velocitas_fix_sbe::normalized_cancel_reject_codec::{
    NormalizedCancelRejectDecoder, NormalizedCancelRejectEncoder,
};
use velocitas_fix_sbe::normalized_cancel_request_codec::{
    NormalizedCancelRequestDecoder, NormalizedCancelRequestEncoder,
};
use velocitas_fix_sbe::normalized_execution_report_codec::{
    NormalizedExecutionReportDecoder, NormalizedExecutionReportEncoder,
};
use velocitas_fix_sbe::normalized_order_codec::{NormalizedOrderDecoder, NormalizedOrderEncoder};
use velocitas_fix_sbe::normalized_order_status_request_codec::{
    NormalizedOrderStatusRequestDecoder, NormalizedOrderStatusRequestEncoder,
};
use velocitas_fix_sbe::normalized_replace_request_codec::{
    NormalizedReplaceRequestDecoder, NormalizedReplaceRequestEncoder,
};
use velocitas_fix_sbe::{Encoder, ReadBuf, WriteBuf, SBE_SCHEMA_ID};

use crate::message::MessageView;
use crate::tags;

const VAR_DATA_LENGTH_PREFIX: usize = 4;
const NORMALIZED_ORDER_VAR_DATA_FIELDS: usize = 5;
const NORMALIZED_EXECUTION_REPORT_VAR_DATA_FIELDS: usize = 6;
const NORMALIZED_CANCEL_REQUEST_VAR_DATA_FIELDS: usize = 5;
const NORMALIZED_REPLACE_REQUEST_VAR_DATA_FIELDS: usize = 6;
const NORMALIZED_ORDER_STATUS_REQUEST_VAR_DATA_FIELDS: usize = 5;
const NORMALIZED_CANCEL_REJECT_VAR_DATA_FIELDS: usize = 4;

pub const NORMALIZED_ORDER_SBE_OVERHEAD: usize = SBE_HEADER_LENGTH
    + velocitas_fix_sbe::normalized_order_codec::SBE_BLOCK_LENGTH as usize
    + (NORMALIZED_ORDER_VAR_DATA_FIELDS * VAR_DATA_LENGTH_PREFIX);
pub const NORMALIZED_EXECUTION_REPORT_SBE_OVERHEAD: usize = SBE_HEADER_LENGTH
    + velocitas_fix_sbe::normalized_execution_report_codec::SBE_BLOCK_LENGTH as usize
    + (NORMALIZED_EXECUTION_REPORT_VAR_DATA_FIELDS * VAR_DATA_LENGTH_PREFIX);
pub const NORMALIZED_CANCEL_REQUEST_SBE_OVERHEAD: usize = SBE_HEADER_LENGTH
    + velocitas_fix_sbe::normalized_cancel_request_codec::SBE_BLOCK_LENGTH as usize
    + (NORMALIZED_CANCEL_REQUEST_VAR_DATA_FIELDS * VAR_DATA_LENGTH_PREFIX);
pub const NORMALIZED_REPLACE_REQUEST_SBE_OVERHEAD: usize = SBE_HEADER_LENGTH
    + velocitas_fix_sbe::normalized_replace_request_codec::SBE_BLOCK_LENGTH as usize
    + (NORMALIZED_REPLACE_REQUEST_VAR_DATA_FIELDS * VAR_DATA_LENGTH_PREFIX);
pub const NORMALIZED_ORDER_STATUS_REQUEST_SBE_OVERHEAD: usize = SBE_HEADER_LENGTH
    + velocitas_fix_sbe::normalized_order_status_request_codec::SBE_BLOCK_LENGTH as usize
    + (NORMALIZED_ORDER_STATUS_REQUEST_VAR_DATA_FIELDS * VAR_DATA_LENGTH_PREFIX);
pub const NORMALIZED_CANCEL_REJECT_SBE_OVERHEAD: usize = SBE_HEADER_LENGTH
    + velocitas_fix_sbe::normalized_cancel_reject_codec::SBE_BLOCK_LENGTH as usize
    + (NORMALIZED_CANCEL_REJECT_VAR_DATA_FIELDS * VAR_DATA_LENGTH_PREFIX);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizedOrderView<'a> {
    pub cl_ord_id: &'a str,
    pub symbol: &'a str,
    pub side: u8,
    pub qty: i64,
    pub price: &'a str,
    pub sender_comp_id: &'a str,
    pub target_comp_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedCancelRequest {
    pub cl_ord_id: String,
    pub orig_cl_ord_id: String,
    pub symbol: String,
    pub side: u8,
    pub qty: i64,
    pub sender_comp_id: String,
    pub target_comp_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizedCancelRequestView<'a> {
    pub cl_ord_id: &'a str,
    pub orig_cl_ord_id: &'a str,
    pub symbol: &'a str,
    pub side: u8,
    pub qty: i64,
    pub sender_comp_id: &'a str,
    pub target_comp_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedReplaceRequest {
    pub cl_ord_id: String,
    pub orig_cl_ord_id: String,
    pub symbol: String,
    pub side: u8,
    pub qty: i64,
    pub price: String,
    pub sender_comp_id: String,
    pub target_comp_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizedReplaceRequestView<'a> {
    pub cl_ord_id: &'a str,
    pub orig_cl_ord_id: &'a str,
    pub symbol: &'a str,
    pub side: u8,
    pub qty: i64,
    pub price: &'a str,
    pub sender_comp_id: &'a str,
    pub target_comp_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedOrderStatusRequest {
    pub cl_ord_id: String,
    pub order_id: String,
    pub symbol: String,
    pub side: u8,
    pub sender_comp_id: String,
    pub target_comp_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizedOrderStatusRequestView<'a> {
    pub cl_ord_id: &'a str,
    pub order_id: &'a str,
    pub symbol: &'a str,
    pub side: u8,
    pub sender_comp_id: &'a str,
    pub target_comp_id: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderLifecycleAction {
    Cancel,
    Replace,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizedOrderLifecycleRequest {
    Cancel(NormalizedCancelRequest),
    Replace(NormalizedReplaceRequest),
    Status(NormalizedOrderStatusRequest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizedOrderLifecycleRequestView<'a> {
    Cancel(NormalizedCancelRequestView<'a>),
    Replace(NormalizedReplaceRequestView<'a>),
    Status(NormalizedOrderStatusRequestView<'a>),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizedExecutionReportView<'a> {
    pub cl_ord_id: &'a str,
    pub symbol: &'a str,
    pub side: u8,
    pub order_qty: i64,
    pub last_qty: i64,
    pub last_px: &'a str,
    pub leaves_qty: i64,
    pub cum_qty: i64,
    pub avg_px: &'a str,
    pub order_id: &'a str,
    pub exec_id: &'a str,
    pub exec_type: u8,
    pub ord_status: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedCancelReject {
    pub order_id: String,
    pub cl_ord_id: String,
    pub orig_cl_ord_id: String,
    pub ord_status: u8,
    pub cxl_rej_response_to: u8,
    pub cxl_rej_reason: i32,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizedCancelRejectView<'a> {
    pub order_id: &'a str,
    pub cl_ord_id: &'a str,
    pub orig_cl_ord_id: &'a str,
    pub ord_status: u8,
    pub cxl_rej_response_to: u8,
    pub cxl_rej_reason: i32,
    pub text: &'a str,
}

#[derive(Debug, Default, Clone)]
pub struct NormalizedExecutionReportScratch {
    order_id: String,
    exec_id: String,
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

fn decode_utf8_str(bytes: &[u8]) -> io::Result<&str> {
    std::str::from_utf8(bytes).map_err(|error| invalid_data(error.to_string()))
}

fn decode_var_data_str(frame: &[u8], coordinates: (usize, usize)) -> io::Result<&str> {
    let end = coordinates
        .0
        .checked_add(coordinates.1)
        .ok_or_else(|| invalid_data("SBE varData coordinates overflow"))?;
    let bytes = frame
        .get(coordinates.0..end)
        .ok_or_else(|| invalid_data("SBE varData coordinates exceed frame length"))?;
    decode_utf8_str(bytes)
}

fn read_header(frame: &[u8]) -> io::Result<MessageHeaderDecoder<ReadBuf<'_>>> {
    if frame.len() < SBE_HEADER_LENGTH {
        return Err(invalid_data(
            "frame is too small to contain an SBE message header",
        ));
    }

    let header = MessageHeaderDecoder::default().wrap(ReadBuf::new(frame), 0);
    let schema_id = header.schema_id();
    if schema_id != SBE_SCHEMA_ID {
        return Err(invalid_data(format!(
            "unexpected SBE schema id in message header: {schema_id}",
        )));
    }

    Ok(header)
}

pub fn peek_sbe_template_id(frame: &[u8]) -> io::Result<u16> {
    Ok(read_header(frame)?.template_id())
}

impl OrderLifecycleAction {
    pub fn label(self) -> &'static str {
        match self {
            OrderLifecycleAction::Cancel => "cancel",
            OrderLifecycleAction::Replace => "replace",
            OrderLifecycleAction::Status => "status",
        }
    }

    pub fn from_fix_msg_type(msg_type: &[u8]) -> io::Result<Self> {
        match msg_type {
            b"F" => Ok(OrderLifecycleAction::Cancel),
            b"G" => Ok(OrderLifecycleAction::Replace),
            b"H" => Ok(OrderLifecycleAction::Status),
            _ => Err(invalid_input(format!(
                "unsupported FIX message type for lifecycle request: {}",
                String::from_utf8_lossy(msg_type),
            ))),
        }
    }

    pub fn from_cancel_reject_response_to(value: u8) -> io::Result<Self> {
        match value {
            b'1' => Ok(OrderLifecycleAction::Cancel),
            b'2' => Ok(OrderLifecycleAction::Replace),
            _ => Err(invalid_data(format!(
                "unsupported cancel reject response target: {}",
                char::from(value),
            ))),
        }
    }

    pub fn to_fix_cancel_reject_response_to(self) -> io::Result<u8> {
        match self {
            OrderLifecycleAction::Cancel => Ok(b'1'),
            OrderLifecycleAction::Replace => Ok(b'2'),
            OrderLifecycleAction::Status => Err(invalid_input(
                "OrderStatusRequest is not a valid cancel-reject response target",
            )),
        }
    }
}

fn validate_header(
    frame: &[u8],
    expected_template_id: u16,
) -> io::Result<MessageHeaderDecoder<ReadBuf<'_>>> {
    let header = read_header(frame)?;
    let template_id = header.template_id();
    if template_id != expected_template_id {
        return Err(invalid_data(format!(
            "unexpected SBE message header: template_id={template_id}",
        )));
    }

    Ok(header)
}

fn encoded_normalized_order_len_parts(
    cl_ord_id: &str,
    symbol: &str,
    price: &str,
    sender_comp_id: &str,
    target_comp_id: &str,
) -> io::Result<usize> {
    for value in [cl_ord_id, symbol, price, sender_comp_id, target_comp_id] {
        ensure_var_data_len(value)?;
    }

    Ok(NORMALIZED_ORDER_SBE_OVERHEAD
        + cl_ord_id.len()
        + symbol.len()
        + price.len()
        + sender_comp_id.len()
        + target_comp_id.len())
}

fn encoded_normalized_execution_report_len_parts(
    cl_ord_id: &str,
    symbol: &str,
    last_px: &str,
    avg_px: &str,
    order_id: &str,
    exec_id: &str,
) -> io::Result<usize> {
    for value in [cl_ord_id, symbol, last_px, avg_px, order_id, exec_id] {
        ensure_var_data_len(value)?;
    }

    Ok(NORMALIZED_EXECUTION_REPORT_SBE_OVERHEAD
        + cl_ord_id.len()
        + symbol.len()
        + last_px.len()
        + avg_px.len()
        + order_id.len()
        + exec_id.len())
}

fn encoded_normalized_cancel_request_len_parts(
    cl_ord_id: &str,
    orig_cl_ord_id: &str,
    symbol: &str,
    sender_comp_id: &str,
    target_comp_id: &str,
) -> io::Result<usize> {
    for value in [
        cl_ord_id,
        orig_cl_ord_id,
        symbol,
        sender_comp_id,
        target_comp_id,
    ] {
        ensure_var_data_len(value)?;
    }

    Ok(NORMALIZED_CANCEL_REQUEST_SBE_OVERHEAD
        + cl_ord_id.len()
        + orig_cl_ord_id.len()
        + symbol.len()
        + sender_comp_id.len()
        + target_comp_id.len())
}

fn encoded_normalized_replace_request_len_parts(
    cl_ord_id: &str,
    orig_cl_ord_id: &str,
    symbol: &str,
    price: &str,
    sender_comp_id: &str,
    target_comp_id: &str,
) -> io::Result<usize> {
    for value in [
        cl_ord_id,
        orig_cl_ord_id,
        symbol,
        price,
        sender_comp_id,
        target_comp_id,
    ] {
        ensure_var_data_len(value)?;
    }

    Ok(NORMALIZED_REPLACE_REQUEST_SBE_OVERHEAD
        + cl_ord_id.len()
        + orig_cl_ord_id.len()
        + symbol.len()
        + price.len()
        + sender_comp_id.len()
        + target_comp_id.len())
}

fn encoded_normalized_order_status_request_len_parts(
    cl_ord_id: &str,
    order_id: &str,
    symbol: &str,
    sender_comp_id: &str,
    target_comp_id: &str,
) -> io::Result<usize> {
    for value in [cl_ord_id, order_id, symbol, sender_comp_id, target_comp_id] {
        ensure_var_data_len(value)?;
    }

    Ok(NORMALIZED_ORDER_STATUS_REQUEST_SBE_OVERHEAD
        + cl_ord_id.len()
        + order_id.len()
        + symbol.len()
        + sender_comp_id.len()
        + target_comp_id.len())
}

fn encoded_normalized_cancel_reject_len_parts(
    order_id: &str,
    cl_ord_id: &str,
    orig_cl_ord_id: &str,
    text: &str,
) -> io::Result<usize> {
    for value in [order_id, cl_ord_id, orig_cl_ord_id, text] {
        ensure_var_data_len(value)?;
    }

    Ok(NORMALIZED_CANCEL_REJECT_SBE_OVERHEAD
        + order_id.len()
        + cl_ord_id.len()
        + orig_cl_ord_id.len()
        + text.len())
}

pub fn encoded_normalized_order_len(order: &NormalizedOrder) -> io::Result<usize> {
    encoded_normalized_order_len_parts(
        order.cl_ord_id.as_str(),
        order.symbol.as_str(),
        order.price.as_str(),
        order.sender_comp_id.as_str(),
        order.target_comp_id.as_str(),
    )
}

pub fn encode_normalized_order_as_sbe(
    frame_buf: &mut Vec<u8>,
    order: &NormalizedOrder,
) -> io::Result<usize> {
    let order_view: NormalizedOrderView<'_> = order.into();
    encode_normalized_order_view_as_sbe(frame_buf, &order_view)
}

pub fn encode_normalized_order_view_as_sbe(
    frame_buf: &mut Vec<u8>,
    order: &NormalizedOrderView<'_>,
) -> io::Result<usize> {
    let encoded_len = encoded_normalized_order_len_parts(
        order.cl_ord_id,
        order.symbol,
        order.price,
        order.sender_comp_id,
        order.target_comp_id,
    )?;
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
    encoder.cl_ord_id(order.cl_ord_id);
    encoder.symbol(order.symbol);
    encoder.price(order.price);
    encoder.sender_comp_id(order.sender_comp_id);
    encoder.target_comp_id(order.target_comp_id);

    let actual_len = encoder.get_limit();
    if actual_len != encoded_len {
        return Err(invalid_data(format!(
            "unexpected normalized order SBE length: expected {encoded_len}, got {actual_len}",
        )));
    }

    Ok(actual_len)
}

pub fn decode_normalized_order_from_sbe(frame: &[u8]) -> io::Result<NormalizedOrder> {
    Ok(decode_normalized_order_view_from_sbe(frame)?.into())
}

pub fn decode_normalized_order_view_from_sbe(frame: &[u8]) -> io::Result<NormalizedOrderView<'_>> {
    let header = validate_header(
        frame,
        velocitas_fix_sbe::normalized_order_codec::SBE_TEMPLATE_ID,
    )?;
    let mut decoder = NormalizedOrderDecoder::default().header(header, 0);
    let side = decoder.side();
    let qty = decoder.qty();
    let cl_ord_id_coordinates = decoder.cl_ord_id_decoder();
    let symbol_coordinates = decoder.symbol_decoder();
    let price_coordinates = decoder.price_decoder();
    let sender_comp_id_coordinates = decoder.sender_comp_id_decoder();
    let target_comp_id_coordinates = decoder.target_comp_id_decoder();

    let cl_ord_id = decode_var_data_str(frame, cl_ord_id_coordinates)?;
    let symbol = decode_var_data_str(frame, symbol_coordinates)?;
    let price = decode_var_data_str(frame, price_coordinates)?;
    let sender_comp_id = decode_var_data_str(frame, sender_comp_id_coordinates)?;
    let target_comp_id = decode_var_data_str(frame, target_comp_id_coordinates)?;

    Ok(NormalizedOrderView {
        cl_ord_id,
        symbol,
        side,
        qty,
        price,
        sender_comp_id,
        target_comp_id,
    })
}

pub fn encoded_normalized_cancel_request_len(
    request: &NormalizedCancelRequest,
) -> io::Result<usize> {
    encoded_normalized_cancel_request_len_parts(
        request.cl_ord_id.as_str(),
        request.orig_cl_ord_id.as_str(),
        request.symbol.as_str(),
        request.sender_comp_id.as_str(),
        request.target_comp_id.as_str(),
    )
}

pub fn encode_normalized_cancel_request_as_sbe(
    frame_buf: &mut Vec<u8>,
    request: &NormalizedCancelRequest,
) -> io::Result<usize> {
    let request_view: NormalizedCancelRequestView<'_> = request.into();
    encode_normalized_cancel_request_view_as_sbe(frame_buf, &request_view)
}

pub fn encode_normalized_cancel_request_view_as_sbe(
    frame_buf: &mut Vec<u8>,
    request: &NormalizedCancelRequestView<'_>,
) -> io::Result<usize> {
    let encoded_len = encoded_normalized_cancel_request_len_parts(
        request.cl_ord_id,
        request.orig_cl_ord_id,
        request.symbol,
        request.sender_comp_id,
        request.target_comp_id,
    )?;
    if frame_buf.len() < encoded_len {
        frame_buf.resize(encoded_len, 0);
    }

    let mut encoder = NormalizedCancelRequestEncoder::default();
    encoder = encoder.wrap(WriteBuf::new(frame_buf.as_mut_slice()), SBE_HEADER_LENGTH);
    encoder = encoder
        .header(0)
        .parent()
        .map_err(|err| invalid_data(format!("failed to finalize SBE header: {err}")))?;
    encoder.side(request.side);
    encoder.qty(request.qty);
    encoder.cl_ord_id(request.cl_ord_id);
    encoder.orig_cl_ord_id(request.orig_cl_ord_id);
    encoder.symbol(request.symbol);
    encoder.sender_comp_id(request.sender_comp_id);
    encoder.target_comp_id(request.target_comp_id);

    let actual_len = encoder.get_limit();
    if actual_len != encoded_len {
        return Err(invalid_data(format!(
            "unexpected normalized cancel request SBE length: expected {encoded_len}, got {actual_len}",
        )));
    }

    Ok(actual_len)
}

pub fn decode_normalized_cancel_request_from_sbe(
    frame: &[u8],
) -> io::Result<NormalizedCancelRequest> {
    Ok(decode_normalized_cancel_request_view_from_sbe(frame)?.into())
}

pub fn decode_normalized_cancel_request_view_from_sbe(
    frame: &[u8],
) -> io::Result<NormalizedCancelRequestView<'_>> {
    let header = validate_header(
        frame,
        velocitas_fix_sbe::normalized_cancel_request_codec::SBE_TEMPLATE_ID,
    )?;
    let mut decoder = NormalizedCancelRequestDecoder::default().header(header, 0);
    let side = decoder.side();
    let qty = decoder.qty();
    let cl_ord_id_coordinates = decoder.cl_ord_id_decoder();
    let orig_cl_ord_id_coordinates = decoder.orig_cl_ord_id_decoder();
    let symbol_coordinates = decoder.symbol_decoder();
    let sender_comp_id_coordinates = decoder.sender_comp_id_decoder();
    let target_comp_id_coordinates = decoder.target_comp_id_decoder();

    Ok(NormalizedCancelRequestView {
        cl_ord_id: decode_var_data_str(frame, cl_ord_id_coordinates)?,
        orig_cl_ord_id: decode_var_data_str(frame, orig_cl_ord_id_coordinates)?,
        symbol: decode_var_data_str(frame, symbol_coordinates)?,
        side,
        qty,
        sender_comp_id: decode_var_data_str(frame, sender_comp_id_coordinates)?,
        target_comp_id: decode_var_data_str(frame, target_comp_id_coordinates)?,
    })
}

pub fn encoded_normalized_replace_request_len(
    request: &NormalizedReplaceRequest,
) -> io::Result<usize> {
    encoded_normalized_replace_request_len_parts(
        request.cl_ord_id.as_str(),
        request.orig_cl_ord_id.as_str(),
        request.symbol.as_str(),
        request.price.as_str(),
        request.sender_comp_id.as_str(),
        request.target_comp_id.as_str(),
    )
}

pub fn encode_normalized_replace_request_as_sbe(
    frame_buf: &mut Vec<u8>,
    request: &NormalizedReplaceRequest,
) -> io::Result<usize> {
    let request_view: NormalizedReplaceRequestView<'_> = request.into();
    encode_normalized_replace_request_view_as_sbe(frame_buf, &request_view)
}

pub fn encode_normalized_replace_request_view_as_sbe(
    frame_buf: &mut Vec<u8>,
    request: &NormalizedReplaceRequestView<'_>,
) -> io::Result<usize> {
    let encoded_len = encoded_normalized_replace_request_len_parts(
        request.cl_ord_id,
        request.orig_cl_ord_id,
        request.symbol,
        request.price,
        request.sender_comp_id,
        request.target_comp_id,
    )?;
    if frame_buf.len() < encoded_len {
        frame_buf.resize(encoded_len, 0);
    }

    let mut encoder = NormalizedReplaceRequestEncoder::default();
    encoder = encoder.wrap(WriteBuf::new(frame_buf.as_mut_slice()), SBE_HEADER_LENGTH);
    encoder = encoder
        .header(0)
        .parent()
        .map_err(|err| invalid_data(format!("failed to finalize SBE header: {err}")))?;
    encoder.side(request.side);
    encoder.qty(request.qty);
    encoder.cl_ord_id(request.cl_ord_id);
    encoder.orig_cl_ord_id(request.orig_cl_ord_id);
    encoder.symbol(request.symbol);
    encoder.price(request.price);
    encoder.sender_comp_id(request.sender_comp_id);
    encoder.target_comp_id(request.target_comp_id);

    let actual_len = encoder.get_limit();
    if actual_len != encoded_len {
        return Err(invalid_data(format!(
            "unexpected normalized replace request SBE length: expected {encoded_len}, got {actual_len}",
        )));
    }

    Ok(actual_len)
}

pub fn decode_normalized_replace_request_from_sbe(
    frame: &[u8],
) -> io::Result<NormalizedReplaceRequest> {
    Ok(decode_normalized_replace_request_view_from_sbe(frame)?.into())
}

pub fn decode_normalized_replace_request_view_from_sbe(
    frame: &[u8],
) -> io::Result<NormalizedReplaceRequestView<'_>> {
    let header = validate_header(
        frame,
        velocitas_fix_sbe::normalized_replace_request_codec::SBE_TEMPLATE_ID,
    )?;
    let mut decoder = NormalizedReplaceRequestDecoder::default().header(header, 0);
    let side = decoder.side();
    let qty = decoder.qty();
    let cl_ord_id_coordinates = decoder.cl_ord_id_decoder();
    let orig_cl_ord_id_coordinates = decoder.orig_cl_ord_id_decoder();
    let symbol_coordinates = decoder.symbol_decoder();
    let price_coordinates = decoder.price_decoder();
    let sender_comp_id_coordinates = decoder.sender_comp_id_decoder();
    let target_comp_id_coordinates = decoder.target_comp_id_decoder();

    Ok(NormalizedReplaceRequestView {
        cl_ord_id: decode_var_data_str(frame, cl_ord_id_coordinates)?,
        orig_cl_ord_id: decode_var_data_str(frame, orig_cl_ord_id_coordinates)?,
        symbol: decode_var_data_str(frame, symbol_coordinates)?,
        side,
        qty,
        price: decode_var_data_str(frame, price_coordinates)?,
        sender_comp_id: decode_var_data_str(frame, sender_comp_id_coordinates)?,
        target_comp_id: decode_var_data_str(frame, target_comp_id_coordinates)?,
    })
}

pub fn encoded_normalized_order_status_request_len(
    request: &NormalizedOrderStatusRequest,
) -> io::Result<usize> {
    encoded_normalized_order_status_request_len_parts(
        request.cl_ord_id.as_str(),
        request.order_id.as_str(),
        request.symbol.as_str(),
        request.sender_comp_id.as_str(),
        request.target_comp_id.as_str(),
    )
}

pub fn encode_normalized_order_status_request_as_sbe(
    frame_buf: &mut Vec<u8>,
    request: &NormalizedOrderStatusRequest,
) -> io::Result<usize> {
    let request_view: NormalizedOrderStatusRequestView<'_> = request.into();
    encode_normalized_order_status_request_view_as_sbe(frame_buf, &request_view)
}

pub fn encode_normalized_order_status_request_view_as_sbe(
    frame_buf: &mut Vec<u8>,
    request: &NormalizedOrderStatusRequestView<'_>,
) -> io::Result<usize> {
    let encoded_len = encoded_normalized_order_status_request_len_parts(
        request.cl_ord_id,
        request.order_id,
        request.symbol,
        request.sender_comp_id,
        request.target_comp_id,
    )?;
    if frame_buf.len() < encoded_len {
        frame_buf.resize(encoded_len, 0);
    }

    let mut encoder = NormalizedOrderStatusRequestEncoder::default();
    encoder = encoder.wrap(WriteBuf::new(frame_buf.as_mut_slice()), SBE_HEADER_LENGTH);
    encoder = encoder
        .header(0)
        .parent()
        .map_err(|err| invalid_data(format!("failed to finalize SBE header: {err}")))?;
    encoder.side(request.side);
    encoder.cl_ord_id(request.cl_ord_id);
    encoder.order_id(request.order_id);
    encoder.symbol(request.symbol);
    encoder.sender_comp_id(request.sender_comp_id);
    encoder.target_comp_id(request.target_comp_id);

    let actual_len = encoder.get_limit();
    if actual_len != encoded_len {
        return Err(invalid_data(format!(
            "unexpected normalized order status request SBE length: expected {encoded_len}, got {actual_len}",
        )));
    }

    Ok(actual_len)
}

pub fn decode_normalized_order_status_request_from_sbe(
    frame: &[u8],
) -> io::Result<NormalizedOrderStatusRequest> {
    Ok(decode_normalized_order_status_request_view_from_sbe(frame)?.into())
}

pub fn decode_normalized_order_status_request_view_from_sbe(
    frame: &[u8],
) -> io::Result<NormalizedOrderStatusRequestView<'_>> {
    let header = validate_header(
        frame,
        velocitas_fix_sbe::normalized_order_status_request_codec::SBE_TEMPLATE_ID,
    )?;
    let mut decoder = NormalizedOrderStatusRequestDecoder::default().header(header, 0);
    let side = decoder.side();
    let cl_ord_id_coordinates = decoder.cl_ord_id_decoder();
    let order_id_coordinates = decoder.order_id_decoder();
    let symbol_coordinates = decoder.symbol_decoder();
    let sender_comp_id_coordinates = decoder.sender_comp_id_decoder();
    let target_comp_id_coordinates = decoder.target_comp_id_decoder();

    Ok(NormalizedOrderStatusRequestView {
        cl_ord_id: decode_var_data_str(frame, cl_ord_id_coordinates)?,
        order_id: decode_var_data_str(frame, order_id_coordinates)?,
        symbol: decode_var_data_str(frame, symbol_coordinates)?,
        side,
        sender_comp_id: decode_var_data_str(frame, sender_comp_id_coordinates)?,
        target_comp_id: decode_var_data_str(frame, target_comp_id_coordinates)?,
    })
}

pub fn encode_normalized_order_lifecycle_request_as_sbe(
    frame_buf: &mut Vec<u8>,
    request: &NormalizedOrderLifecycleRequest,
) -> io::Result<usize> {
    match request {
        NormalizedOrderLifecycleRequest::Cancel(request) => {
            encode_normalized_cancel_request_as_sbe(frame_buf, request)
        }
        NormalizedOrderLifecycleRequest::Replace(request) => {
            encode_normalized_replace_request_as_sbe(frame_buf, request)
        }
        NormalizedOrderLifecycleRequest::Status(request) => {
            encode_normalized_order_status_request_as_sbe(frame_buf, request)
        }
    }
}

pub fn encode_normalized_order_lifecycle_request_view_as_sbe(
    frame_buf: &mut Vec<u8>,
    request: &NormalizedOrderLifecycleRequestView<'_>,
) -> io::Result<usize> {
    match request {
        NormalizedOrderLifecycleRequestView::Cancel(request) => {
            encode_normalized_cancel_request_view_as_sbe(frame_buf, request)
        }
        NormalizedOrderLifecycleRequestView::Replace(request) => {
            encode_normalized_replace_request_view_as_sbe(frame_buf, request)
        }
        NormalizedOrderLifecycleRequestView::Status(request) => {
            encode_normalized_order_status_request_view_as_sbe(frame_buf, request)
        }
    }
}

pub fn decode_normalized_order_lifecycle_request_from_sbe(
    frame: &[u8],
) -> io::Result<NormalizedOrderLifecycleRequest> {
    Ok(decode_normalized_order_lifecycle_request_view_from_sbe(frame)?.into())
}

pub fn decode_normalized_order_lifecycle_request_view_from_sbe(
    frame: &[u8],
) -> io::Result<NormalizedOrderLifecycleRequestView<'_>> {
    match peek_sbe_template_id(frame)? {
        velocitas_fix_sbe::normalized_cancel_request_codec::SBE_TEMPLATE_ID => {
            Ok(NormalizedOrderLifecycleRequestView::Cancel(
                decode_normalized_cancel_request_view_from_sbe(frame)?,
            ))
        }
        velocitas_fix_sbe::normalized_replace_request_codec::SBE_TEMPLATE_ID => {
            Ok(NormalizedOrderLifecycleRequestView::Replace(
                decode_normalized_replace_request_view_from_sbe(frame)?,
            ))
        }
        velocitas_fix_sbe::normalized_order_status_request_codec::SBE_TEMPLATE_ID => {
            Ok(NormalizedOrderLifecycleRequestView::Status(
                decode_normalized_order_status_request_view_from_sbe(frame)?,
            ))
        }
        template_id => Err(invalid_data(format!(
            "unexpected lifecycle request template id: {template_id}",
        ))),
    }
}

pub fn encoded_normalized_execution_report_len(
    report: &NormalizedExecutionReport,
) -> io::Result<usize> {
    encoded_normalized_execution_report_len_parts(
        report.cl_ord_id.as_str(),
        report.symbol.as_str(),
        report.last_px.as_str(),
        report.avg_px.as_str(),
        report.order_id.as_str(),
        report.exec_id.as_str(),
    )
}

pub fn encode_normalized_execution_report_as_sbe(
    frame_buf: &mut Vec<u8>,
    report: &NormalizedExecutionReport,
) -> io::Result<usize> {
    let report_view: NormalizedExecutionReportView<'_> = report.into();
    encode_normalized_execution_report_view_as_sbe(frame_buf, &report_view)
}

pub fn encode_normalized_execution_report_view_as_sbe(
    frame_buf: &mut Vec<u8>,
    report: &NormalizedExecutionReportView<'_>,
) -> io::Result<usize> {
    let encoded_len = encoded_normalized_execution_report_len_parts(
        report.cl_ord_id,
        report.symbol,
        report.last_px,
        report.avg_px,
        report.order_id,
        report.exec_id,
    )?;
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
    encoder.cl_ord_id(report.cl_ord_id);
    encoder.symbol(report.symbol);
    encoder.last_px(report.last_px);
    encoder.avg_px(report.avg_px);
    encoder.order_id(report.order_id);
    encoder.exec_id(report.exec_id);

    let actual_len = encoder.get_limit();
    if actual_len != encoded_len {
        return Err(invalid_data(format!(
            "unexpected normalized execution report SBE length: expected {encoded_len}, got {actual_len}",
        )));
    }

    Ok(actual_len)
}

pub fn encode_filled_execution_report_from_order_view_as_sbe(
    frame_buf: &mut Vec<u8>,
    scratch: &mut NormalizedExecutionReportScratch,
    order: &NormalizedOrderView<'_>,
) -> io::Result<usize> {
    scratch.populate_from_cl_ord_id(order.cl_ord_id);
    let report = NormalizedExecutionReportView {
        cl_ord_id: order.cl_ord_id,
        symbol: order.symbol,
        side: order.side,
        order_qty: order.qty,
        last_qty: order.qty,
        last_px: order.price,
        leaves_qty: 0,
        cum_qty: order.qty,
        avg_px: order.price,
        order_id: scratch.order_id(),
        exec_id: scratch.exec_id(),
        exec_type: b'F',
        ord_status: b'2',
    };
    encode_normalized_execution_report_view_as_sbe(frame_buf, &report)
}

pub fn decode_normalized_execution_report_from_sbe(
    frame: &[u8],
) -> io::Result<NormalizedExecutionReport> {
    Ok(decode_normalized_execution_report_view_from_sbe(frame)?.into())
}

pub fn decode_normalized_execution_report_view_from_sbe(
    frame: &[u8],
) -> io::Result<NormalizedExecutionReportView<'_>> {
    let header = validate_header(
        frame,
        velocitas_fix_sbe::normalized_execution_report_codec::SBE_TEMPLATE_ID,
    )?;
    let mut decoder = NormalizedExecutionReportDecoder::default().header(header, 0);
    let side = decoder.side();
    let order_qty = decoder.order_qty();
    let last_qty = decoder.last_qty();
    let leaves_qty = decoder.leaves_qty();
    let cum_qty = decoder.cum_qty();
    let exec_type = decoder.exec_type();
    let ord_status = decoder.ord_status();
    let cl_ord_id_coordinates = decoder.cl_ord_id_decoder();
    let symbol_coordinates = decoder.symbol_decoder();
    let last_px_coordinates = decoder.last_px_decoder();
    let avg_px_coordinates = decoder.avg_px_decoder();
    let order_id_coordinates = decoder.order_id_decoder();
    let exec_id_coordinates = decoder.exec_id_decoder();

    let cl_ord_id = decode_var_data_str(frame, cl_ord_id_coordinates)?;
    let symbol = decode_var_data_str(frame, symbol_coordinates)?;
    let last_px = decode_var_data_str(frame, last_px_coordinates)?;
    let avg_px = decode_var_data_str(frame, avg_px_coordinates)?;
    let order_id = decode_var_data_str(frame, order_id_coordinates)?;
    let exec_id = decode_var_data_str(frame, exec_id_coordinates)?;

    Ok(NormalizedExecutionReportView {
        cl_ord_id,
        symbol,
        side,
        order_qty,
        last_qty,
        last_px,
        leaves_qty,
        cum_qty,
        avg_px,
        order_id,
        exec_id,
        exec_type,
        ord_status,
    })
}

pub fn encoded_normalized_cancel_reject_len(reject: &NormalizedCancelReject) -> io::Result<usize> {
    encoded_normalized_cancel_reject_len_parts(
        reject.order_id.as_str(),
        reject.cl_ord_id.as_str(),
        reject.orig_cl_ord_id.as_str(),
        reject.text.as_str(),
    )
}

pub fn encode_normalized_cancel_reject_as_sbe(
    frame_buf: &mut Vec<u8>,
    reject: &NormalizedCancelReject,
) -> io::Result<usize> {
    let reject_view: NormalizedCancelRejectView<'_> = reject.into();
    encode_normalized_cancel_reject_view_as_sbe(frame_buf, &reject_view)
}

pub fn encode_normalized_cancel_reject_view_as_sbe(
    frame_buf: &mut Vec<u8>,
    reject: &NormalizedCancelRejectView<'_>,
) -> io::Result<usize> {
    let encoded_len = encoded_normalized_cancel_reject_len_parts(
        reject.order_id,
        reject.cl_ord_id,
        reject.orig_cl_ord_id,
        reject.text,
    )?;
    if frame_buf.len() < encoded_len {
        frame_buf.resize(encoded_len, 0);
    }

    let mut encoder = NormalizedCancelRejectEncoder::default();
    encoder = encoder.wrap(WriteBuf::new(frame_buf.as_mut_slice()), SBE_HEADER_LENGTH);
    encoder = encoder
        .header(0)
        .parent()
        .map_err(|err| invalid_data(format!("failed to finalize SBE header: {err}")))?;
    encoder.ord_status(reject.ord_status);
    encoder.cxl_rej_response_to(reject.cxl_rej_response_to);
    encoder.cxl_rej_reason(reject.cxl_rej_reason);
    encoder.order_id(reject.order_id);
    encoder.cl_ord_id(reject.cl_ord_id);
    encoder.orig_cl_ord_id(reject.orig_cl_ord_id);
    encoder.text(reject.text);

    let actual_len = encoder.get_limit();
    if actual_len != encoded_len {
        return Err(invalid_data(format!(
            "unexpected normalized cancel reject SBE length: expected {encoded_len}, got {actual_len}",
        )));
    }

    Ok(actual_len)
}

pub fn decode_normalized_cancel_reject_from_sbe(
    frame: &[u8],
) -> io::Result<NormalizedCancelReject> {
    Ok(decode_normalized_cancel_reject_view_from_sbe(frame)?.into())
}

pub fn decode_normalized_cancel_reject_view_from_sbe(
    frame: &[u8],
) -> io::Result<NormalizedCancelRejectView<'_>> {
    let header = validate_header(
        frame,
        velocitas_fix_sbe::normalized_cancel_reject_codec::SBE_TEMPLATE_ID,
    )?;
    let mut decoder = NormalizedCancelRejectDecoder::default().header(header, 0);
    let ord_status = decoder.ord_status();
    let cxl_rej_response_to = decoder.cxl_rej_response_to();
    let cxl_rej_reason = decoder.cxl_rej_reason();
    let order_id_coordinates = decoder.order_id_decoder();
    let cl_ord_id_coordinates = decoder.cl_ord_id_decoder();
    let orig_cl_ord_id_coordinates = decoder.orig_cl_ord_id_decoder();
    let text_coordinates = decoder.text_decoder();

    Ok(NormalizedCancelRejectView {
        order_id: decode_var_data_str(frame, order_id_coordinates)?,
        cl_ord_id: decode_var_data_str(frame, cl_ord_id_coordinates)?,
        orig_cl_ord_id: decode_var_data_str(frame, orig_cl_ord_id_coordinates)?,
        ord_status,
        cxl_rej_response_to,
        cxl_rej_reason,
        text: decode_var_data_str(frame, text_coordinates)?,
    })
}

impl NormalizedOrder {
    pub fn from_fix(msg: &MessageView<'_>) -> io::Result<Self> {
        Ok(NormalizedOrderView::from_fix(msg).into())
    }
}

impl NormalizedCancelRequest {
    pub fn from_fix(msg: &MessageView<'_>) -> io::Result<Self> {
        Ok(NormalizedCancelRequestView::from_fix(msg).into())
    }
}

impl NormalizedReplaceRequest {
    pub fn from_fix(msg: &MessageView<'_>) -> io::Result<Self> {
        Ok(NormalizedReplaceRequestView::from_fix(msg).into())
    }
}

impl NormalizedOrderStatusRequest {
    pub fn from_fix(msg: &MessageView<'_>) -> io::Result<Self> {
        Ok(NormalizedOrderStatusRequestView::from_fix(msg).into())
    }
}

impl NormalizedOrderLifecycleRequest {
    pub fn from_fix(msg: &MessageView<'_>) -> io::Result<Self> {
        Ok(NormalizedOrderLifecycleRequestView::from_fix(msg)?.into())
    }

    pub fn action(&self) -> OrderLifecycleAction {
        match self {
            NormalizedOrderLifecycleRequest::Cancel(_) => OrderLifecycleAction::Cancel,
            NormalizedOrderLifecycleRequest::Replace(_) => OrderLifecycleAction::Replace,
            NormalizedOrderLifecycleRequest::Status(_) => OrderLifecycleAction::Status,
        }
    }
}

impl<'a> NormalizedOrderView<'a> {
    pub fn from_fix(msg: &MessageView<'a>) -> Self {
        Self {
            cl_ord_id: msg.get_field_str(tags::CL_ORD_ID).unwrap_or("?"),
            symbol: msg.get_field_str(tags::SYMBOL).unwrap_or("?"),
            side: msg
                .get_field_str(tags::SIDE)
                .and_then(|value| value.as_bytes().first().copied())
                .unwrap_or(b'?'),
            qty: msg.get_field_i64(tags::ORDER_QTY).unwrap_or(0),
            price: msg.get_field_str(tags::PRICE).unwrap_or("0"),
            sender_comp_id: msg.sender_comp_id().unwrap_or("?"),
            target_comp_id: msg.target_comp_id().unwrap_or("?"),
        }
    }
}

impl<'a> NormalizedCancelRequestView<'a> {
    pub fn from_fix(msg: &MessageView<'a>) -> Self {
        Self {
            cl_ord_id: msg.get_field_str(tags::CL_ORD_ID).unwrap_or("?"),
            orig_cl_ord_id: msg.get_field_str(tags::ORIG_CL_ORD_ID).unwrap_or("?"),
            symbol: msg.get_field_str(tags::SYMBOL).unwrap_or("?"),
            side: msg
                .get_field_str(tags::SIDE)
                .and_then(|value| value.as_bytes().first().copied())
                .unwrap_or(b'?'),
            qty: msg.get_field_i64(tags::ORDER_QTY).unwrap_or(0),
            sender_comp_id: msg.sender_comp_id().unwrap_or("?"),
            target_comp_id: msg.target_comp_id().unwrap_or("?"),
        }
    }
}

impl<'a> NormalizedReplaceRequestView<'a> {
    pub fn from_fix(msg: &MessageView<'a>) -> Self {
        Self {
            cl_ord_id: msg.get_field_str(tags::CL_ORD_ID).unwrap_or("?"),
            orig_cl_ord_id: msg.get_field_str(tags::ORIG_CL_ORD_ID).unwrap_or("?"),
            symbol: msg.get_field_str(tags::SYMBOL).unwrap_or("?"),
            side: msg
                .get_field_str(tags::SIDE)
                .and_then(|value| value.as_bytes().first().copied())
                .unwrap_or(b'?'),
            qty: msg.get_field_i64(tags::ORDER_QTY).unwrap_or(0),
            price: msg.get_field_str(tags::PRICE).unwrap_or("0"),
            sender_comp_id: msg.sender_comp_id().unwrap_or("?"),
            target_comp_id: msg.target_comp_id().unwrap_or("?"),
        }
    }
}

impl<'a> NormalizedOrderStatusRequestView<'a> {
    pub fn from_fix(msg: &MessageView<'a>) -> Self {
        Self {
            cl_ord_id: msg.get_field_str(tags::CL_ORD_ID).unwrap_or("?"),
            order_id: msg.get_field_str(tags::ORDER_ID).unwrap_or(""),
            symbol: msg.get_field_str(tags::SYMBOL).unwrap_or("?"),
            side: msg
                .get_field_str(tags::SIDE)
                .and_then(|value| value.as_bytes().first().copied())
                .unwrap_or(b'?'),
            sender_comp_id: msg.sender_comp_id().unwrap_or("?"),
            target_comp_id: msg.target_comp_id().unwrap_or("?"),
        }
    }
}

impl<'a> NormalizedOrderLifecycleRequestView<'a> {
    pub fn from_fix(msg: &MessageView<'a>) -> io::Result<Self> {
        match OrderLifecycleAction::from_fix_msg_type(msg.msg_type().unwrap_or_default())? {
            OrderLifecycleAction::Cancel => Ok(NormalizedOrderLifecycleRequestView::Cancel(
                NormalizedCancelRequestView::from_fix(msg),
            )),
            OrderLifecycleAction::Replace => Ok(NormalizedOrderLifecycleRequestView::Replace(
                NormalizedReplaceRequestView::from_fix(msg),
            )),
            OrderLifecycleAction::Status => Ok(NormalizedOrderLifecycleRequestView::Status(
                NormalizedOrderStatusRequestView::from_fix(msg),
            )),
        }
    }

    pub fn action(self) -> OrderLifecycleAction {
        match self {
            NormalizedOrderLifecycleRequestView::Cancel(_) => OrderLifecycleAction::Cancel,
            NormalizedOrderLifecycleRequestView::Replace(_) => OrderLifecycleAction::Replace,
            NormalizedOrderLifecycleRequestView::Status(_) => OrderLifecycleAction::Status,
        }
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

impl NormalizedExecutionReportScratch {
    pub fn populate_from_cl_ord_id(&mut self, cl_ord_id: &str) {
        self.order_id.clear();
        self.order_id.reserve("VENUE-".len() + cl_ord_id.len());
        self.order_id.push_str("VENUE-");
        self.order_id.push_str(cl_ord_id);

        self.exec_id.clear();
        self.exec_id.reserve("EXEC-".len() + cl_ord_id.len());
        self.exec_id.push_str("EXEC-");
        self.exec_id.push_str(cl_ord_id);
    }

    pub fn order_id(&self) -> &str {
        self.order_id.as_str()
    }

    pub fn exec_id(&self) -> &str {
        self.exec_id.as_str()
    }
}

impl<'a> NormalizedCancelRejectView<'a> {
    pub fn response_action(self) -> io::Result<OrderLifecycleAction> {
        OrderLifecycleAction::from_cancel_reject_response_to(self.cxl_rej_response_to)
    }
}

impl<'a> From<&'a NormalizedOrder> for NormalizedOrderView<'a> {
    fn from(order: &'a NormalizedOrder) -> Self {
        Self {
            cl_ord_id: order.cl_ord_id.as_str(),
            symbol: order.symbol.as_str(),
            side: order.side,
            qty: order.qty,
            price: order.price.as_str(),
            sender_comp_id: order.sender_comp_id.as_str(),
            target_comp_id: order.target_comp_id.as_str(),
        }
    }
}

impl<'a> From<NormalizedOrderView<'a>> for NormalizedOrder {
    fn from(order: NormalizedOrderView<'a>) -> Self {
        Self {
            cl_ord_id: order.cl_ord_id.to_string(),
            symbol: order.symbol.to_string(),
            side: order.side,
            qty: order.qty,
            price: order.price.to_string(),
            sender_comp_id: order.sender_comp_id.to_string(),
            target_comp_id: order.target_comp_id.to_string(),
        }
    }
}

impl<'a> From<&'a NormalizedCancelRequest> for NormalizedCancelRequestView<'a> {
    fn from(request: &'a NormalizedCancelRequest) -> Self {
        Self {
            cl_ord_id: request.cl_ord_id.as_str(),
            orig_cl_ord_id: request.orig_cl_ord_id.as_str(),
            symbol: request.symbol.as_str(),
            side: request.side,
            qty: request.qty,
            sender_comp_id: request.sender_comp_id.as_str(),
            target_comp_id: request.target_comp_id.as_str(),
        }
    }
}

impl<'a> From<NormalizedCancelRequestView<'a>> for NormalizedCancelRequest {
    fn from(request: NormalizedCancelRequestView<'a>) -> Self {
        Self {
            cl_ord_id: request.cl_ord_id.to_string(),
            orig_cl_ord_id: request.orig_cl_ord_id.to_string(),
            symbol: request.symbol.to_string(),
            side: request.side,
            qty: request.qty,
            sender_comp_id: request.sender_comp_id.to_string(),
            target_comp_id: request.target_comp_id.to_string(),
        }
    }
}

impl<'a> From<&'a NormalizedReplaceRequest> for NormalizedReplaceRequestView<'a> {
    fn from(request: &'a NormalizedReplaceRequest) -> Self {
        Self {
            cl_ord_id: request.cl_ord_id.as_str(),
            orig_cl_ord_id: request.orig_cl_ord_id.as_str(),
            symbol: request.symbol.as_str(),
            side: request.side,
            qty: request.qty,
            price: request.price.as_str(),
            sender_comp_id: request.sender_comp_id.as_str(),
            target_comp_id: request.target_comp_id.as_str(),
        }
    }
}

impl<'a> From<NormalizedReplaceRequestView<'a>> for NormalizedReplaceRequest {
    fn from(request: NormalizedReplaceRequestView<'a>) -> Self {
        Self {
            cl_ord_id: request.cl_ord_id.to_string(),
            orig_cl_ord_id: request.orig_cl_ord_id.to_string(),
            symbol: request.symbol.to_string(),
            side: request.side,
            qty: request.qty,
            price: request.price.to_string(),
            sender_comp_id: request.sender_comp_id.to_string(),
            target_comp_id: request.target_comp_id.to_string(),
        }
    }
}

impl<'a> From<&'a NormalizedOrderStatusRequest> for NormalizedOrderStatusRequestView<'a> {
    fn from(request: &'a NormalizedOrderStatusRequest) -> Self {
        Self {
            cl_ord_id: request.cl_ord_id.as_str(),
            order_id: request.order_id.as_str(),
            symbol: request.symbol.as_str(),
            side: request.side,
            sender_comp_id: request.sender_comp_id.as_str(),
            target_comp_id: request.target_comp_id.as_str(),
        }
    }
}

impl<'a> From<NormalizedOrderStatusRequestView<'a>> for NormalizedOrderStatusRequest {
    fn from(request: NormalizedOrderStatusRequestView<'a>) -> Self {
        Self {
            cl_ord_id: request.cl_ord_id.to_string(),
            order_id: request.order_id.to_string(),
            symbol: request.symbol.to_string(),
            side: request.side,
            sender_comp_id: request.sender_comp_id.to_string(),
            target_comp_id: request.target_comp_id.to_string(),
        }
    }
}

impl From<NormalizedOrderLifecycleRequestView<'_>> for NormalizedOrderLifecycleRequest {
    fn from(request: NormalizedOrderLifecycleRequestView<'_>) -> Self {
        match request {
            NormalizedOrderLifecycleRequestView::Cancel(request) => {
                NormalizedOrderLifecycleRequest::Cancel(request.into())
            }
            NormalizedOrderLifecycleRequestView::Replace(request) => {
                NormalizedOrderLifecycleRequest::Replace(request.into())
            }
            NormalizedOrderLifecycleRequestView::Status(request) => {
                NormalizedOrderLifecycleRequest::Status(request.into())
            }
        }
    }
}

impl<'a> From<&'a NormalizedOrderLifecycleRequest> for NormalizedOrderLifecycleRequestView<'a> {
    fn from(request: &'a NormalizedOrderLifecycleRequest) -> Self {
        match request {
            NormalizedOrderLifecycleRequest::Cancel(request) => {
                NormalizedOrderLifecycleRequestView::Cancel(request.into())
            }
            NormalizedOrderLifecycleRequest::Replace(request) => {
                NormalizedOrderLifecycleRequestView::Replace(request.into())
            }
            NormalizedOrderLifecycleRequest::Status(request) => {
                NormalizedOrderLifecycleRequestView::Status(request.into())
            }
        }
    }
}

impl<'a> From<&'a NormalizedExecutionReport> for NormalizedExecutionReportView<'a> {
    fn from(report: &'a NormalizedExecutionReport) -> Self {
        Self {
            cl_ord_id: report.cl_ord_id.as_str(),
            symbol: report.symbol.as_str(),
            side: report.side,
            order_qty: report.order_qty,
            last_qty: report.last_qty,
            last_px: report.last_px.as_str(),
            leaves_qty: report.leaves_qty,
            cum_qty: report.cum_qty,
            avg_px: report.avg_px.as_str(),
            order_id: report.order_id.as_str(),
            exec_id: report.exec_id.as_str(),
            exec_type: report.exec_type,
            ord_status: report.ord_status,
        }
    }
}

impl<'a> From<NormalizedExecutionReportView<'a>> for NormalizedExecutionReport {
    fn from(report: NormalizedExecutionReportView<'a>) -> Self {
        Self {
            cl_ord_id: report.cl_ord_id.to_string(),
            symbol: report.symbol.to_string(),
            side: report.side,
            order_qty: report.order_qty,
            last_qty: report.last_qty,
            last_px: report.last_px.to_string(),
            leaves_qty: report.leaves_qty,
            cum_qty: report.cum_qty,
            avg_px: report.avg_px.to_string(),
            order_id: report.order_id.to_string(),
            exec_id: report.exec_id.to_string(),
            exec_type: report.exec_type,
            ord_status: report.ord_status,
        }
    }
}

impl<'a> From<&'a NormalizedCancelReject> for NormalizedCancelRejectView<'a> {
    fn from(reject: &'a NormalizedCancelReject) -> Self {
        Self {
            order_id: reject.order_id.as_str(),
            cl_ord_id: reject.cl_ord_id.as_str(),
            orig_cl_ord_id: reject.orig_cl_ord_id.as_str(),
            ord_status: reject.ord_status,
            cxl_rej_response_to: reject.cxl_rej_response_to,
            cxl_rej_reason: reject.cxl_rej_reason,
            text: reject.text.as_str(),
        }
    }
}

impl<'a> From<NormalizedCancelRejectView<'a>> for NormalizedCancelReject {
    fn from(reject: NormalizedCancelRejectView<'a>) -> Self {
        Self {
            order_id: reject.order_id.to_string(),
            cl_ord_id: reject.cl_ord_id.to_string(),
            orig_cl_ord_id: reject.orig_cl_ord_id.to_string(),
            ord_status: reject.ord_status,
            cxl_rej_response_to: reject.cxl_rej_response_to,
            cxl_rej_reason: reject.cxl_rej_reason,
            text: reject.text.to_string(),
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

    #[test]
    fn normalized_order_view_sbe_round_trip() {
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
        let view: NormalizedOrderView<'_> = (&order).into();
        let len = encode_normalized_order_view_as_sbe(&mut buffer, &view).unwrap();
        let decoded = decode_normalized_order_view_from_sbe(&buffer[..len]).unwrap();

        assert_eq!(decoded.cl_ord_id, order.cl_ord_id);
        assert_eq!(decoded.symbol, order.symbol);
        assert_eq!(decoded.side, order.side);
        assert_eq!(decoded.qty, order.qty);
        assert_eq!(decoded.price, order.price);
        assert_eq!(decoded.sender_comp_id, order.sender_comp_id);
        assert_eq!(decoded.target_comp_id, order.target_comp_id);
    }

    #[test]
    fn filled_execution_report_from_order_view_round_trip() {
        let order = NormalizedOrder {
            cl_ord_id: "BPIPE-ORD-0001".into(),
            symbol: "EUR/USD".into(),
            side: b'1',
            qty: 5_000_000,
            price: "1.08255".into(),
            sender_comp_id: "BLOOMBERG_FX".into(),
            target_comp_id: "VELOCITAS_GATEWAY".into(),
        };

        let mut scratch = NormalizedExecutionReportScratch::default();
        let mut buffer = Vec::new();
        let order_view: NormalizedOrderView<'_> = (&order).into();
        let len = encode_filled_execution_report_from_order_view_as_sbe(
            &mut buffer,
            &mut scratch,
            &order_view,
        )
        .unwrap();
        let decoded = decode_normalized_execution_report_from_sbe(&buffer[..len]).unwrap();

        assert_eq!(decoded.cl_ord_id, order.cl_ord_id);
        assert_eq!(decoded.symbol, order.symbol);
        assert_eq!(decoded.side, order.side);
        assert_eq!(decoded.order_qty, order.qty);
        assert_eq!(decoded.last_qty, order.qty);
        assert_eq!(decoded.last_px, order.price);
        assert_eq!(decoded.avg_px, order.price);
        assert_eq!(decoded.order_id, format!("VENUE-{}", order.cl_ord_id));
        assert_eq!(decoded.exec_id, format!("EXEC-{}", order.cl_ord_id));
        assert_eq!(decoded.exec_type, b'F');
        assert_eq!(decoded.ord_status, b'2');
    }

    #[test]
    fn lifecycle_request_sbe_round_trips_and_template_ids() {
        let cancel_request = NormalizedCancelRequest {
            cl_ord_id: "BPIPE-CXL-0001".into(),
            orig_cl_ord_id: "BPIPE-ORD-0002".into(),
            symbol: "EUR/USD".into(),
            side: b'1',
            qty: 6_000_000,
            sender_comp_id: "BLOOMBERG_FX".into(),
            target_comp_id: "VELOCITAS_GATEWAY".into(),
        };
        let replace_request = NormalizedReplaceRequest {
            cl_ord_id: "BPIPE-ORD-0002".into(),
            orig_cl_ord_id: "BPIPE-ORD-0001".into(),
            symbol: "EUR/USD".into(),
            side: b'1',
            qty: 6_000_000,
            price: "1.08260".into(),
            sender_comp_id: "BLOOMBERG_FX".into(),
            target_comp_id: "VELOCITAS_GATEWAY".into(),
        };
        let status_request = NormalizedOrderStatusRequest {
            cl_ord_id: "BPIPE-ORD-0002".into(),
            order_id: "VENUE-BPIPE-ORD-0001".into(),
            symbol: "EUR/USD".into(),
            side: b'1',
            sender_comp_id: "BLOOMBERG_FX".into(),
            target_comp_id: "VELOCITAS_GATEWAY".into(),
        };

        let mut buffer = Vec::new();
        let len = encode_normalized_cancel_request_as_sbe(&mut buffer, &cancel_request).unwrap();
        assert_eq!(
            peek_sbe_template_id(&buffer[..len]).unwrap(),
            velocitas_fix_sbe::normalized_cancel_request_codec::SBE_TEMPLATE_ID
        );
        assert_eq!(
            decode_normalized_cancel_request_from_sbe(&buffer[..len]).unwrap(),
            cancel_request
        );

        let len = encode_normalized_replace_request_as_sbe(&mut buffer, &replace_request).unwrap();
        assert_eq!(
            peek_sbe_template_id(&buffer[..len]).unwrap(),
            velocitas_fix_sbe::normalized_replace_request_codec::SBE_TEMPLATE_ID
        );
        assert_eq!(
            decode_normalized_replace_request_from_sbe(&buffer[..len]).unwrap(),
            replace_request
        );

        let len =
            encode_normalized_order_status_request_as_sbe(&mut buffer, &status_request).unwrap();
        assert_eq!(
            peek_sbe_template_id(&buffer[..len]).unwrap(),
            velocitas_fix_sbe::normalized_order_status_request_codec::SBE_TEMPLATE_ID
        );
        assert_eq!(
            decode_normalized_order_status_request_from_sbe(&buffer[..len]).unwrap(),
            status_request
        );
    }

    #[test]
    fn normalized_cancel_reject_sbe_round_trip() {
        let reject = NormalizedCancelReject {
            order_id: "VENUE-BPIPE-ORD-0001".into(),
            cl_ord_id: "BPIPE-CXL-0002".into(),
            orig_cl_ord_id: "BPIPE-ORD-0002".into(),
            ord_status: b'4',
            cxl_rej_response_to: b'1',
            cxl_rej_reason: 1,
            text: "order already canceled".into(),
        };

        let mut buffer = Vec::new();
        let len = encode_normalized_cancel_reject_as_sbe(&mut buffer, &reject).unwrap();
        assert_eq!(
            peek_sbe_template_id(&buffer[..len]).unwrap(),
            velocitas_fix_sbe::normalized_cancel_reject_codec::SBE_TEMPLATE_ID
        );

        let decoded = decode_normalized_cancel_reject_from_sbe(&buffer[..len]).unwrap();
        assert_eq!(decoded, reject);
    }

    #[test]
    fn canonical_lifecycle_request_dispatch_round_trip() {
        let requests = [
            NormalizedOrderLifecycleRequest::Cancel(NormalizedCancelRequest {
                cl_ord_id: "BPIPE-CXL-0001".into(),
                orig_cl_ord_id: "BPIPE-ORD-0002".into(),
                symbol: "EUR/USD".into(),
                side: b'1',
                qty: 6_000_000,
                sender_comp_id: "BLOOMBERG_FX".into(),
                target_comp_id: "VELOCITAS_GATEWAY".into(),
            }),
            NormalizedOrderLifecycleRequest::Replace(NormalizedReplaceRequest {
                cl_ord_id: "BPIPE-ORD-0002".into(),
                orig_cl_ord_id: "BPIPE-ORD-0001".into(),
                symbol: "EUR/USD".into(),
                side: b'1',
                qty: 6_000_000,
                price: "1.08260".into(),
                sender_comp_id: "BLOOMBERG_FX".into(),
                target_comp_id: "VELOCITAS_GATEWAY".into(),
            }),
            NormalizedOrderLifecycleRequest::Status(NormalizedOrderStatusRequest {
                cl_ord_id: "BPIPE-ORD-0002".into(),
                order_id: "VENUE-BPIPE-ORD-0001".into(),
                symbol: "EUR/USD".into(),
                side: b'1',
                sender_comp_id: "BLOOMBERG_FX".into(),
                target_comp_id: "VELOCITAS_GATEWAY".into(),
            }),
        ];

        let mut buffer = Vec::new();
        for request in requests {
            let len =
                encode_normalized_order_lifecycle_request_as_sbe(&mut buffer, &request).unwrap();
            let decoded =
                decode_normalized_order_lifecycle_request_from_sbe(&buffer[..len]).unwrap();
            assert_eq!(decoded, request);
        }
    }
}
