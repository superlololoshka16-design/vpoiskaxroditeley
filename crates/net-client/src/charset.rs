use core_utils::BytesExt as _;
use encoding_rs::Encoding;

pub fn charset_from_content_type(header: &str) -> Option<&str> {
    header.split(';').find_map(|part| {
        let rest = part.trim().as_bytes().strip_prefix_ci(b"charset")?;
        let value = rest.trim_ascii().strip_prefix(b"=")?.trim_ascii_extra(b"\"'");
        std::str::from_utf8(value).ok().filter(|v| !v.is_empty())
    })
}

fn meta_attribute<'a>(tag: &'a str, target: &str) -> Option<&'a str> {
    let rest = tag.strip_prefix('<')?;
    let rest = rest.as_bytes().strip_prefix_ci(b"meta")?;
    let mut rest = core_utils::utf8::basic::from_utf8(rest).ok()?;
    if rest
        .chars()
        .next()
        .is_some_and(|c| !c.is_ascii_whitespace() && !matches!(c, '>' | '/'))
    {
        return None;
    }
    while !rest.is_empty() {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '/');
        if rest.is_empty() || rest.starts_with('>') {
            break;
        }
        let name_end = rest
            .find(|c: char| c.is_ascii_whitespace() || matches!(c, '=' | '>' | '/'))
            .unwrap_or(rest.len());
        if name_end == 0 {
            let mut chars = rest.chars();
            let skip = chars.next()?.len_utf8();
            rest = &rest[skip..];
            continue;
        }
        let name = &rest[..name_end];
        rest = rest[name_end..].trim_start();
        let mut value = "";
        if let Some(after_equals) = rest.strip_prefix('=') {
            let after_equals = after_equals.trim_start();
            if let Some(quote) = after_equals
                .chars()
                .next()
                .filter(|c| matches!(c, '"' | '\''))
            {
                let quoted = &after_equals[quote.len_utf8()..];
                if let Some(end) = quoted.find(quote) {
                    value = &quoted[..end];
                    rest = &quoted[end + quote.len_utf8()..];
                } else {
                    value = quoted;
                    rest = "";
                }
            } else {
                let end = after_equals
                    .find(|c: char| c.is_ascii_whitespace() || matches!(c, '>' | '/'))
                    .unwrap_or(after_equals.len());
                value = &after_equals[..end];
                rest = &after_equals[end..];
            }
        }
        if name.as_bytes().eq_ci(target.as_bytes()) {
            return Some(value);
        }
    }
    None
}

pub fn sniff_meta_charset(bytes: &[u8]) -> Option<&str> {
    let prefix = &bytes[..bytes.len().min(1024)];
    let mut pos = 0;
    while let Some(rel) = prefix[pos..].find_sub(b"<") {
        let abs = pos + rel;
        if prefix[abs..].starts_with_ci(b"<meta") {
            let end = prefix[abs..]
                .iter()
                .position(|&b| b == b'>')
                .map(|e| abs + e)
                .unwrap_or(prefix.len());

            if let Ok(tag) = core_utils::utf8::basic::from_utf8(&prefix[abs..end]) {
                if let Some(enc) = meta_attribute(tag, "charset").filter(|v| !v.is_empty()) {
                    return Some(enc);
                }
                let is_legacy = meta_attribute(tag, "http-equiv")
                    .is_some_and(|v| v.as_bytes().eq_ci(b"content-type"));
                if is_legacy
                    && let Some(content) = meta_attribute(tag, "content")
                    && let Some(enc) = charset_from_content_type(content)
                {
                    return Some(enc);
                }
            }
            pos = end + 1;
        } else {
            pos = abs + 1;
        }
        if pos >= prefix.len() {
            break;
        }
    }
    None
}

pub fn encoding_for_label(label: &str) -> Option<&'static Encoding> {
    Encoding::for_label(label.as_bytes())
}

#[inline]
pub fn is_utf8(enc: &'static Encoding) -> bool {
    enc == encoding_rs::UTF_8
}

pub(crate) fn resolve_encoding(
    content_type: Option<&str>,
    head: &[u8],
) -> Option<&'static Encoding> {
    if let Some((enc, _)) = Encoding::for_bom(head) {
        return (!is_utf8(enc)).then_some(enc);
    }
    let from_header = content_type
        .and_then(charset_from_content_type)
        .and_then(encoding_for_label)
        .filter(|e| !is_utf8(*e));
    from_header.or_else(|| {
        sniff_meta_charset(head)
            .and_then(encoding_for_label)
            .filter(|e| !is_utf8(*e))
    })
}
