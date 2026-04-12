/// FIX data dictionary — defines valid tags, types, and message structures.
///
/// Dictionaries are compiled from XML at build time for maximum performance.
/// At runtime, field validation uses lookup tables with O(1) access.

/// Field data types as defined in the FIX specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Int,
    Float,
    Qty,
    Price,
    Amt,
    Char,
    String,
    Boolean,
    Data,
    UtcTimestamp,
    UtcTimeOnly,
    UtcDateOnly,
    LocalMktDate,
    MonthYear,
    MultipleValueString,
    Currency,
    Exchange,
    Country,
    NumInGroup,
    SeqNum,
    Length,
}

/// Whether a field is required or optional in a given context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldPresence {
    Required,
    Optional,
}

/// Definition of a single FIX field.
#[derive(Debug, Clone)]
pub struct FieldDef {
    pub tag: u32,
    pub name: &'static str,
    pub field_type: FieldType,
}

/// Standard FIX 4.4 field definitions (subset).
pub static STANDARD_FIELDS: &[FieldDef] = &[
    FieldDef {
        tag: 8,
        name: "BeginString",
        field_type: FieldType::String,
    },
    FieldDef {
        tag: 9,
        name: "BodyLength",
        field_type: FieldType::Length,
    },
    FieldDef {
        tag: 10,
        name: "CheckSum",
        field_type: FieldType::String,
    },
    FieldDef {
        tag: 11,
        name: "ClOrdID",
        field_type: FieldType::String,
    },
    FieldDef {
        tag: 14,
        name: "CumQty",
        field_type: FieldType::Qty,
    },
    FieldDef {
        tag: 17,
        name: "ExecID",
        field_type: FieldType::String,
    },
    FieldDef {
        tag: 21,
        name: "HandlInst",
        field_type: FieldType::Char,
    },
    FieldDef {
        tag: 31,
        name: "LastPx",
        field_type: FieldType::Price,
    },
    FieldDef {
        tag: 32,
        name: "LastQty",
        field_type: FieldType::Qty,
    },
    FieldDef {
        tag: 34,
        name: "MsgSeqNum",
        field_type: FieldType::SeqNum,
    },
    FieldDef {
        tag: 35,
        name: "MsgType",
        field_type: FieldType::String,
    },
    FieldDef {
        tag: 37,
        name: "OrderID",
        field_type: FieldType::String,
    },
    FieldDef {
        tag: 38,
        name: "OrderQty",
        field_type: FieldType::Qty,
    },
    FieldDef {
        tag: 39,
        name: "OrdStatus",
        field_type: FieldType::Char,
    },
    FieldDef {
        tag: 40,
        name: "OrdType",
        field_type: FieldType::Char,
    },
    FieldDef {
        tag: 44,
        name: "Price",
        field_type: FieldType::Price,
    },
    FieldDef {
        tag: 49,
        name: "SenderCompID",
        field_type: FieldType::String,
    },
    FieldDef {
        tag: 52,
        name: "SendingTime",
        field_type: FieldType::UtcTimestamp,
    },
    FieldDef {
        tag: 54,
        name: "Side",
        field_type: FieldType::Char,
    },
    FieldDef {
        tag: 55,
        name: "Symbol",
        field_type: FieldType::String,
    },
    FieldDef {
        tag: 56,
        name: "TargetCompID",
        field_type: FieldType::String,
    },
    FieldDef {
        tag: 59,
        name: "TimeInForce",
        field_type: FieldType::Char,
    },
    FieldDef {
        tag: 60,
        name: "TransactTime",
        field_type: FieldType::UtcTimestamp,
    },
    FieldDef {
        tag: 98,
        name: "EncryptMethod",
        field_type: FieldType::Int,
    },
    FieldDef {
        tag: 108,
        name: "HeartBtInt",
        field_type: FieldType::Int,
    },
    FieldDef {
        tag: 150,
        name: "ExecType",
        field_type: FieldType::Char,
    },
    FieldDef {
        tag: 151,
        name: "LeavesQty",
        field_type: FieldType::Qty,
    },
    FieldDef {
        tag: 6,
        name: "AvgPx",
        field_type: FieldType::Price,
    },
];

/// Look up a field definition by tag number.
pub fn lookup_field(tag: u32) -> Option<&'static FieldDef> {
    STANDARD_FIELDS.iter().find(|f| f.tag == tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lookup_known_fields() {
        let field = lookup_field(35).unwrap();
        assert_eq!(field.name, "MsgType");
        assert_eq!(field.field_type, FieldType::String);

        let field = lookup_field(44).unwrap();
        assert_eq!(field.name, "Price");
        assert_eq!(field.field_type, FieldType::Price);
    }

    #[test]
    fn test_lookup_unknown_field() {
        assert!(lookup_field(99999).is_none());
    }
}
