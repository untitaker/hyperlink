#[inline]
pub fn is_external_link(url: &[u8]) -> bool {
    // check if url is empty
    let first_char = match url.first() {
        Some(x) => x,
        None => return false,
    };

    // protocol-relative URL
    if url.starts_with(b"//") {
        return true;
    }

    // check if string before first : is a valid URL scheme
    // see RFC 2396, Appendix A for what constitutes a valid scheme

    if !first_char.is_ascii_alphabetic() {
        return false;
    }

    for c in &url[1..] {
        match c {
            b'a'..=b'z' => (),
            b'A'..=b'Z' => (),
            b'0'..=b'9' => (),
            b'+' => (),
            b'-' => (),
            b'.' => (),
            b':' => return true,
            _ => return false,
        }
    }

    false
}

#[test]
fn test_is_bad_schema() {
    assert!(is_external_link(b"//"));
    assert!(!is_external_link(b""));
    assert!(!is_external_link(b"http"));
    assert!(is_external_link(b"http:"));
    assert!(is_external_link(b"http:/"));
    assert!(!is_external_link(b"http/"));
}
