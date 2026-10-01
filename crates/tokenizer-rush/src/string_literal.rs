//! Decode parser-provided quoted literals without allocating intermediate strings.
pub(crate) struct Characters<'s> {
    chars: std::str::Chars<'s>,
    failed: bool,
}
pub(crate) fn characters(text: &str) -> Characters<'_> {
    let mut chars = text.chars();
    chars.next();
    chars.next_back();
    Characters {
        chars,
        failed: false,
    }
}
impl Iterator for Characters<'_> {
    type Item = Result<char, &'static str>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        let c = self.chars.next()?;
        if c != '\\' {
            return Some(Ok(c));
        }
        let decoded = match self.chars.next() {
            Some('n') => Ok('\n'),
            Some('r') => Ok('\r'),
            Some('t') => Ok('\t'),
            Some('\\') => Ok('\\'),
            Some('"') => Ok('"'),
            Some('\'') => Ok('\''),
            Some('u') => {
                if self.chars.next() != Some('{') {
                    Err("Unicode escape requires braces")
                } else {
                    let (mut code, mut digits) = (0u32, 0usize);
                    loop {
                        match self.chars.next() {
                            Some('}') if digits > 0 => {
                                break char::from_u32(code).ok_or("Invalid Unicode scalar value");
                            }
                            Some(c) if c.is_ascii_hexdigit() && digits < 6 => {
                                code = code * 16 + c.to_digit(16).unwrap();
                                digits += 1;
                            }
                            _ => break Err("Invalid Unicode escape"),
                        }
                    }
                }
            }
            _ => Err("Unsupported string escape"),
        };
        self.failed = decoded.is_err();
        Some(decoded)
    }
}
