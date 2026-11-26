//! The page's state in a URL fragment, so a link reproduces it:
//! `#e=<expression>&v=<name>,<type>,<value>&…&fuel=N&memory=N&depth=N&len=N`,
//! each part percent-encoded (`,` and `&` included).

/// What a link carries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shared {
    pub source: String,
    pub vars: Vec<(String, String, String)>,
    pub fuel: Option<u64>,
    pub memory: Option<u32>,
    pub depth: Option<usize>,
    pub len: Option<usize>,
}

/// The fragment for `shared`, without its `#`.
pub fn encode(shared: &Shared) -> String {
    let mut out = format!("e={}", escape(&shared.source));

    for (name, ty, value) in &shared.vars {
        out.push_str(&format!(
            "&v={},{},{}",
            escape(name),
            escape(ty),
            escape(value)
        ));
    }

    if let Some(fuel) = shared.fuel {
        out.push_str(&format!("&fuel={fuel}"));
    }

    if let Some(memory) = shared.memory {
        out.push_str(&format!("&memory={memory}"));
    }

    if let Some(depth) = shared.depth {
        out.push_str(&format!("&depth={depth}"));
    }

    if let Some(len) = shared.len {
        out.push_str(&format!("&len={len}"));
    }

    out
}

/// Read a fragment, with or without its `#`. `None` when it is empty or
/// any part of it is malformed, so the page falls back to its default.
pub fn decode(fragment: &str) -> Option<Shared> {
    let fragment = fragment.strip_prefix('#').unwrap_or(fragment);

    if fragment.is_empty() {
        return None;
    }

    let mut shared = Shared::default();
    let mut source = None;

    for part in fragment.split('&') {
        let (key, value) = part.split_once('=')?;

        match key {
            "e" => source = Some(unescape(value)?),
            "v" => {
                let mut parts = value.split(',');
                let name = unescape(parts.next()?)?;
                let ty = unescape(parts.next()?)?;
                let value = unescape(parts.next()?)?;

                if parts.next().is_some() {
                    return None;
                }

                shared.vars.push((name, ty, value));
            }
            "fuel" => shared.fuel = Some(value.parse().ok()?),
            "memory" => shared.memory = Some(value.parse().ok()?),
            "depth" => shared.depth = Some(value.parse().ok()?),
            "len" => shared.len = Some(value.parse().ok()?),
            _ => return None,
        }
    }

    shared.source = source?;
    Some(shared)
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());

    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }

    out
}

fn unescape(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;

    while let Some(&byte) = bytes.get(at) {
        if byte == b'%' {
            let hex = bytes.get(at.saturating_add(1)..at.saturating_add(3))?;
            out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            at = at.saturating_add(3);
        } else {
            out.push(byte);
            at = at.saturating_add(1);
        }
    }

    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let shared = Shared {
            source: "health.boxes - health.count[s] & \"é, 100%\"".into(),
            vars: vec![
                (
                    "health".into(),
                    "{boxes: Int, count: List<Int>}".into(),
                    "{boxes: 7, count: [3, 1, 0]}".into(),
                ),
                ("s".into(), "Int".into(), "1".into()),
            ],
            fuel: Some(20),
            memory: None,
            depth: Some(8),
            len: Some(100),
        };

        let fragment = encode(&shared);
        assert!(!fragment.contains(' '));
        assert_eq!(decode(&format!("#{fragment}")), Some(shared));
    }

    #[test]
    fn malformed_is_none() {
        for fragment in [
            "",
            "#",
            "#e=%ZZ",
            "#e=%E9",
            "#e=1&v=a,Int",
            "#e=1&v=a,Int,1,2",
            "#e=1&fuel=-1",
            "#e=1&what=2",
            "#v=a,Int,1",
            "#e",
            "#e=%4",
        ] {
            assert_eq!(decode(fragment), None, "{fragment}");
        }
    }
}
