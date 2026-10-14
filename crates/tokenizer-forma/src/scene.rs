//! The deliberately small, shared Forma markup frontend for the vector demo.
//! Unsupported syntax is an error rather than an invisible no-op.

use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub name: String,
    pub width: f32,
    pub height: f32,
    pub background: [u8; 4],
    pub overflow: String,
    pub clip: bool,
    pub radius: f32,
    pub scroll: bool,
    pub padding: [f32; 4],
    pub content_width: f32,
    pub content_height: f32,
    pub button: ButtonSpec,
    pub buttons: Vec<ButtonSpec>,
    pub gap: f32,
    /// Modal layers in stacking order. Their controls follow the base controls
    /// in `buttons`; `ModalSpec::controls` gives each layer's index range.
    pub modals: Vec<ModalSpec>,
}

/// One modal layer: a surface rectangle over the whole frame, a translucent
/// backdrop and the controls that stay interactive while the layer is open.
#[derive(Debug, Clone, PartialEq)]
pub struct ModalSpec {
    pub key: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub backdrop: [u8; 4],
    pub dismiss: bool,
    pub open: bool,
    pub action: Option<String>,
    pub controls: std::ops::Range<usize>,
}

/// Which properties the document actually declared for a control. The parser accepts a
/// closed vocabulary, so this is a bitset over that list: as a `HashSet<String>` every
/// control paid one allocation per property name, and every copy of the spec paid them
/// again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Specified(u32);

/// The properties a `Button` may declare, in bit order.
const SPECIFIED_NAMES: [&str; 20] = [
    "key",
    "x",
    "y",
    "width",
    "height",
    "radius",
    "background",
    "color",
    "font.size",
    "fontSize",
    "hoverBackground",
    "pressedBackground",
    "disabledBackground",
    "borderColor",
    "focusBorderColor",
    "borderWidth",
    "transitionDuration",
    "text",
    "disabled",
    "clicked",
];

impl Specified {
    const FONT_SIZE: &'static str = "fontSize";
    const FONT_SIZE_ALIAS: &'static str = "font.size";

    fn bit(name: &str) -> Option<u32> {
        SPECIFIED_NAMES
            .iter()
            .position(|known| *known == name)
            .map(|index| 1u32 << index)
    }

    pub fn contains(&self, name: &str) -> bool {
        Self::bit(name).is_some_and(|bit| self.0 & bit != 0)
    }

    /// Folds the `font.size` spelling into the canonical `fontSize` bit, so a property is
    /// recorded once however the document wrote it. Run after parsing, once both spellings
    /// have had their own bit for the duplicate and alias checks.
    fn canonicalize(&mut self) {
        let alias = Self::bit(Self::FONT_SIZE_ALIAS).unwrap_or(0);
        if self.0 & alias != 0 {
            self.0 &= !alias;
            self.0 |= Self::bit(Self::FONT_SIZE).unwrap_or(0);
        }
    }

    /// Records a declared property and reports whether it was new. Names outside the
    /// vocabulary report as new: a caller rejects an unsupported property on its own.
    pub fn try_insert(&mut self, name: &str) -> bool {
        match Self::bit(name) {
            Some(bit) if self.0 & bit != 0 => false,
            Some(bit) => {
                self.0 |= bit;
                true
            }
            None => true,
        }
    }
}

/// A colour property a control may override, addressed by its document name.
///
/// The template language accepts a closed list of these, so each one gets a slot instead of
/// an entry in a `HashMap<String, _>`: as a map every spec allocated a key string per name
/// plus its table, and every copy of a spec — one per control on a load — allocated them
/// again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorProps {
    pub hover_background: [u8; 4],
    pub pressed_background: [u8; 4],
    pub disabled_background: [u8; 4],
    pub border: [u8; 4],
    pub focus_border: [u8; 4],
}

/// The number properties a control may override; see [`ColorProps`] for the shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberProps {
    pub border_width: f32,
    pub transition_duration: f32,
}

impl Default for ColorProps {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Default for NumberProps {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl ColorProps {
    /// The demo's default palette, spelled once here rather than per spec.
    pub const DEFAULT: Self = Self {
        hover_background: [168, 186, 255, 255],
        pressed_background: [106, 131, 218, 255],
        disabled_background: [89, 98, 115, 255],
        border: [190, 208, 255, 255],
        focus_border: [255; 4],
    };

    /// Looks a property up by the name the markup and the template language use.
    pub fn get(&self, name: &str) -> Option<[u8; 4]> {
        Some(match name {
            "hoverBackground" => self.hover_background,
            "pressedBackground" => self.pressed_background,
            "disabledBackground" => self.disabled_background,
            "borderColor" => self.border,
            "focusBorderColor" => self.focus_border,
            _ => return None,
        })
    }

    /// Names outside the vocabulary are ignored: only the two parsers fill this in, and each
    /// rejects an unsupported property before reaching here.
    pub fn set(&mut self, name: &str, value: [u8; 4]) {
        match name {
            "hoverBackground" => self.hover_background = value,
            "pressedBackground" => self.pressed_background = value,
            "disabledBackground" => self.disabled_background = value,
            "borderColor" => self.border = value,
            "focusBorderColor" => self.focus_border = value,
            _ => {}
        }
    }
}

impl NumberProps {
    pub const DEFAULT: Self = Self {
        border_width: 1.,
        transition_duration: 140.,
    };

    pub fn get(&self, name: &str) -> Option<f32> {
        Some(match name {
            "borderWidth" => self.border_width,
            "transitionDuration" => self.transition_duration,
            _ => return None,
        })
    }

    pub fn set(&mut self, name: &str, value: f32) {
        match name {
            "borderWidth" => self.border_width = value,
            "transitionDuration" => self.transition_duration = value,
            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ButtonSpec {
    pub specified: Specified,
    pub colors: ColorProps,
    pub numbers: NumberProps,
    pub key: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub background: [u8; 4],
    pub color: [u8; 4],
    pub font_size: f32,
    pub text: String,
    pub disabled: bool,
    pub action: Option<String>,
}

impl Default for ButtonSpec {
    fn default() -> Self {
        Self {
            specified: Specified::default(),
            colors: ColorProps::DEFAULT,
            numbers: NumberProps::DEFAULT,
            key: String::new(),
            x: 0.,
            y: 0.,
            width: 200.,
            height: 70.,
            radius: 16.,
            background: [140, 165, 255, 255],
            color: [20, 33, 59, 255],
            font_size: 18.,
            text: String::new(),
            disabled: false,
            action: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Kind<'a> {
    Ident(&'a str),
    String(String),
    Number(f32),
    Duration(f32),
    Hex(&'a str),
    Open,
    Close,
    Colon,
    Semi,
    LeftParen,
    RightParen,
    Arrow,
    Eof,
}

#[derive(Debug, Clone)]
struct Token<'a> {
    kind: Kind<'a>,
    offset: usize,
}

struct Lexer<'a> {
    source: &'a str,
    offset: usize,
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }
    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.offset += ch.len_utf8();
        Some(ch)
    }
    /// The next byte without decoding it. Every caller of this decides about ASCII only, so
    /// a byte of `0x80..` is simply "not one of mine"; UTF-8 keeps `*/` and `\n` searchable
    /// byte by byte because continuation bytes never match an ASCII character.
    fn peek_byte(&self) -> Option<u8> {
        self.source.as_bytes().get(self.offset).copied()
    }
    fn starts_with(&self, token: &str) -> bool {
        self.source.as_bytes()[self.offset..].starts_with(token.as_bytes())
    }
    fn skip_bytes(&mut self, mut accepted: impl FnMut(u8) -> bool) {
        while self.peek_byte().is_some_and(&mut accepted) {
            self.offset += 1;
        }
    }
    /// `char::is_whitespace`, answered by byte for the ASCII run and by decoding only where
    /// the document actually holds a non-ASCII space.
    fn skip_whitespace(&mut self) {
        // `is_ascii_whitespace` leaves out the vertical tab, which is whitespace by `char`.
        while self
            .peek_byte()
            .is_some_and(|b| b.is_ascii_whitespace() || b == b'\x0b')
        {
            self.offset += 1;
        }
        while self.peek_byte().is_some_and(|b| b >= 0x80)
            && self.peek().is_some_and(char::is_whitespace)
        {
            self.bump();
        }
    }
    fn error(&self, message: &str) -> String {
        format!("{message} (byte {})", self.offset)
    }

    fn next(&mut self) -> Result<Token<'a>, String> {
        loop {
            self.skip_whitespace();
            if self.starts_with("//") {
                while self.peek_byte().is_some_and(|b| b != b'\n') {
                    self.offset += 1;
                }
            } else if self.starts_with("/*") {
                let start = self.offset;
                self.offset += 2;
                let mut depth = 1;
                while depth > 0 {
                    if self.starts_with("/*") {
                        depth += 1;
                        self.offset += 2;
                    } else if self.starts_with("*/") {
                        depth -= 1;
                        self.offset += 2;
                    } else if self.peek_byte().is_none() {
                        return Err(format!("Unclosed block comment (byte {start})"));
                    } else {
                        self.offset += 1;
                    }
                }
            } else {
                break;
            }
        }
        let offset = self.offset;
        let Some(ch) = self.bump() else {
            return Ok(Token {
                kind: Kind::Eof,
                offset,
            });
        };
        let kind = match ch {
            '{' => Kind::Open,
            '}' => Kind::Close,
            ':' => Kind::Colon,
            ';' => Kind::Semi,
            '(' => Kind::LeftParen,
            ')' => Kind::RightParen,
            '-' if self.starts_with(">") => {
                self.offset += 1;
                Kind::Arrow
            }
            '\'' | '"' => {
                let mut value = String::new();
                loop {
                    match self.bump() {
                        Some(c) if c == ch => break,
                        Some('\\') => {
                            let escaped = match self.bump() {
                                Some('n') => '\n',
                                Some('r') => '\r',
                                Some('t') => '\t',
                                Some('\\') => '\\',
                                Some('\'') => '\'',
                                Some('"') => '"',
                                Some(c) => {
                                    return Err(
                                        self.error(&format!("Unsupported string escape \\{c}"))
                                    );
                                }
                                None => return Err(format!("Unclosed string (byte {offset})")),
                            };
                            value.push(escaped);
                        }
                        Some(c) => value.push(c),
                        None => return Err(format!("Unclosed string (byte {offset})")),
                    }
                }
                Kind::String(value)
            }
            '#' => {
                let start = self.offset;
                self.skip_bytes(|b| b.is_ascii_alphanumeric());
                Kind::Hex(&self.source[start..self.offset])
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                self.skip_bytes(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.');
                Kind::Ident(&self.source[offset..self.offset])
            }
            c if c.is_ascii_digit() || matches!(c, '.' | '-' | '+') => {
                self.skip_bytes(|b| b.is_ascii_digit() || b == b'.');
                if self.peek_byte().is_some_and(|b| b == b'e' || b == b'E') {
                    self.offset += 1;
                    if self.peek_byte().is_some_and(|b| b == b'+' || b == b'-') {
                        self.offset += 1;
                    }
                    self.skip_bytes(|b| b.is_ascii_digit());
                }
                let raw = &self.source[offset..self.offset];
                let value: f32 = raw
                    .parse()
                    .map_err(|_| format!("Invalid number '{raw}' (byte {offset})"))?;
                if !value.is_finite() {
                    return Err(format!("Number must be finite (byte {offset})"));
                }
                if self.starts_with("ms") {
                    self.offset += 2;
                    Kind::Duration(value)
                } else {
                    if self.starts_with("px") {
                        self.offset += 2;
                    }
                    Kind::Number(value)
                }
            }
            c => {
                return Err(format!(
                    "Unsupported character '{c}' (byte {offset}); this vector demo does not support bindings or expressions"
                ));
            }
        };
        Ok(Token { kind, offset })
    }
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    current: Token<'a>,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Result<Self, String> {
        let mut lexer = Lexer { source, offset: 0 };
        let current = lexer.next()?;
        Ok(Self { lexer, current })
    }
    fn advance(&mut self) -> Result<Kind<'a>, String> {
        let next = self.lexer.next()?;
        Ok(std::mem::replace(&mut self.current, next).kind)
    }
    fn error(&self, message: &str) -> String {
        format!("{message} (byte {})", self.current.offset)
    }
    fn expect(&mut self, kind: Kind<'a>, description: &str) -> Result<(), String> {
        if self.current.kind != kind {
            return Err(self.error(&format!(
                "Expected {description}, found {:?}",
                self.current.kind
            )));
        }
        self.advance()?;
        Ok(())
    }
    fn expect_ident(&mut self, name: &str, description: &str) -> Result<(), String> {
        if !matches!(self.current.kind, Kind::Ident(value) if value == name) {
            return Err(self.error(&format!(
                "Expected {description}, found {:?}",
                self.current.kind
            )));
        }
        self.advance()?;
        Ok(())
    }
    fn ident(&mut self) -> Result<&'a str, String> {
        if let Kind::Ident(value) = self.current.kind {
            self.advance()?;
            Ok(value)
        } else {
            Err(self.error("Expected an identifier"))
        }
    }
    fn string(&mut self, property: &str) -> Result<String, String> {
        if let Kind::String(value) = &self.current.kind {
            let value = value.clone();
            self.advance()?;
            Ok(value)
        } else {
            Err(self.error(&format!("'{property}' requires a quoted string; bindings and expressions are not supported in the vector demo")))
        }
    }
    fn number(&mut self, property: &str, zero_allowed: bool) -> Result<f32, String> {
        if let Kind::Number(value) = self.current.kind {
            if value < 0. || (!zero_allowed && value == 0.) {
                return Err(self.error(&format!(
                    "'{property}' must be {}",
                    if zero_allowed {
                        "non-negative"
                    } else {
                        "positive"
                    }
                )));
            }
            self.advance()?;
            Ok(value)
        } else {
            Err(self.error(&format!("'{property}' requires a finite pixel number")))
        }
    }
    fn color(&mut self, property: &str) -> Result<[u8; 4], String> {
        let Kind::Hex(raw) = &self.current.kind else {
            return Err(self.error(&format!(
                "'{property}' requires an unquoted #RGB, #RRGGBB or #RRGGBBAA color"
            )));
        };
        if !matches!(raw.len(), 3 | 6 | 8) || !raw.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(self.error(&format!(
                "Invalid hex color '#{raw}': expected #RGB, #RRGGBB or #RRGGBBAA"
            )));
        }
        let mut color = [0, 0, 0, 255];
        if raw.len() == 3 {
            for (i, c) in raw.chars().enumerate() {
                color[i] = c.to_digit(16).unwrap() as u8 * 17;
            }
        } else {
            for i in 0..raw.len() / 2 {
                color[i] = u8::from_str_radix(&raw[i * 2..i * 2 + 2], 16).unwrap();
            }
        }
        self.advance()?;
        Ok(color)
    }
    fn padding(&mut self) -> Result<[f32; 4], String> {
        let mut values = Vec::new();
        while matches!(self.current.kind, Kind::Number(_)) {
            values.push(self.number("padding", true)?);
            if values.len() > 4 {
                return Err(self.error("'padding' accepts 1, 2 or 4 values"));
            }
        }
        match values.as_slice() {
            [all] => Ok([*all; 4]),
            [vertical, horizontal] => Ok([*vertical, *horizontal, *vertical, *horizontal]),
            [top, right, bottom, left] => Ok([*top, *right, *bottom, *left]),
            _ => Err(self.error("'padding' accepts 1, 2 or 4 non-negative pixel values")),
        }
    }
    fn button(&mut self) -> Result<ButtonSpec, String> {
        self.expect(Kind::Open, "'{' after Button")?;
        let mut button = ButtonSpec::default();
        while self.current.kind != Kind::Close {
            let name = self.ident()?;
            // The declared-property set doubles as the duplicate check: it holds one bit per
            // name the vocabulary knows, so detecting a repeat costs no allocation.
            if !button.specified.try_insert(name) {
                return Err(self.error(&format!("Duplicate Button property '{name}'")));
            }
            if name == "clicked" {
                self.expect(Kind::Arrow, "'->' after clicked")?;
                let action = self.ident()?;
                let method = action.strip_prefix("actions.").unwrap_or("");
                if !valid_identifier(method) {
                    return Err(self.error("'clicked' requires actions.<name>() without arguments"));
                }
                self.expect(Kind::LeftParen, "'(' after action name")?;
                self.expect(Kind::RightParen, "')'; action arguments are not supported")?;
                button.action = Some(action.to_owned());
            } else {
                if !matches!(
                    name,
                    "key"
                        | "x"
                        | "y"
                        | "width"
                        | "height"
                        | "radius"
                        | "background"
                        | "color"
                        | "font.size"
                        | "fontSize"
                        | "hoverBackground"
                        | "pressedBackground"
                        | "disabledBackground"
                        | "borderColor"
                        | "focusBorderColor"
                        | "borderWidth"
                        | "transitionDuration"
                        | "text"
                        | "disabled"
                ) {
                    return Err(
                        self.error(&format!("Unsupported Button property or child '{name}'"))
                    );
                }
                self.expect(
                    Kind::Colon,
                    "':' after property name; bindings are not supported",
                )?;
                match name {
                    "key" => {
                        button.key = self.string("key")?;
                        if button.key.is_empty() {
                            return Err(self.error("Explicit 'key' must not be empty"));
                        }
                    }
                    "x" => button.x = self.number("x", true)?,
                    "y" => button.y = self.number("y", true)?,
                    "width" => button.width = self.number("width", false)?,
                    "height" => button.height = self.number("height", false)?,
                    "radius" => button.radius = self.number("radius", true)?,
                    "background" => button.background = self.color("background")?,
                    "color" => button.color = self.color("color")?,
                    "font.size" | "fontSize" => {
                        if button.specified.contains("font.size")
                            && button.specified.contains("fontSize")
                        {
                            return Err(
                                self.error("fontSize and font.size cannot both be specified")
                            );
                        }
                        button.font_size = self.number("fontSize", false)?;
                    }
                    "hoverBackground" | "pressedBackground" | "disabledBackground"
                    | "borderColor" | "focusBorderColor" => {
                        let color = self.color(name)?;
                        button.colors.set(name, color);
                    }
                    "borderWidth" => {
                        let n = self.number(name, true)?;
                        button.numbers.set(name, n);
                    }
                    "transitionDuration" => {
                        let Kind::Duration(n) = self.current.kind else {
                            return Err(self.error("transitionDuration requires ms"));
                        };
                        if !(0. ..=2000.).contains(&n) {
                            return Err(self.error("transitionDuration must be 0..2000ms"));
                        }
                        self.advance()?;
                        button.numbers.set(name, n);
                    }
                    "text" => button.text = self.string("text")?,
                    "disabled" => {
                        button.disabled = match &self.current.kind {
                            Kind::Ident(value) if *value == "true" => true,
                            Kind::Ident(value) if *value == "false" => false,
                            _ => return Err(self.error("'disabled' requires true or false")),
                        };
                        self.advance()?;
                    }
                    _ => unreachable!(),
                }
            }
            self.expect(Kind::Semi, "';' after Button property")?;
        }
        self.expect(Kind::Close, "'}' after Button")?;
        button.specified.canonicalize();
        // Like CSS rounded corners, a radius may be larger than half the box.
        button.radius = button.radius.min(button.width.min(button.height) / 2.);
        Ok(button)
    }
    fn scene(&mut self) -> Result<Scene, String> {
        self.expect_ident("component", "'component'")?;
        let name = self.ident()?;
        if !valid_identifier(name) {
            return Err(self.error("Invalid component name"));
        }
        self.expect(Kind::Open, "'{' after component name")?;
        self.expect_ident("Frame", "exactly one root Frame")?;
        self.expect(Kind::Open, "'{' after Frame")?;
        let mut width = 360.;
        let mut height = 220.;
        let mut background = [24, 30, 42, 255];
        let mut padding = [0.; 4];
        let mut overflow = "visible";
        let mut clip = false;
        let mut radius = 0.;
        let mut scroll = false;
        let mut buttons = Vec::new();
        let mut gap = 0.;
        let mut seen: HashSet<&'a str> = HashSet::new();
        let mut modals: Vec<ModalSpec> = Vec::new();
        while self.current.kind != Kind::Close {
            let property = self.ident()?;
            if property == "Modal" {
                let start = buttons.len();
                let mut modal = self.modal(&mut buttons)?;
                modal.controls = start..buttons.len();
                if !modal.key.is_empty() && modals.iter().any(|m| m.key == modal.key) {
                    return Err(self.error("Duplicate Modal key"));
                }
                modals.push(modal);
                continue;
            }
            if property == "Button" || property == "Scroll" {
                if !modals.is_empty() {
                    return Err(self.error("Modal layers must follow the base controls"));
                }
                if property == "Scroll" {
                    if scroll || !buttons.is_empty() {
                        return Err(self.error("Scroll must be the only Frame child"));
                    }
                    scroll = true;
                    self.expect(Kind::Open, "'{' after Scroll")?;
                    while self.current.kind != Kind::Close {
                        self.expect_ident("Button", "Button inside Scroll")?;
                        buttons.push(self.button()?);
                    }
                    self.expect(Kind::Close, "'}' after Scroll")?;
                } else {
                    if scroll {
                        return Err(self.error("Scroll must be the only Frame child"));
                    }
                    buttons.push(self.button()?);
                }
                continue;
            }
            if !matches!(
                property,
                "gap"
                    | "width"
                    | "height"
                    | "padding"
                    | "background"
                    | "overflow"
                    | "clip"
                    | "radius"
            ) {
                return Err(
                    self.error(&format!("Unsupported Frame property or child '{property}'"))
                );
            }
            if !seen.insert(property) {
                return Err(self.error(&format!("Duplicate Frame property '{property}'")));
            }
            self.expect(Kind::Colon, "':' after Frame property")?;
            match property {
                "gap" => gap = self.number("gap", true)?,
                "width" => width = self.number("width", false)?,
                "height" => height = self.number("height", false)?,
                "padding" => padding = self.padding()?,
                "background" => background = self.color("background")?,
                "radius" => radius = self.number("radius", true)?,
                "clip" => {
                    clip = match self.ident()? {
                        "true" => true,
                        "false" => false,
                        _ => return Err(self.error("clip requires true or false")),
                    };
                }
                "overflow" => {
                    overflow = self.ident()?;
                    if !matches!(overflow, "visible" | "hidden") {
                        return Err(self.error("overflow: expected visible or hidden; use a Scroll element for scrolling"));
                    }
                }
                _ => unreachable!(),
            }
            self.expect(Kind::Semi, "';' after Frame property")?;
        }
        self.expect(Kind::Close, "'}' after Frame")?;
        self.expect(Kind::Close, "'}' after component")?;
        if self.current.kind != Kind::Eof {
            return Err(
                self.error("Unexpected content after component; only one component is supported")
            );
        }
        if buttons.len() > 256 {
            return Err(self.error("At most 256 controls per scene"));
        }
        if seen.contains("clip") && seen.contains("overflow") {
            return Err(self.error("Use clip or legacy overflow, not both"));
        }
        if overflow == "hidden" {
            clip = true;
        }
        let mut keys = HashSet::new();
        let mut y = padding[0];
        let mut used_width = width;
        let mut used_height = height;
        let base = modals.first().map_or(buttons.len(), |m| m.controls.start);
        for button in &mut buttons[..base] {
            if !button.key.is_empty() && !keys.insert(button.key.clone()) {
                return Err(self.error("Duplicate control key"));
            }
            if !button.specified.contains("x") {
                button.x = padding[3];
            }
            if !button.specified.contains("y") {
                button.y = y;
            }
            y = button.y + button.height + gap;
            used_width = used_width.max(button.x + button.width + padding[1]);
            used_height = used_height.max(button.y + button.height + padding[2]);
        }
        // Modal controls are positioned relative to their surface and never
        // extend the scrollable content; overlays do not change scene size.
        for modal in &mut modals {
            if modal.width <= 0. {
                modal.width = width;
            }
            if modal.height <= 0. {
                modal.height = height;
            }
            if !modal.width.is_finite() || !modal.height.is_finite() {
                return Err(self.error("Modal size must be finite"));
            }
            let mut y = modal.y;
            for button in &mut buttons[modal.controls.clone()] {
                if !button.key.is_empty() && !keys.insert(button.key.clone()) {
                    return Err(self.error("Duplicate control key"));
                }
                if button.specified.contains("x") {
                    button.x += modal.x;
                } else {
                    button.x = modal.x;
                }
                if button.specified.contains("y") {
                    button.y += modal.y;
                } else {
                    button.y = y;
                }
                y = button.y + button.height + gap;
            }
        }
        let button = buttons.first().cloned().unwrap_or_default();
        Ok(Scene {
            name: name.to_owned(),
            width,
            height,
            background,
            overflow: overflow.to_owned(),
            clip,
            radius: radius.min(width.min(height) / 2.),
            scroll,
            padding,
            content_width: used_width.min(f32::MAX).max(width),
            content_height: used_height.min(f32::MAX).max(height),
            button,
            buttons,
            gap,
            modals,
        })
    }
    /// `Modal { key; x; y; width; height; backdrop; dismiss; open; dismissed -> actions.name(); Button {…} }`.
    /// Child coordinates are relative to the modal surface. A size of zero
    /// means the whole frame, so outside-click can never dismiss such a layer.
    fn modal(&mut self, buttons: &mut Vec<ButtonSpec>) -> Result<ModalSpec, String> {
        self.expect(Kind::Open, "'{' after Modal")?;
        let mut modal = ModalSpec {
            key: String::new(),
            x: 0.,
            y: 0.,
            width: 0.,
            height: 0.,
            backdrop: [0, 0, 0, 128],
            dismiss: true,
            open: true,
            action: None,
            controls: 0..0,
        };
        let mut seen: HashSet<&'a str> = HashSet::new();
        while self.current.kind != Kind::Close {
            let name = self.ident()?;
            if name == "Button" {
                buttons.push(self.button()?);
                if buttons.len() > 256 {
                    return Err(self.error("At most 256 controls per scene"));
                }
                continue;
            }
            if !seen.insert(name) {
                return Err(self.error(&format!("Duplicate Modal property '{name}'")));
            }
            if name == "dismissed" {
                self.expect(Kind::Arrow, "'->' after dismissed")?;
                let action = self.ident()?;
                if !valid_identifier(action.strip_prefix("actions.").unwrap_or("")) {
                    return Err(
                        self.error("'dismissed' requires actions.<name>() without arguments")
                    );
                }
                self.expect(Kind::LeftParen, "'(' after action name")?;
                self.expect(Kind::RightParen, "')'; action arguments are not supported")?;
                self.expect(Kind::Semi, "';' after Modal handler")?;
                modal.action = Some(action.to_owned());
                continue;
            }
            if !matches!(
                name,
                "key" | "x" | "y" | "width" | "height" | "backdrop" | "dismiss" | "open"
            ) {
                return Err(self.error(&format!("Unsupported Modal property or child '{name}'")));
            }
            self.expect(Kind::Colon, "':' after Modal property")?;
            match name {
                "key" => {
                    modal.key = self.string("key")?;
                    if modal.key.is_empty() {
                        return Err(self.error("Explicit 'key' must not be empty"));
                    }
                }
                "x" => modal.x = self.number("x", true)?,
                "y" => modal.y = self.number("y", true)?,
                "width" => modal.width = self.number("width", false)?,
                "height" => modal.height = self.number("height", false)?,
                "backdrop" => modal.backdrop = self.color("backdrop")?,
                "dismiss" | "open" => {
                    let value = match self.ident()? {
                        "true" => true,
                        "false" => false,
                        _ => return Err(self.error(&format!("'{name}' requires true or false"))),
                    };
                    if name == "dismiss" {
                        modal.dismiss = value;
                    } else {
                        modal.open = value;
                    }
                }
                _ => unreachable!(),
            }
            self.expect(Kind::Semi, "';' after Modal property")?;
        }
        self.expect(Kind::Close, "'}' after Modal")?;
        Ok(modal)
    }
}

fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn parse(source: &str) -> Result<Scene, String> {
    Parser::new(source)?.scene()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO: &str = "component SearchWindow { Frame { width: 360; height: 220; padding: 65 80; background: #181e2a; Button { key: 'run'; width: 200; height: 70; radius: 16; background: #8ca5ff; color: #14213b; font.size: 18; text: 'Найти'; disabled: false; clicked -> actions.search(); } } }";
    fn with_button(props: &str) -> Result<Scene, String> {
        parse(&format!(
            "component Demo {{ Frame {{ Button {{ {props} }} }} }}"
        ))
    }

    #[test]
    fn parses_the_actual_forma_button_and_layout() {
        let scene = parse(DEMO).unwrap();
        assert_eq!(scene.name, "SearchWindow");
        assert_eq!((scene.width, scene.height), (360., 220.));
        assert_eq!(scene.background, [24, 30, 42, 255]);
        assert_eq!((scene.button.x, scene.button.y), (80., 65.));
        assert_eq!((scene.button.width, scene.button.height), (200., 70.));
        assert_eq!(scene.button.radius, 16.);
        assert_eq!(scene.button.font_size, 18.);
        assert_eq!(scene.button.key, "run");
        assert_eq!(scene.button.text, "Найти");
        assert_eq!(scene.button.color, [20, 33, 59, 255]);
        assert_eq!(scene.button.background, [140, 165, 255, 255]);
        assert_eq!(scene.button.action.as_deref(), Some("actions.search"));
        assert!(!scene.button.disabled);
    }

    #[test]
    fn defaults_are_explicit_and_deterministic() {
        let scene = with_button("").unwrap();
        assert_eq!(scene.button, ButtonSpec::default());
        assert_eq!((scene.width, scene.height), (360., 220.));
    }

    #[test]
    fn the_declared_property_set_doubles_as_the_duplicate_check() {
        let scene = with_button("key: 'go'; font.size: 20; clicked -> actions.run();").unwrap();
        assert!(scene.button.specified.contains("fontSize"));
        assert!(scene.button.specified.contains("clicked"));
        // The alias spelling folds into the name a component template asks about.
        assert!(!scene.button.specified.contains("font.size"));
        assert!(
            with_button("width: 10; width: 20;")
                .unwrap_err()
                .contains("Duplicate Button property 'width'")
        );
        assert!(
            with_button("fontSize: 12; font.size: 14;")
                .unwrap_err()
                .contains("cannot both be specified")
        );
    }

    #[test]
    fn comments_quotes_and_unicode_are_preserved() {
        let scene = parse("// hello\ncomponent Demo /* nested /* comment */ ok */ { Frame { Button { text: \"Найти 'текст' \\\"ещё\\\"\\n\\t\\\\\"; } } } // end").unwrap();
        assert_eq!(scene.button.text, "Найти 'текст' \"ещё\"\n\t\\");
        assert_eq!(
            with_button("text: 'it\\'s // not a comment';")
                .unwrap()
                .button
                .text,
            "it's // not a comment"
        );
    }

    #[test]
    fn css_hex_colors_include_alpha_and_uppercase() {
        assert_eq!(
            with_button("color: #AbC;").unwrap().button.color,
            [170, 187, 204, 255]
        );
        assert_eq!(
            with_button("background: #01020304;")
                .unwrap()
                .button
                .background,
            [1, 2, 3, 4]
        );
        for value in [
            "#abcd",
            "#12",
            "#123456789",
            "#xyzxyz",
            "'#ffffff'",
            "rgb(1,2,3)",
        ] {
            assert!(
                with_button(&format!("color: {value};")).is_err(),
                "accepted {value}"
            );
        }
    }

    #[test]
    fn padding_uses_top_right_bottom_left() {
        for (value, position) in [
            ("10", (10., 10.)),
            ("10px 20px", (20., 10.)),
            ("10 20 30 40", (40., 10.)),
        ] {
            let scene = parse(&format!(
                "component Demo {{ Frame {{ padding: {value}; Button {{ }} }} }}"
            ))
            .unwrap();
            assert_eq!((scene.button.x, scene.button.y), position);
        }
        for value in ["", "1 2 3", "1 2 3 4 5", "-1", "1, 2", "auto"] {
            assert!(
                parse(&format!(
                    "component Demo {{ Frame {{ padding: {value}; Button {{ }} }} }}"
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn accepts_fractional_and_px_sizes_and_clamps_rounding() {
        let scene =
            with_button("width: 20.5px; height: 1e1; radius: 100; font.size: .5; disabled: true;")
                .unwrap();
        assert_eq!(
            (
                scene.button.width,
                scene.button.height,
                scene.button.radius,
                scene.button.font_size
            ),
            (20.5, 10., 5., 0.5)
        );
        assert!(scene.button.disabled);
    }

    #[test]
    fn rejects_nonpositive_or_nonfinite_sizes() {
        for value in [
            "0", "-1", "1e999", "NaN", "Infinity", "10%", "*", "1.2.3", "1em",
        ] {
            assert!(
                with_button(&format!("width: {value};")).is_err(),
                "accepted {value}"
            );
        }
        assert!(with_button("radius: -1;").is_err());
        assert!(with_button("font.size: 0;").is_err());
        assert!(with_button("radius: 0;").is_ok());
    }

    #[test]
    fn rejects_invalid_frame_sizes() {
        for source in [
            "component X { Frame { width: 0; Button {} } }",
            "component X { Frame { height: -1; Button {} } }",
        ] {
            assert!(parse(source).is_err(), "accepted {source}");
        }
    }

    #[test]
    fn rejects_duplicates_unsupported_properties_and_bindings() {
        for props in [
            "text:'a'; text:'b';",
            "key:'';",
            "row: 1;",
            "font.weight: 700;",
            "text: state.label;",
            "text <-> state.label;",
            "Text {}",
            "disabled: 1;",
            "clicked -> actions.a(); clicked -> actions.b();",
        ] {
            assert!(with_button(props).is_err(), "accepted {props}");
        }
        for contents in [
            "padding:0; padding:1; Button{}",
            "width:360; width:360; Button{}",
            "rows:[*]; Button{}",
            "Frame{}",
            "Text{}",
        ] {
            assert!(
                parse(&format!("component X {{ Frame {{ {contents} }} }}")).is_err(),
                "accepted {contents}"
            );
        }
    }

    #[test]
    fn actions_must_be_explicit_and_have_no_arguments() {
        for action in [
            "search()",
            "actions.search(1)",
            "actions.search",
            "state.search()",
            "actions.nested.search()",
            "actions.()",
        ] {
            assert!(
                with_button(&format!("clicked -> {action};")).is_err(),
                "accepted {action}"
            );
        }
        assert_eq!(
            with_button("clicked -> actions._run2();")
                .unwrap()
                .button
                .action
                .as_deref(),
            Some("actions._run2")
        );
    }

    #[test]
    fn modal_layers_follow_base_controls_and_position_children_relative_to_the_surface() {
        let scene = parse("component X { Frame { width: 300; height: 200; padding: 10; gap: 5; Button { key: 'base'; height: 20; } Modal { key: 'ask'; x: 40; y: 30; width: 200; height: 120; backdrop: #00000080; dismissed -> actions.cancel(); Button { key: 'a'; width: 50; height: 20; } Button { key: 'b'; x: 10; y: 60; width: 50; height: 20; } } Modal { open: false; dismiss: false; Button { key: 'c'; height: 10; } } } }").unwrap();
        assert_eq!(scene.buttons.len(), 4);
        assert_eq!(scene.modals.len(), 2);
        let ask = &scene.modals[0];
        assert_eq!(
            (ask.key.as_str(), ask.x, ask.y, ask.width, ask.height),
            ("ask", 40., 30., 200., 120.)
        );
        assert_eq!(ask.backdrop, [0, 0, 0, 128]);
        assert!(ask.dismiss && ask.open);
        assert_eq!(ask.action.as_deref(), Some("actions.cancel"));
        assert_eq!(ask.controls, 1..3);
        assert_eq!((scene.buttons[1].x, scene.buttons[1].y), (40., 30.));
        assert_eq!((scene.buttons[2].x, scene.buttons[2].y), (50., 90.));
        let second = &scene.modals[1];
        assert_eq!((second.width, second.height), (300., 200.));
        assert!(!second.open && !second.dismiss);
        assert_eq!(second.controls, 3..4);
        assert_eq!((scene.buttons[3].x, scene.buttons[3].y), (0., 0.));
        // Overlays never extend scrollable content.
        assert_eq!((scene.content_width, scene.content_height), (300., 200.));
        for source in [
            "component X { Frame { Modal { Button {} } Button {} } }",
            "component X { Frame { Modal { key: 'a'; } Modal { key: 'a'; } } }",
            "component X { Frame { Modal { Button { key: 'a'; } } Modal { Button { key: 'a'; } } } }",
            "component X { Frame { Modal { dismiss: 1; } } }",
            "component X { Frame { Modal { dismissed -> cancel(); } } }",
            "component X { Frame { Modal { Scroll {} } } }",
            "component X { Frame { Modal { key: ''; } } }",
        ] {
            assert!(parse(source).is_err(), "accepted {source}");
        }
    }

    #[test]
    fn rejects_incomplete_documents_and_trailing_content() {
        for source in [
            "",
            "component X {}",
            "component X { Frame { Button {} }",
            "component X { Frame { Button {} } } Button {}",
            "component X { Frame { Button {} } Frame {} }",
            "component X { Frame { Button { text: 'x' } } }",
            "component X { Frame { Button { text: 'unterminated; } } }",
            "/* unclosed",
            "component X { Frame { Button{} } } /* unclosed",
        ] {
            let error = parse(source).unwrap_err();
            assert!(error.contains("byte"), "missing location: {error}");
        }
    }
}
