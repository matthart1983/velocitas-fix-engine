/// Repeating group parser for FIX messages with nested group support.
///
/// FIX repeating groups are defined by a "NumInGroup" count tag followed by
/// that many repetitions of a fixed set of fields. Each repetition starts
/// with the same delimiter tag. Groups can be nested.
use crate::message::FieldEntry;

/// Definition of a repeating group's structure.
#[derive(Debug, Clone)]
pub struct GroupDef {
    /// The "count" tag (e.g., 268 for NoMDEntries)
    pub count_tag: u32,
    /// The delimiter tag — first tag in each group entry
    pub delimiter_tag: u32,
    /// All valid tags within this group (including delimiter)
    pub member_tags: Vec<u32>,
    /// Nested group definitions within this group
    pub nested_groups: Vec<GroupDef>,
}

/// A parsed repeating group instance.
#[derive(Debug)]
pub struct RepeatingGroup<'a> {
    count_tag: u32,
    count: usize,
    entries: Vec<GroupEntry<'a>>,
}

/// A single entry within a repeating group.
#[derive(Debug)]
pub struct GroupEntry<'a> {
    fields: Vec<(u32, &'a [u8])>,
    nested: Vec<RepeatingGroup<'a>>,
}

impl GroupDef {
    /// Create a new group definition.
    pub fn new(count_tag: u32, delimiter_tag: u32, member_tags: Vec<u32>) -> Self {
        GroupDef {
            count_tag,
            delimiter_tag,
            member_tags,
            nested_groups: Vec::new(),
        }
    }

    /// Add a nested group definition. Builder pattern.
    pub fn with_nested(mut self, nested: GroupDef) -> Self {
        self.nested_groups.push(nested);
        self
    }

    fn is_member(&self, tag: u32) -> bool {
        self.member_tags.contains(&tag)
    }

    fn find_nested(&self, tag: u32) -> Option<&GroupDef> {
        self.nested_groups.iter().find(|g| g.count_tag == tag)
    }
}

impl<'a> RepeatingGroup<'a> {
    /// Parse a repeating group from a buffer and its field entries.
    ///
    /// `buffer` — the raw FIX message bytes.
    /// `field_entries` — the slice of parsed `FieldEntry` items from the `MessageView`.
    /// `start_index` — the index in `field_entries` where the count tag lives.
    /// `group_def` — the structural definition of this group.
    ///
    /// Returns `(RepeatingGroup, next_index)` where `next_index` is the field index
    /// immediately after the last field consumed by this group.
    pub fn parse(
        buffer: &'a [u8],
        field_entries: &[FieldEntry],
        start_index: usize,
        group_def: &GroupDef,
    ) -> Option<(RepeatingGroup<'a>, usize)> {
        if start_index >= field_entries.len() {
            return None;
        }

        let count_entry = &field_entries[start_index];
        if count_entry.tag != group_def.count_tag {
            return None;
        }

        let count = parse_usize_from_buf(buffer, count_entry)?;

        if count == 0 {
            return Some((
                RepeatingGroup {
                    count_tag: group_def.count_tag,
                    count: 0,
                    entries: Vec::new(),
                },
                start_index + 1,
            ));
        }

        let mut entries = Vec::with_capacity(count);
        let mut idx = start_index + 1;

        for _ in 0..count {
            if idx >= field_entries.len() || field_entries[idx].tag != group_def.delimiter_tag {
                break;
            }

            let mut fields = Vec::new();
            let mut nested = Vec::new();

            // Consume the delimiter field
            fields.push((
                field_entries[idx].tag,
                field_value(buffer, &field_entries[idx]),
            ));
            idx += 1;

            // Consume remaining member fields for this entry
            while idx < field_entries.len() {
                let tag = field_entries[idx].tag;

                // If we hit the delimiter again, this entry is done
                if tag == group_def.delimiter_tag {
                    break;
                }

                // Check for nested group count tags
                if let Some(nested_def) = group_def.find_nested(tag) {
                    if let Some((nested_group, next)) =
                        RepeatingGroup::parse(buffer, field_entries, idx, nested_def)
                    {
                        nested.push(nested_group);
                        idx = next;
                        continue;
                    }
                }

                // If it's a member tag, consume it
                if group_def.is_member(tag) {
                    fields.push((tag, field_value(buffer, &field_entries[idx])));
                    idx += 1;
                } else {
                    // Non-member tag — this entry (and group) is done
                    break;
                }
            }

            entries.push(GroupEntry { fields, nested });
        }

        let actual_count = entries.len();
        Some((
            RepeatingGroup {
                count_tag: group_def.count_tag,
                count: actual_count,
                entries,
            },
            idx,
        ))
    }

    /// Number of entries in this group.
    #[inline]
    pub fn count(&self) -> usize {
        self.count
    }

    /// Get an entry by index.
    #[inline]
    pub fn get_entry(&self, index: usize) -> Option<&GroupEntry<'a>> {
        self.entries.get(index)
    }
}

impl<'a> GroupEntry<'a> {
    /// Get a field value by tag.
    #[inline]
    pub fn get_field(&self, tag: u32) -> Option<&'a [u8]> {
        self.fields.iter().find(|(t, _)| *t == tag).map(|(_, v)| *v)
    }

    /// Get a field value as `&str`.
    #[inline]
    pub fn get_field_str(&self, tag: u32) -> Option<&'a str> {
        self.get_field(tag)
            .and_then(|b| std::str::from_utf8(b).ok())
    }

    /// Get a field value as `i64`.
    #[inline]
    pub fn get_field_i64(&self, tag: u32) -> Option<i64> {
        self.get_field(tag).and_then(parse_i64_bytes)
    }

    /// Get a nested repeating group by its count tag.
    #[inline]
    pub fn get_nested(&self, count_tag: u32) -> Option<&RepeatingGroup<'a>> {
        self.nested.iter().find(|g| g.count_tag == count_tag)
    }
}

/// Extract the value slice from a FieldEntry.
#[inline]
fn field_value<'a>(buffer: &'a [u8], entry: &FieldEntry) -> &'a [u8] {
    let offset = entry.offset as usize;
    let length = entry.length as usize;
    &buffer[offset..offset + length]
}

/// Parse a usize from the value of a FieldEntry.
#[inline]
fn parse_usize_from_buf(buffer: &[u8], entry: &FieldEntry) -> Option<usize> {
    let val = field_value(buffer, entry);
    let mut result: usize = 0;
    for &b in val {
        if b < b'0' || b > b'9' {
            return None;
        }
        result = result * 10 + (b - b'0') as usize;
    }
    Some(result)
}

/// Parse i64 from ASCII bytes without allocation.
#[inline]
fn parse_i64_bytes(bytes: &[u8]) -> Option<i64> {
    if bytes.is_empty() {
        return None;
    }
    let (negative, start) = if bytes[0] == b'-' {
        (true, 1)
    } else {
        (false, 0)
    };
    let mut result: i64 = 0;
    for &b in &bytes[start..] {
        if b < b'0' || b > b'9' {
            return None;
        }
        result = result * 10 + (b - b'0') as i64;
    }
    Some(if negative { -result } else { result })
}

// ---------------------------------------------------------------------------
// Standard group definitions
// ---------------------------------------------------------------------------

/// NoMDEntries (268): MDEntryType(269), MDEntryPx(270), MDEntrySize(271), MDUpdateAction(279).
pub fn md_entries_group() -> GroupDef {
    GroupDef::new(268, 269, vec![269, 270, 271, 279])
}

/// NoLegs (555): LegSymbol(600), LegSide(624), LegQty(687).
pub fn legs_group() -> GroupDef {
    GroupDef::new(555, 600, vec![600, 624, 687])
}

/// NoFills (1362): FillExecID(1363), FillPx(1364), FillQty(1365).
pub fn fills_group() -> GroupDef {
    GroupDef::new(1362, 1363, vec![1363, 1364, 1365])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MessageView;

    /// Build a MessageView from raw FIX-like bytes (SOH-delimited tag=value pairs).
    /// Skips real checksum/body-length validation — just populates field entries.
    fn build_view(msg: &[u8]) -> MessageView<'_> {
        let mut view = MessageView::new(msg);
        let mut pos = 0;
        let len = msg.len();

        while pos < len {
            let mut tag: u32 = 0;
            while pos < len && msg[pos] != b'=' {
                if msg[pos] >= b'0' && msg[pos] <= b'9' {
                    tag = tag * 10 + (msg[pos] - b'0') as u32;
                }
                pos += 1;
            }
            if pos >= len {
                break;
            }
            pos += 1; // skip '='
            let value_start = pos;
            while pos < len && msg[pos] != 0x01 {
                pos += 1;
            }
            let value_len = pos - value_start;
            view.add_field(tag, value_start as u32, value_len as u16);
            if pos < len {
                pos += 1; // skip SOH
            }
        }
        view
    }

    #[test]
    fn test_parse_three_md_entries() {
        // MarketDataIncrementalRefresh with 3 MD entries
        let msg = b"268=3\x01\
                     269=0\x01270=100.25\x01271=500\x01279=0\x01\
                     269=1\x01270=99.75\x01271=300\x01279=0\x01\
                     269=2\x01270=100.50\x01271=200\x01279=1\x01";

        let view = build_view(msg);
        let def = md_entries_group();
        let (group, next_idx) = RepeatingGroup::parse(msg, view.fields(), 0, &def).unwrap();

        assert_eq!(group.count(), 3);
        assert_eq!(next_idx, view.field_count());

        // Entry 0
        let e0 = group.get_entry(0).unwrap();
        assert_eq!(e0.get_field_str(269), Some("0"));
        assert_eq!(e0.get_field_str(270), Some("100.25"));
        assert_eq!(e0.get_field_i64(271), Some(500));
        assert_eq!(e0.get_field_i64(279), Some(0));

        // Entry 1
        let e1 = group.get_entry(1).unwrap();
        assert_eq!(e1.get_field_str(269), Some("1"));
        assert_eq!(e1.get_field_str(270), Some("99.75"));
        assert_eq!(e1.get_field_i64(271), Some(300));

        // Entry 2
        let e2 = group.get_entry(2).unwrap();
        assert_eq!(e2.get_field_str(269), Some("2"));
        assert_eq!(e2.get_field_str(270), Some("100.50"));
        assert_eq!(e2.get_field_i64(271), Some(200));
        assert_eq!(e2.get_field_i64(279), Some(1));
    }

    #[test]
    fn test_parse_nested_groups() {
        // An MD entry group where each entry can contain nested legs
        let msg = b"268=2\x01\
                     269=0\x01270=100.00\x01271=1000\x01\
                     555=2\x01600=AAPL\x01624=1\x01687=500\x01600=MSFT\x01624=2\x01687=500\x01\
                     269=1\x01270=99.50\x01271=2000\x01\
                     555=1\x01600=GOOG\x01624=1\x01687=2000\x01";

        let view = build_view(msg);
        let def = md_entries_group().with_nested(legs_group());
        let (group, next_idx) = RepeatingGroup::parse(msg, view.fields(), 0, &def).unwrap();

        assert_eq!(group.count(), 2);
        assert_eq!(next_idx, view.field_count());

        // Entry 0: has 2 nested legs
        let e0 = group.get_entry(0).unwrap();
        assert_eq!(e0.get_field_str(269), Some("0"));
        assert_eq!(e0.get_field_str(270), Some("100.00"));

        let legs0 = e0.get_nested(555).unwrap();
        assert_eq!(legs0.count(), 2);
        assert_eq!(legs0.get_entry(0).unwrap().get_field_str(600), Some("AAPL"));
        assert_eq!(legs0.get_entry(0).unwrap().get_field_i64(624), Some(1));
        assert_eq!(legs0.get_entry(0).unwrap().get_field_i64(687), Some(500));
        assert_eq!(legs0.get_entry(1).unwrap().get_field_str(600), Some("MSFT"));

        // Entry 1: has 1 nested leg
        let e1 = group.get_entry(1).unwrap();
        assert_eq!(e1.get_field_str(269), Some("1"));
        let legs1 = e1.get_nested(555).unwrap();
        assert_eq!(legs1.count(), 1);
        assert_eq!(legs1.get_entry(0).unwrap().get_field_str(600), Some("GOOG"));
    }

    #[test]
    fn test_empty_group() {
        let msg = b"268=0\x01";
        let view = build_view(msg);
        let def = md_entries_group();
        let (group, next_idx) = RepeatingGroup::parse(msg, view.fields(), 0, &def).unwrap();

        assert_eq!(group.count(), 0);
        assert_eq!(next_idx, 1);
        assert!(group.get_entry(0).is_none());
    }

    #[test]
    fn test_single_entry_group() {
        let msg = b"268=1\x01269=0\x01270=42.00\x01271=100\x01279=0\x01";
        let view = build_view(msg);
        let def = md_entries_group();
        let (group, _) = RepeatingGroup::parse(msg, view.fields(), 0, &def).unwrap();

        assert_eq!(group.count(), 1);
        let entry = group.get_entry(0).unwrap();
        assert_eq!(entry.get_field_str(269), Some("0"));
        assert_eq!(entry.get_field_str(270), Some("42.00"));
        assert_eq!(entry.get_field_i64(271), Some(100));
    }

    #[test]
    fn test_get_field_returns_none_for_missing() {
        let msg = b"268=1\x01269=0\x01270=50.00\x01";
        let view = build_view(msg);
        let def = md_entries_group();
        let (group, _) = RepeatingGroup::parse(msg, view.fields(), 0, &def).unwrap();

        let entry = group.get_entry(0).unwrap();
        // tag 271 (MDEntrySize) not present in this entry
        assert!(entry.get_field(271).is_none());
        assert!(entry.get_field_str(271).is_none());
        assert!(entry.get_field_i64(271).is_none());
        // tag 279 (MDUpdateAction) not present either
        assert!(entry.get_field(279).is_none());
        // completely unknown tag
        assert!(entry.get_field(9999).is_none());
    }

    #[test]
    fn test_get_field_i64_and_str() {
        let msg = b"1362=2\x01\
                     1363=FILL001\x011364=150\x011365=100\x01\
                     1363=FILL002\x011364=151\x011365=200\x01";

        let view = build_view(msg);
        let def = fills_group();
        let (group, _) = RepeatingGroup::parse(msg, view.fields(), 0, &def).unwrap();

        assert_eq!(group.count(), 2);

        let e0 = group.get_entry(0).unwrap();
        assert_eq!(e0.get_field_str(1363), Some("FILL001"));
        assert_eq!(e0.get_field_i64(1364), Some(150));
        assert_eq!(e0.get_field_i64(1365), Some(100));

        let e1 = group.get_entry(1).unwrap();
        assert_eq!(e1.get_field_str(1363), Some("FILL002"));
        assert_eq!(e1.get_field_i64(1364), Some(151));
        assert_eq!(e1.get_field_i64(1365), Some(200));
    }

    #[test]
    fn test_get_nested_returns_none_when_absent() {
        let msg = b"268=1\x01269=0\x01270=50.00\x01";
        let view = build_view(msg);
        let def = md_entries_group();
        let (group, _) = RepeatingGroup::parse(msg, view.fields(), 0, &def).unwrap();

        let entry = group.get_entry(0).unwrap();
        assert!(entry.get_nested(555).is_none());
    }

    #[test]
    fn test_parse_returns_none_for_wrong_count_tag() {
        let msg = b"269=0\x01";
        let view = build_view(msg);
        let def = md_entries_group();
        assert!(RepeatingGroup::parse(msg, view.fields(), 0, &def).is_none());
    }

    #[test]
    fn test_group_at_offset_in_larger_message() {
        // Simulate fields before the group (e.g., header fields)
        let msg = b"35=X\x0149=SENDER\x0156=TARGET\x01\
                     268=2\x01\
                     269=0\x01270=100.00\x01271=500\x01\
                     269=1\x01270=99.00\x01271=300\x01\
                     10=000\x01";

        let view = build_view(msg);
        let def = md_entries_group();
        // Count tag is at field index 3 (after 35, 49, 56)
        let (group, next_idx) = RepeatingGroup::parse(msg, view.fields(), 3, &def).unwrap();

        assert_eq!(group.count(), 2);
        assert_eq!(
            group.get_entry(0).unwrap().get_field_str(270),
            Some("100.00")
        );
        assert_eq!(group.get_entry(1).unwrap().get_field_i64(271), Some(300));
        // next_idx should point to the checksum field (tag 10)
        assert_eq!(view.fields()[next_idx].tag, 10);
    }

    #[test]
    fn test_legs_group_def() {
        let def = legs_group();
        assert_eq!(def.count_tag, 555);
        assert_eq!(def.delimiter_tag, 600);
        assert!(def.member_tags.contains(&600));
        assert!(def.member_tags.contains(&624));
        assert!(def.member_tags.contains(&687));
    }

    #[test]
    fn test_fills_group_def() {
        let def = fills_group();
        assert_eq!(def.count_tag, 1362);
        assert_eq!(def.delimiter_tag, 1363);
        assert!(def.member_tags.contains(&1363));
        assert!(def.member_tags.contains(&1364));
        assert!(def.member_tags.contains(&1365));
    }

    #[test]
    fn test_with_nested_builder() {
        let def = md_entries_group()
            .with_nested(legs_group())
            .with_nested(fills_group());
        assert_eq!(def.nested_groups.len(), 2);
        assert_eq!(def.nested_groups[0].count_tag, 555);
        assert_eq!(def.nested_groups[1].count_tag, 1362);
    }
}
