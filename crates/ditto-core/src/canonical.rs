//! Canonical JSON encoding for content addressing.
//!
//! Sorted object keys, no whitespace, no insignificant zeros, UTF-8. This must
//! match `ditto_eval.types._canonical_json` in the Python eval harness so that
//! event_ids computed in Python and Rust are bit-identical.
//!
//! This is a deliberately small subset of RFC 8785 JCS. We accept JSON values
//! that have already been parsed by `serde_json`; we do not attempt to handle
//! the full JCS number-formatting spec (the eval harness does not produce
//! exotic numbers, and we control both ends of the wire).

use serde_json::Value;

use crate::error::Error;

/// Serialize a `serde_json::Value` as canonical JSON bytes.
pub fn to_canonical_bytes(value: &Value) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(128);
    write_value(value, &mut out)?;
    Ok(out)
}

fn write_value(value: &Value, out: &mut Vec<u8>) -> Result<(), Error> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::String(s) => write_string(s, out),
        Value::Array(arr) => {
            out.push(b'[');
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_value(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(obj) => {
            let mut keys: Vec<&String> = obj.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_string(k, out);
                out.push(b':');
                write_value(&obj[*k], out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

fn write_string(s: &str, out: &mut Vec<u8>) {
    out.push(b'"');
    for c in s.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\u{08}' => out.extend_from_slice(b"\\b"),
            '\u{0c}' => out.extend_from_slice(b"\\f"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes());
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sorted_object_keys() {
        let a = to_canonical_bytes(&json!({"b": 1, "a": 2})).unwrap();
        let b = to_canonical_bytes(&json!({"a": 2, "b": 1})).unwrap();
        assert_eq!(a, b);
        assert_eq!(&a, br#"{"a":2,"b":1}"#);
    }

    #[test]
    fn no_whitespace() {
        let out = to_canonical_bytes(&json!({"x": [1, 2, 3]})).unwrap();
        assert_eq!(&out, br#"{"x":[1,2,3]}"#);
    }

    #[test]
    fn escapes_control_chars() {
        let out = to_canonical_bytes(&json!("a\nb")).unwrap();
        assert_eq!(&out, br#""a\nb""#);
    }

    #[test]
    fn matches_python_event_id_fixture() {
        // Must match ditto_eval.types.content_address({"content": "hi", "ts": 1})
        let bytes = to_canonical_bytes(&json!({"content": "hi", "ts": 1})).unwrap();
        assert_eq!(&bytes, br#"{"content":"hi","ts":1}"#);
    }

    #[test]
    fn nested_objects_sort_recursively() {
        let a = to_canonical_bytes(&json!({"outer": {"b": 1, "a": 2}})).unwrap();
        let b = to_canonical_bytes(&json!({"outer": {"a": 2, "b": 1}})).unwrap();
        assert_eq!(a, b);
    }
}
