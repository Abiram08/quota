//! Tolerant accessors over `serde_json::Value`. Provider endpoints are
//! undocumented, so parsers navigate loosely: unknown fields are ignored,
//! missing ones yield `None`, numbers may arrive as strings (protobuf JSON
//! encodes int64 as strings), and snake_case/camelCase are both accepted.

use serde_json::Value;

/// Walks a path of object keys.
pub fn at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = v;
    for key in path {
        cur = cur.get(*key)?;
    }
    (!cur.is_null()).then_some(cur)
}

/// First non-null value among alternative keys (e.g. snake and camel case).
pub fn any<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| v.get(*k).filter(|x| !x.is_null()))
}

/// Number from a JSON number or numeric string.
pub fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|f| f.is_finite()),
        _ => None,
    }
}

pub fn num_any(v: &Value, keys: &[&str]) -> Option<f64> {
    any(v, keys).and_then(num)
}

pub fn str_any<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    any(v, keys).and_then(Value::as_str)
}

pub fn bool_any(v: &Value, keys: &[&str]) -> Option<bool> {
    any(v, keys).and_then(Value::as_bool)
}

/// Timestamp from RFC 3339 string, unix seconds, or unix milliseconds
/// (anything above year ~5138 in seconds is treated as milliseconds).
pub fn ts(v: &Value) -> Option<i64> {
    if let Some(s) = v.as_str() {
        if let Some(t) = crate::time::parse_rfc3339(s) {
            return Some(t);
        }
    }
    let n = num(v)?;
    if n <= 0.0 {
        return None;
    }
    let secs = if n > 1e11 { n / 1000.0 } else { n };
    Some(secs as i64)
}

pub fn ts_any(v: &Value, keys: &[&str]) -> Option<i64> {
    any(v, keys).and_then(ts)
}

/// Turns `weekly_all` / `GPT-5 codex` into `Weekly all` / `GPT-5 codex`.
pub fn humanize(key: &str) -> String {
    let s = key.replace('_', " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Base64url (or standard base64) decode without padding requirements.
/// Used only to read the `exp` claim of JWT access tokens.
pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// `exp` claim (unix seconds) of a JWT, without verifying it.
pub fn jwt_exp(token: &str) -> Option<i64> {
    let mut parts = token.split('.');
    let (_h, payload, _sig) = (parts.next()?, parts.next()?, parts.next()?);
    let bytes = b64_decode(payload)?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    v.get("exp").and_then(num).map(|e| e as i64)
}

/// True when the string looks like a JWT (three non-empty dot segments).
pub fn is_jwt(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_from_strings_and_numbers() {
        assert_eq!(num(&json!(5)), Some(5.0));
        assert_eq!(num(&json!("1762000000000")), Some(1_762_000_000_000.0));
        assert_eq!(num(&json!("x")), None);
        assert_eq!(num(&json!(null)), None);
    }

    #[test]
    fn timestamps() {
        assert_eq!(ts(&json!("1970-01-01T00:00:10Z")), Some(10));
        assert_eq!(ts(&json!(1_796_511_158)), Some(1_796_511_158));
        assert_eq!(ts(&json!("1794531839765")), Some(1_794_531_839));
        assert_eq!(ts(&json!(0)), None);
    }

    #[test]
    fn path_and_alternatives() {
        let v = json!({"a": {"b": {"c": 1}}, "camelCase": 2, "nul": null});
        assert_eq!(at(&v, &["a", "b", "c"]).and_then(num), Some(1.0));
        assert!(at(&v, &["a", "x"]).is_none());
        assert!(at(&v, &["nul"]).is_none());
        assert_eq!(num_any(&v, &["camel_case", "camelCase"]), Some(2.0));
    }

    #[test]
    fn jwt_exp_claim() {
        // header {"alg":"none"} . payload {"exp":1800000000} . sig
        let payload = "eyJleHAiOjE4MDAwMDAwMDB9";
        let token = format!("eyJhbGciOiJub25lIn0.{payload}.c2ln");
        assert!(is_jwt(&token));
        assert_eq!(jwt_exp(&token), Some(1_800_000_000));
        assert!(!is_jwt("opaque-token"));
        assert_eq!(jwt_exp("a.b"), None);
    }

    #[test]
    fn humanizes_keys() {
        assert_eq!(humanize("weekly_all"), "Weekly all");
        assert_eq!(humanize(""), "");
    }
}
