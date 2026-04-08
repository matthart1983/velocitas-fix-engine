/// FIX tag constants — compile-time known for branch prediction and inlining.

// Session-level tags
pub const BEGIN_STRING: u32 = 8;
pub const BODY_LENGTH: u32 = 9;
pub const MSG_TYPE: u32 = 35;
pub const SENDER_COMP_ID: u32 = 49;
pub const TARGET_COMP_ID: u32 = 56;
pub const MSG_SEQ_NUM: u32 = 34;
pub const SENDING_TIME: u32 = 52;
pub const CHECKSUM: u32 = 10;
pub const POSS_DUP_FLAG: u32 = 43;
pub const POSS_RESEND: u32 = 97;
pub const ORIG_SENDING_TIME: u32 = 122;
pub const ENCRYPT_METHOD: u32 = 98;
pub const HEARTBT_INT: u32 = 108;
pub const RESET_SEQ_NUM_FLAG: u32 = 141;
pub const TEST_REQ_ID: u32 = 112;
pub const BEGIN_SEQ_NO: u32 = 7;
pub const END_SEQ_NO: u32 = 16;
pub const GAP_FILL_FLAG: u32 = 123;
pub const NEW_SEQ_NO: u32 = 36;
pub const REF_SEQ_NUM: u32 = 45;
pub const REF_MSG_TYPE: u32 = 372;
pub const SESSION_REJECT_REASON: u32 = 373;
pub const TEXT: u32 = 58;
pub const USERNAME: u32 = 553;
pub const PASSWORD: u32 = 554;

// Order tags
pub const CL_ORD_ID: u32 = 11;
pub const ORIG_CL_ORD_ID: u32 = 41;
pub const ORDER_ID: u32 = 37;
pub const EXEC_ID: u32 = 17;
pub const EXEC_TYPE: u32 = 150;
pub const ORD_STATUS: u32 = 39;
pub const SYMBOL: u32 = 55;
pub const SIDE: u32 = 54;
pub const ORDER_QTY: u32 = 38;
pub const ORD_TYPE: u32 = 40;
pub const PRICE: u32 = 44;
pub const TIME_IN_FORCE: u32 = 59;
pub const TRANSACT_TIME: u32 = 60;
pub const LAST_QTY: u32 = 32;
pub const LAST_PX: u32 = 31;
pub const LEAVES_QTY: u32 = 151;
pub const CUM_QTY: u32 = 14;
pub const AVG_PX: u32 = 6;
pub const CXL_REJ_REASON: u32 = 102;
pub const CXL_REJ_RESPONSE_TO: u32 = 434;
pub const ACCOUNT: u32 = 1;
pub const HANDL_INST: u32 = 21;
pub const SECURITY_EXCHANGE: u32 = 207;

// Market data tags
pub const MD_REQ_ID: u32 = 262;
pub const SUBSCRIPTION_REQUEST_TYPE: u32 = 263;
pub const MARKET_DEPTH: u32 = 264;
pub const NO_MD_ENTRY_TYPES: u32 = 267;
pub const MD_ENTRY_TYPE: u32 = 269;
pub const NO_MD_ENTRIES: u32 = 268;
pub const MD_ENTRY_PX: u32 = 270;
pub const MD_ENTRY_SIZE: u32 = 271;
pub const MD_UPDATE_ACTION: u32 = 279;

// Constants
pub const SOH: u8 = 0x01;
pub const EQUALS: u8 = b'=';
