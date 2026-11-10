//! Compile-time open-addressed hash table for $O(1)$ case-insensitive keyword and type lookup.

use super::{lexer::SyntaxKind, profile::FullProfile};

const LOOKUP_CAP: usize = 512;
const LOOKUP_MASK: usize = LOOKUP_CAP - 1;
const LOOKUP_MAX_ITEMS: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LookupSlot {
    kind: SyntaxKind,
    idx: u8,
}

impl LookupSlot {
    const EMPTY: Self = Self {
        kind: SyntaxKind::Whitespace,
        idx: 0,
    };
}

#[inline]
const fn hash_ascii_ci(bytes: &[u8]) -> u32 {
    let mut h = 0x811c_9dc5u32;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i].to_ascii_lowercase() as u32;
        h = h.wrapping_mul(0x0100_0193);
        i += 1;
    }
    h
}

#[inline]
const fn eq_ascii_ci(a: &[u8], b: &[u8]) -> bool {
    a.eq_ignore_ascii_case(b)
}

#[inline]
pub(crate) fn contains_ascii_case_insensitive(list: &[&str], needle: &str) -> bool {
    let len = needle.len();
    list.iter()
        .any(|&item| item.len() == len && item.eq_ignore_ascii_case(needle))
}

#[inline]
pub(crate) fn classify_linear(profile: &FullProfile, text: &str) -> SyntaxKind {
    if contains_ascii_case_insensitive(profile.keywords, text) {
        SyntaxKind::Keyword
    } else if contains_ascii_case_insensitive(profile.types, text) {
        SyntaxKind::TypeIdent
    } else {
        SyntaxKind::Identifier
    }
}

/// Compile-time open-addressed hash table for $O(1)$ case-insensitive keyword and type lookup.
#[derive(Debug, Clone, Copy)]
pub struct KeywordLookup {
    slots: [LookupSlot; LOOKUP_CAP],
    len_mask: u64,
    complete: bool,
}

impl KeywordLookup {
    #[must_use]
    pub const fn from_profile(profile: &FullProfile) -> Self {
        let mut slots = [LookupSlot::EMPTY; LOOKUP_CAP];
        let mut len_mask = 0u64;
        if profile.keywords.len() > LOOKUP_MAX_ITEMS
            || profile.types.len() > LOOKUP_MAX_ITEMS
            || profile.keywords.len() + profile.types.len() > 384
        {
            return Self {
                slots,
                len_mask: u64::MAX,
                complete: false,
            };
        }

        let mut i = 0;
        while i < profile.keywords.len() {
            let kw = profile.keywords[i].as_bytes();
            let bit = if kw.len() < 64 { kw.len() } else { 63 };
            len_mask |= 1u64 << bit;
            let h = hash_ascii_ci(kw) as usize;
            let mut step = 0;
            while step < LOOKUP_CAP {
                let idx = (h + step) & LOOKUP_MASK;
                match slots[idx].kind {
                    SyntaxKind::Whitespace => {
                        slots[idx] = LookupSlot {
                            kind: SyntaxKind::Keyword,
                            idx: i as u8,
                        };
                        break;
                    }
                    SyntaxKind::Keyword => {
                        let existing = profile.keywords[slots[idx].idx as usize].as_bytes();
                        if eq_ascii_ci(existing, kw) {
                            break;
                        }
                    }
                    _ => {}
                }
                step += 1;
            }
            i += 1;
        }

        let mut j = 0;
        while j < profile.types.len() {
            let ty = profile.types[j].as_bytes();
            let bit = if ty.len() < 64 { ty.len() } else { 63 };
            len_mask |= 1u64 << bit;
            let h = hash_ascii_ci(ty) as usize;
            let mut step = 0;
            while step < LOOKUP_CAP {
                let idx = (h + step) & LOOKUP_MASK;
                match slots[idx].kind {
                    SyntaxKind::Whitespace => {
                        slots[idx] = LookupSlot {
                            kind: SyntaxKind::TypeIdent,
                            idx: j as u8,
                        };
                        break;
                    }
                    SyntaxKind::Keyword => {
                        let existing = profile.keywords[slots[idx].idx as usize].as_bytes();
                        if eq_ascii_ci(existing, ty) {
                            break;
                        }
                    }
                    SyntaxKind::TypeIdent => {
                        let existing = profile.types[slots[idx].idx as usize].as_bytes();
                        if eq_ascii_ci(existing, ty) {
                            break;
                        }
                    }
                    _ => {}
                }
                step += 1;
            }
            j += 1;
        }

        Self {
            slots,
            len_mask,
            complete: true,
        }
    }

    #[inline]
    #[must_use]
    pub fn classify(&self, profile: &FullProfile, ident: &str) -> SyntaxKind {
        let len_bit = 1u64 << ident.len().min(63);
        if (self.len_mask & len_bit) == 0 {
            return SyntaxKind::Identifier;
        }
        if !self.complete {
            return classify_linear(profile, ident);
        }
        let h = hash_ascii_ci(ident.as_bytes()) as usize;
        let mut step = 0;
        while step < LOOKUP_CAP {
            let slot = self.slots[(h + step) & LOOKUP_MASK];
            match slot.kind {
                SyntaxKind::Whitespace => return SyntaxKind::Identifier,
                SyntaxKind::Keyword => {
                    if profile.keywords[slot.idx as usize].eq_ignore_ascii_case(ident) {
                        return SyntaxKind::Keyword;
                    }
                }
                SyntaxKind::TypeIdent => {
                    if profile.types[slot.idx as usize].eq_ignore_ascii_case(ident) {
                        return SyntaxKind::TypeIdent;
                    }
                }
                _ => return SyntaxKind::Identifier,
            }
            step += 1;
        }
        SyntaxKind::Identifier
    }
}
