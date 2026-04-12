/// XML FIX dictionary compiler — parses QuickFIX-style XML dictionaries
/// into an optimized runtime format with O(1) field lookup by tag.

/// Field definition compiled from XML.
#[derive(Debug, Clone)]
pub struct CompiledFieldDef {
    pub tag: u32,
    pub name: String,
    pub field_type: String,
    pub values: Vec<(String, String)>,
}

/// Message category as defined in QuickFIX XML dictionaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageCategory {
    Admin,
    Application,
}

/// Message definition compiled from XML.
#[derive(Debug, Clone)]
pub struct CompiledMessageDef {
    pub msg_type: String,
    pub name: String,
    pub category: MessageCategory,
    pub required_fields: Vec<u32>,
    pub optional_fields: Vec<u32>,
}

/// Errors that can occur during dictionary compilation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DictError {
    ParseError(String),
    MissingAttribute(String),
    InvalidFormat(String),
}

impl std::fmt::Display for DictError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DictError::ParseError(s) => write!(f, "parse error: {}", s),
            DictError::MissingAttribute(s) => write!(f, "missing attribute: {}", s),
            DictError::InvalidFormat(s) => write!(f, "invalid format: {}", s),
        }
    }
}

/// Compiled FIX dictionary with O(1) field lookup by tag number.
#[derive(Debug, Clone)]
pub struct CompiledDictionary {
    pub fix_version: String,
    pub fields: Vec<CompiledFieldDef>,
    pub messages: Vec<CompiledMessageDef>,
    tag_index: Vec<Option<usize>>,
}

/// Extract an attribute value from an XML tag string.
/// e.g. `extract_attr("<field number=\"8\" name=\"Foo\"/>", "name")` → `Some("Foo")`
fn extract_attr(tag_str: &str, attr_name: &str) -> Option<String> {
    let pattern = format!("{}=\"", attr_name);
    let start = tag_str.find(&pattern)?;
    let value_start = start + pattern.len();
    let rest = &tag_str[value_start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

impl CompiledDictionary {
    /// Parse a QuickFIX-style XML dictionary string into a compiled dictionary.
    pub fn from_xml(xml: &str) -> Result<Self, DictError> {
        let fix_version = Self::parse_version(xml)?;
        let fields = Self::parse_fields(xml)?;
        let name_to_tag: std::collections::HashMap<&str, u32> =
            fields.iter().map(|f| (f.name.as_str(), f.tag)).collect();
        let messages = Self::parse_messages(xml, &name_to_tag)?;
        let tag_index = Self::build_tag_index(&fields);

        Ok(CompiledDictionary {
            fix_version,
            fields,
            messages,
            tag_index,
        })
    }

    /// O(1) field lookup by tag number.
    pub fn lookup_field(&self, tag: u32) -> Option<&CompiledFieldDef> {
        let idx = tag as usize;
        if idx < self.tag_index.len() {
            self.tag_index[idx].map(|i| &self.fields[i])
        } else {
            None
        }
    }

    /// Look up a message definition by its MsgType string.
    pub fn lookup_message(&self, msg_type: &str) -> Option<&CompiledMessageDef> {
        self.messages.iter().find(|m| m.msg_type == msg_type)
    }

    /// Validate that all required fields for a message type are present.
    /// Returns a list of error strings for each missing required field.
    pub fn validate_message(&self, msg_type: &str, present_tags: &[u32]) -> Vec<String> {
        let msg = match self.lookup_message(msg_type) {
            Some(m) => m,
            None => return vec![format!("unknown message type: {}", msg_type)],
        };
        msg.required_fields
            .iter()
            .filter(|tag| !present_tags.contains(tag))
            .map(|tag| {
                let name = self
                    .lookup_field(*tag)
                    .map(|f| f.name.as_str())
                    .unwrap_or("Unknown");
                format!("missing required field: {} (tag {})", name, tag)
            })
            .collect()
    }

    /// Number of field definitions in this dictionary.
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Number of message definitions in this dictionary.
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    fn parse_version(xml: &str) -> Result<String, DictError> {
        let fix_start = xml
            .find("<fix ")
            .ok_or_else(|| DictError::ParseError("no <fix> tag found".into()))?;
        let fix_end = xml[fix_start..]
            .find('>')
            .ok_or_else(|| DictError::ParseError("unclosed <fix> tag".into()))?;
        let fix_tag = &xml[fix_start..fix_start + fix_end + 1];

        let major = extract_attr(fix_tag, "major")
            .ok_or_else(|| DictError::MissingAttribute("major".into()))?;
        let minor = extract_attr(fix_tag, "minor")
            .ok_or_else(|| DictError::MissingAttribute("minor".into()))?;

        Ok(format!("FIX.{}.{}", major, minor))
    }

    fn parse_fields(xml: &str) -> Result<Vec<CompiledFieldDef>, DictError> {
        let fields_start = match xml.find("<fields>") {
            Some(pos) => pos,
            None => return Ok(Vec::new()),
        };
        let fields_end = xml[fields_start..]
            .find("</fields>")
            .ok_or_else(|| DictError::ParseError("unclosed <fields> section".into()))?;
        let fields_section = &xml[fields_start..fields_start + fields_end];

        let mut fields = Vec::new();
        let mut pos = 0;

        while pos < fields_section.len() {
            let tag_start = match fields_section[pos..].find("<field ") {
                Some(s) => pos + s,
                None => break,
            };

            // Determine if this is a self-closing field or has children (enum values)
            let after_tag = &fields_section[tag_start..];
            let self_close = after_tag.find("/>");
            let open_close = after_tag.find('>');

            let (tag_str, values, next_pos) = match (self_close, open_close) {
                (Some(sc), Some(oc)) if sc < oc => {
                    // Self-closing: <field ... />
                    let tag_str = &fields_section[tag_start..tag_start + sc + 2];
                    (tag_str, Vec::new(), tag_start + sc + 2)
                }
                (_, Some(oc)) => {
                    // Has children — scan for </field>
                    let tag_str = &fields_section[tag_start..tag_start + oc + 1];
                    let close_tag = match fields_section[tag_start..].find("</field>") {
                        Some(ct) => tag_start + ct,
                        None => {
                            pos = tag_start + oc + 1;
                            continue;
                        }
                    };
                    let body = &fields_section[tag_start + oc + 1..close_tag];
                    let values = Self::parse_enum_values(body);
                    (tag_str, values, close_tag + "</field>".len())
                }
                _ => break,
            };

            let number_str = extract_attr(tag_str, "number")
                .ok_or_else(|| DictError::MissingAttribute("field number".into()))?;
            let tag_num: u32 = number_str.parse().map_err(|_| {
                DictError::InvalidFormat(format!("invalid field number: {}", number_str))
            })?;
            let name = extract_attr(tag_str, "name")
                .ok_or_else(|| DictError::MissingAttribute("field name".into()))?;
            let field_type = extract_attr(tag_str, "type")
                .ok_or_else(|| DictError::MissingAttribute("field type".into()))?;

            fields.push(CompiledFieldDef {
                tag: tag_num,
                name,
                field_type,
                values,
            });

            pos = next_pos;
        }

        Ok(fields)
    }

    fn parse_enum_values(body: &str) -> Vec<(String, String)> {
        let mut values = Vec::new();
        let mut pos = 0;

        while pos < body.len() {
            let tag_start = match body[pos..].find("<value ") {
                Some(s) => pos + s,
                None => break,
            };
            let tag_end = match body[tag_start..].find("/>") {
                Some(e) => tag_start + e + 2,
                None => break,
            };
            let tag_str = &body[tag_start..tag_end];

            if let (Some(enum_val), Some(desc)) = (
                extract_attr(tag_str, "enum"),
                extract_attr(tag_str, "description"),
            ) {
                values.push((enum_val, desc));
            }

            pos = tag_end;
        }

        values
    }

    fn parse_messages(
        xml: &str,
        name_to_tag: &std::collections::HashMap<&str, u32>,
    ) -> Result<Vec<CompiledMessageDef>, DictError> {
        let msgs_start = match xml.find("<messages>") {
            Some(pos) => pos,
            None => return Ok(Vec::new()),
        };
        let msgs_end = xml[msgs_start..]
            .find("</messages>")
            .ok_or_else(|| DictError::ParseError("unclosed <messages> section".into()))?;
        let msgs_section = &xml[msgs_start..msgs_start + msgs_end];

        let mut messages = Vec::new();
        let mut pos = 0;

        while pos < msgs_section.len() {
            let msg_start = match msgs_section[pos..].find("<message ") {
                Some(s) => pos + s,
                None => break,
            };
            let header_end = match msgs_section[msg_start..].find('>') {
                Some(e) => msg_start + e,
                None => break,
            };
            let msg_tag = &msgs_section[msg_start..header_end + 1];

            let name = extract_attr(msg_tag, "name")
                .ok_or_else(|| DictError::MissingAttribute("message name".into()))?;
            let msg_type = extract_attr(msg_tag, "msgtype")
                .ok_or_else(|| DictError::MissingAttribute("message msgtype".into()))?;
            let msgcat = extract_attr(msg_tag, "msgcat")
                .ok_or_else(|| DictError::MissingAttribute("message msgcat".into()))?;

            let category = match msgcat.as_str() {
                "admin" => MessageCategory::Admin,
                _ => MessageCategory::Application,
            };

            let msg_close = match msgs_section[msg_start..].find("</message>") {
                Some(c) => msg_start + c,
                None => break,
            };
            let msg_body = &msgs_section[header_end + 1..msg_close];

            let mut required_fields = Vec::new();
            let mut optional_fields = Vec::new();
            let mut fpos = 0;

            while fpos < msg_body.len() {
                let field_start = match msg_body[fpos..].find("<field ") {
                    Some(s) => fpos + s,
                    None => break,
                };
                let field_end = match msg_body[field_start..].find("/>") {
                    Some(e) => field_start + e + 2,
                    None => break,
                };
                let field_tag = &msg_body[field_start..field_end];

                if let Some(field_name) = extract_attr(field_tag, "name") {
                    if let Some(&tag) = name_to_tag.get(field_name.as_str()) {
                        let required = extract_attr(field_tag, "required")
                            .map(|v| v == "Y")
                            .unwrap_or(false);
                        if required {
                            required_fields.push(tag);
                        } else {
                            optional_fields.push(tag);
                        }
                    }
                }

                fpos = field_end;
            }

            messages.push(CompiledMessageDef {
                msg_type,
                name,
                category,
                required_fields,
                optional_fields,
            });

            pos = msg_close + "</message>".len();
        }

        Ok(messages)
    }

    fn build_tag_index(fields: &[CompiledFieldDef]) -> Vec<Option<usize>> {
        let max_tag = fields.iter().map(|f| f.tag).max().unwrap_or(0) as usize;
        let mut index = vec![None; max_tag + 1];
        for (i, field) in fields.iter().enumerate() {
            index[field.tag as usize] = Some(i);
        }
        index
    }
}

pub const TEST_DICTIONARY_XML: &str = r#"<fix major="4" minor="4">
  <fields>
    <field number="8" name="BeginString" type="STRING"/>
    <field number="9" name="BodyLength" type="LENGTH"/>
    <field number="10" name="CheckSum" type="STRING"/>
    <field number="11" name="ClOrdID" type="STRING"/>
    <field number="35" name="MsgType" type="STRING"/>
    <field number="38" name="OrderQty" type="QTY"/>
    <field number="40" name="OrdType" type="CHAR"/>
    <field number="44" name="Price" type="PRICE"/>
    <field number="49" name="SenderCompID" type="STRING"/>
    <field number="54" name="Side" type="CHAR">
      <value enum="1" description="BUY"/>
      <value enum="2" description="SELL"/>
    </field>
    <field number="55" name="Symbol" type="STRING"/>
    <field number="56" name="TargetCompID" type="STRING"/>
    <field number="98" name="EncryptMethod" type="INT"/>
    <field number="108" name="HeartBtInt" type="INT"/>
    <field number="112" name="TestReqID" type="STRING"/>
  </fields>
  <messages>
    <message name="Heartbeat" msgtype="0" msgcat="admin">
      <field name="TestReqID" required="N"/>
    </message>
    <message name="Logon" msgtype="A" msgcat="admin">
      <field name="EncryptMethod" required="Y"/>
      <field name="HeartBtInt" required="Y"/>
    </message>
    <message name="NewOrderSingle" msgtype="D" msgcat="app">
      <field name="ClOrdID" required="Y"/>
      <field name="Symbol" required="Y"/>
      <field name="Side" required="Y"/>
      <field name="OrderQty" required="Y"/>
      <field name="OrdType" required="Y"/>
      <field name="Price" required="N"/>
    </message>
  </messages>
</fix>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_field_and_message_counts() {
        let dict = CompiledDictionary::from_xml(TEST_DICTIONARY_XML).unwrap();
        assert_eq!(dict.field_count(), 15);
        assert_eq!(dict.message_count(), 3);
    }

    #[test]
    fn test_fix_version() {
        let dict = CompiledDictionary::from_xml(TEST_DICTIONARY_XML).unwrap();
        assert_eq!(dict.fix_version, "FIX.4.4");
    }

    #[test]
    fn test_lookup_field_known() {
        let dict = CompiledDictionary::from_xml(TEST_DICTIONARY_XML).unwrap();
        let field = dict.lookup_field(35).unwrap();
        assert_eq!(field.name, "MsgType");
        assert_eq!(field.field_type, "STRING");

        let field = dict.lookup_field(44).unwrap();
        assert_eq!(field.name, "Price");
        assert_eq!(field.field_type, "PRICE");
    }

    #[test]
    fn test_lookup_field_unknown() {
        let dict = CompiledDictionary::from_xml(TEST_DICTIONARY_XML).unwrap();
        assert!(dict.lookup_field(99999).is_none());
        assert!(dict.lookup_field(999).is_none());
    }

    #[test]
    fn test_lookup_message_known() {
        let dict = CompiledDictionary::from_xml(TEST_DICTIONARY_XML).unwrap();
        let msg = dict.lookup_message("D").unwrap();
        assert_eq!(msg.name, "NewOrderSingle");
        assert_eq!(msg.category, MessageCategory::Application);
    }

    #[test]
    fn test_lookup_message_unknown() {
        let dict = CompiledDictionary::from_xml(TEST_DICTIONARY_XML).unwrap();
        assert!(dict.lookup_message("Z").is_none());
    }

    #[test]
    fn test_validate_message_all_present() {
        let dict = CompiledDictionary::from_xml(TEST_DICTIONARY_XML).unwrap();
        let errors = dict.validate_message("A", &[98, 108]);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_message_missing_required() {
        let dict = CompiledDictionary::from_xml(TEST_DICTIONARY_XML).unwrap();
        let errors = dict.validate_message("D", &[11, 55]);
        assert_eq!(errors.len(), 3);
        assert!(errors.iter().any(|e| e.contains("Side")));
        assert!(errors.iter().any(|e| e.contains("OrderQty")));
        assert!(errors.iter().any(|e| e.contains("OrdType")));
    }

    #[test]
    fn test_o1_tag_index_lookup() {
        let dict = CompiledDictionary::from_xml(TEST_DICTIONARY_XML).unwrap();
        // tag_index provides direct indexing — verify it works for boundary tags
        assert!(dict.lookup_field(8).is_some());
        assert!(dict.lookup_field(112).is_some());
        // tags between defined values should be None
        assert!(dict.lookup_field(12).is_none());
        assert!(dict.lookup_field(100).is_none());
    }

    #[test]
    fn test_field_with_enum_values() {
        let dict = CompiledDictionary::from_xml(TEST_DICTIONARY_XML).unwrap();
        let side = dict.lookup_field(54).unwrap();
        assert_eq!(side.name, "Side");
        assert_eq!(side.values.len(), 2);
        assert_eq!(side.values[0], ("1".to_string(), "BUY".to_string()));
        assert_eq!(side.values[1], ("2".to_string(), "SELL".to_string()));
    }

    #[test]
    fn test_message_category_classification() {
        let dict = CompiledDictionary::from_xml(TEST_DICTIONARY_XML).unwrap();
        let heartbeat = dict.lookup_message("0").unwrap();
        assert_eq!(heartbeat.category, MessageCategory::Admin);
        let logon = dict.lookup_message("A").unwrap();
        assert_eq!(logon.category, MessageCategory::Admin);
        let nos = dict.lookup_message("D").unwrap();
        assert_eq!(nos.category, MessageCategory::Application);
    }

    #[test]
    fn test_error_on_malformed_xml_no_fix_tag() {
        let bad_xml = "<notfix><fields></fields></notfix>";
        let result = CompiledDictionary::from_xml(bad_xml);
        assert!(result.is_err());
        match result.unwrap_err() {
            DictError::ParseError(msg) => assert!(msg.contains("no <fix> tag")),
            other => panic!("expected ParseError, got {:?}", other),
        }
    }

    #[test]
    fn test_message_required_and_optional_fields() {
        let dict = CompiledDictionary::from_xml(TEST_DICTIONARY_XML).unwrap();
        let nos = dict.lookup_message("D").unwrap();
        assert_eq!(nos.required_fields, vec![11, 55, 54, 38, 40]);
        assert_eq!(nos.optional_fields, vec![44]);

        let hb = dict.lookup_message("0").unwrap();
        assert!(hb.required_fields.is_empty());
        assert_eq!(hb.optional_fields, vec![112]);
    }
}
