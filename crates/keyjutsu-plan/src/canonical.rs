//! Canonical JSON, as RFC 8785 (JSON Canonicalization Scheme) defines it.
//!
//! A hash is only as trustworthy as the bytes it is taken over. Two
//! serialisations of the same plan must produce the same bytes, whatever order
//! the keys arrived in and however a number was written, or approval would be
//! withdrawn by a reformat and, worse, could survive a change that happened to
//! reformat back. RFC 8785 fixes all of that: object keys sorted by UTF-16
//! code units, no insignificant whitespace, strings escaped as ECMAScript's
//! `JSON.stringify` does, and numbers written as ECMAScript's `Number.toString`
//! writes them.

use serde_json::Value;

/// The canonical form of `value`.
pub fn canonicalise(value: &Value) -> String {
    let mut out = String::new();
    write_value(value, &mut out);
    out
}

fn write_value(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&number(n)),
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            // RFC 8785 §3.2.3: sort by UTF-16 code units, not by UTF-8 bytes
            // or by char. The two orders differ for characters beyond U+FFFF.
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write_value(&map[key], out);
            }
            out.push('}');
        }
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// ECMAScript `Number.prototype.toString` for a JSON number.
fn number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    es_number(n.as_f64().unwrap_or(0.0))
}

fn es_number(x: f64) -> String {
    if x == 0.0 {
        return "0".into(); // Covers -0 as well.
    }
    if x < 0.0 {
        return format!("-{}", es_number(-x));
    }
    // Rust's `{:e}` gives the shortest digits that round-trip, which is what
    // ECMAScript asks for: "d.ddd" and a decimal exponent.
    let sci = format!("{x:e}");
    let (mantissa, exponent) = sci.split_once('e').unwrap_or((&sci, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let k = digits.len() as i32;
    let n = exponent.parse::<i32>().unwrap_or(0) + 1;

    if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let sign = if e < 0 { '-' } else { '+' };
        if k == 1 {
            format!("{digits}e{sign}{}", e.abs())
        } else {
            format!("{}.{}e{sign}{}", &digits[..1], &digits[1..], e.abs())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn num(text: &str) -> String {
        canonicalise(&serde_json::from_str::<Value>(text).unwrap())
    }

    #[test]
    fn numbers_follow_ecmascript_as_rfc_8785_requires() {
        // Test vectors from RFC 8785 §3.2.2.3 and Appendix B.
        assert_eq!(num("333333333.33333329"), "333333333.3333333");
        assert_eq!(num("1E30"), "1e+30");
        assert_eq!(num("4.50"), "4.5");
        assert_eq!(num("2e-3"), "0.002");
        assert_eq!(num("0.000000000000000000000000001"), "1e-27");
        assert_eq!(num("-0"), "0");
        assert_eq!(num("1e21"), "1e+21");
        assert_eq!(num("1e20"), "100000000000000000000");
        assert_eq!(num("0.000001"), "0.000001");
        assert_eq!(num("0.0000001"), "1e-7");
        assert_eq!(num("123.456e-10"), "1.23456e-8");
        assert_eq!(num("9007199254740993"), "9007199254740993", "integers stay exact");
        assert_eq!(num("-12.5"), "-12.5");
    }

    #[test]
    fn keys_sort_by_utf16_code_units() {
        // RFC 8785 §3.2.3's example: U+1F600 (surrogates D83D DE00) sorts
        // before U+FB33 in UTF-16, though after it by code point.
        let v = json!({"\u{fb33}": 1, "\u{1f600}": 2, "a": 3, "\r": 4});
        assert_eq!(canonicalise(&v), "{\"\\r\":4,\"a\":3,\"\u{1f600}\":2,\"\u{fb33}\":1}");
    }

    #[test]
    fn strings_escape_only_what_json_stringify_escapes() {
        let v = json!("quote\" slash\\ tab\t bell\u{7} é / \u{2028}");
        assert_eq!(canonicalise(&v), "\"quote\\\" slash\\\\ tab\\t bell\\u0007 é / \u{2028}\"");
    }

    #[test]
    fn whitespace_and_key_order_in_the_input_do_not_matter() {
        let a: Value = serde_json::from_str(r#"{ "b": [1, 2, {"y": true, "x": null}], "a": "s" }"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"a":"s","b":[1,2,{"x":null,"y":true}]}"#).unwrap();
        assert_eq!(canonicalise(&a), canonicalise(&b));
        assert_eq!(canonicalise(&a), r#"{"a":"s","b":[1,2,{"x":null,"y":true}]}"#);
    }
}
