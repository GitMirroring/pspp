use std::{
    borrow::Cow,
    fmt::{Display, Write},
    mem::discriminant,
    ops::Not,
};

use itertools::Itertools;

use crate::{
    output::pivot::look::{FontStyle, HorzAlign},
    spv::read::html::Style,
};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token<'a> {
    Id(Cow<'a, str>),
    LeftCurly,
    RightCurly,
    Colon,
    Semicolon,
}

struct Lexer<'a>(&'a str);

impl<'a> Iterator for Lexer<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut s = self.0;
        loop {
            s = s.trim_start();
            if let Some(rest) = s.strip_prefix("<!--") {
                s = rest;
            } else if let Some(rest) = s.strip_prefix("-->") {
                s = rest;
            } else {
                break;
            }
        }
        let mut iter = s.chars();
        let (c, mut rest) = (iter.next()?, iter.as_str());
        let (token, rest) = match c {
            '{' => (Token::LeftCurly, rest),
            '}' => (Token::RightCurly, rest),
            ':' => (Token::Colon, rest),
            ';' => (Token::Semicolon, rest),
            '\'' | '"' => {
                let quote = c;
                let mut s = String::new();
                while let Some(c) = iter.next() {
                    if c == quote {
                        break;
                    } else if c != '\\' {
                        s.push(c);
                    } else {
                        let start = iter.as_str();
                        match iter.next() {
                            None => break,
                            Some(a) if a.is_ascii_alphanumeric() => {
                                let n = start
                                    .chars()
                                    .take_while(|c| c.is_ascii_alphanumeric())
                                    .take(6)
                                    .count();
                                iter = start[n..].chars();
                                if let Ok(code_point) = u32::from_str_radix(&start[..n], 16)
                                    && let Ok(c) = char::try_from(code_point)
                                {
                                    s.push(c);
                                }
                            }
                            Some('\n') => (),
                            Some(other) => s.push(other),
                        }
                    }
                }
                (Token::Id(Cow::from(s)), iter.as_str())
            }
            _ => {
                while !iter.as_str().starts_with("-->")
                    && let Some(c) = iter.next()
                    && !c.is_whitespace()
                    && c != '{'
                    && c != '}'
                    && c != ':'
                    && c != ';'
                {
                    rest = iter.as_str();
                }
                let id_len = s.len() - rest.len();
                let (id, rest) = s.split_at(id_len);
                (Token::Id(Cow::from(id)), rest)
            }
        };
        self.0 = rest;
        Some(token)
    }
}

impl HorzAlign {
    /// Parses `s` as CSS and returns the value of `text-align` found in it, if
    /// any.
    ///
    /// This is only good enough to handle the simple CSS found in SPV files.
    pub fn from_css(css: &str) -> Option<Self> {
        let mut lexer = Lexer(css);
        while let Some(token) = lexer.next() {
            if let Token::Id(key) = token
                && let Some(Token::Colon) = lexer.next()
                && let Some(Token::Id(value)) = lexer.next()
                && key.as_ref() == "text-align"
                && let Ok(align) = value.parse()
            {
                return Some(align);
            }
        }
        None
    }
}

impl Style {
    /// Parses the CSS found in `css` and returns the corresponding [Style]s.
    ///
    /// This is only good enough to parse the simple CSS found in SPV files.
    pub fn parse_css(css: &str) -> Vec<Style> {
        let mut lexer = Lexer(css);
        let mut styles = Vec::new();
        while let Some(token) = lexer.next() {
            if let Token::Id(key) = token
                && let Some(Token::Colon) = lexer.next()
                && let Some(Token::Id(value)) = lexer.next()
                && let Some((style, add)) = match key.as_ref() {
                    "color" => value.parse().ok().map(|color| (Style::Color(color), true)),
                    "font-weight" => Some((Style::Bold, value == "bold")),
                    "font-style" => Some((Style::Italic, value == "italic")),
                    "text-decoration" => Some((Style::Underline, value == "underline")),
                    "font-family" => Some((Style::Face(value.into()), true)),
                    "font-size" => value
                        .strip_suffix("pt")
                        .unwrap_or(&value)
                        .parse::<i32>()
                        .ok()
                        .map(|size| (Style::Size(size as f64 * 0.75), true)),
                    _ => None,
                }
            {
                // Remove from `styles` any style of the same kind as `style`.
                styles.retain(|s| discriminant(s) != discriminant(&style));
                if add {
                    styles.push(style);
                }
            }
        }
        styles
    }
}

impl FontStyle {
    /// Parses the CSS found in `css` and modifies this `FontStyle` accordingly.
    ///
    /// This is only good enough to parse the simple CSS found in SPV files.
    pub fn parse_css(&mut self, css: &str) {
        let mut lexer = Lexer(css);
        while let Some(token) = lexer.next() {
            if let Token::Id(key) = token
                && let Some(Token::Colon) = lexer.next()
                && let Some(Token::Id(value)) = lexer.next()
            {
                match key.as_ref() {
                    "color" => {
                        if let Ok(color) = value.parse() {
                            self.fg = color;
                        }
                    }
                    "font-weight" => self.bold = value == "bold",
                    "font-style" => self.italic = value == "italic",
                    "text-decoration" => self.underline = value == "underline",
                    "font-family" => self.font = value.into(),
                    "font-size" => {
                        if let Ok(size) = value.parse::<i32>() {
                            self.size = (size as i64 * 3 / 4) as i32;
                        }
                    }
                    _ => (),
                }
            }
        }
    }

    /// Parses the CSS found in `css` and returns a corresponding `FontStyle`
    /// (starting from [FontStyle::default]).
    ///
    /// This is only good enough to parse the simple CSS found in SPV files.
    pub fn from_css(css: &str) -> Self {
        let mut style = FontStyle::default();
        style.parse_css(css);
        style
    }

    /// Returns CSS for the differences from `base` to this font style.  Returns
    /// `None` if there are no differences.
    pub fn to_css(&self, base: &FontStyle) -> Option<String> {
        let mut settings = Vec::new();
        if self.font != base.font {
            if is_css_ident(&self.font) {
                settings.push(format!("font-family: {}", &self.font));
            } else {
                settings.push(format!("font-family: {}", CssString(&self.font)));
            }
        }
        if self.bold != base.bold {
            settings.push(format!(
                "font-weight: {}",
                if self.bold { "bold" } else { "normal" }
            ));
        }
        if self.italic != base.italic {
            settings.push(format!(
                "font-style: {}",
                if self.bold { "italic" } else { "normal" }
            ));
        }
        if self.underline != base.underline {
            settings.push(format!(
                "text-decoration: {}",
                if self.bold { "underline" } else { "none" }
            ));
        }
        if self.size != base.size {
            settings.push(format!("font-size: {}", self.size as i64 * 4 / 3));
        }
        if self.fg != base.fg {
            settings.push(format!("color: {}", self.fg.display_css()));
        }
        settings
            .is_empty()
            .not()
            .then(|| format!("<!-- p {{ {} }} -->", settings.into_iter().join("; ")))
    }
}

fn is_css_ident(s: &str) -> bool {
    fn is_nmstart(c: char) -> bool {
        c.is_ascii_alphabetic() || c == '_'
    }
    s.chars().next().is_some_and(is_nmstart) && s.chars().all(|c| is_nmstart(c) || c as u32 > 159)
}

struct CssString<'a>(&'a str);

impl<'a> Display for CssString<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let quote = if self.0.contains('"') && !self.0.contains('\'') {
            '\''
        } else {
            '"'
        };
        f.write_char(quote)?;
        for c in self.0.chars() {
            match c {
                _ if c == quote || c == '\\' => {
                    f.write_char('\\')?;
                    f.write_char(c)?;
                }
                '\n' => f.write_str("\\00000a")?,
                c => f.write_char(c)?,
            }
        }
        f.write_char(quote)
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use crate::{
        output::pivot::look::{Color, FontStyle, HorzAlign},
        spv::read::css::{Lexer, Token},
    };

    #[test]
    fn css_horz_align() {
        assert_eq!(
            HorzAlign::from_css("text-align: left"),
            Some(HorzAlign::Left)
        );
        assert_eq!(
            HorzAlign::from_css("margin-top: 0; text-align:center"),
            Some(HorzAlign::Center)
        );
        assert_eq!(
            HorzAlign::from_css("text-align: Right; margin-top:0"),
            Some(HorzAlign::Right)
        );
        assert_eq!(HorzAlign::from_css("text-align: other"), None);
        assert_eq!(HorzAlign::from_css("margin-top: 0"), None);
    }

    #[test]
    fn css_strings() {
        #[track_caller]
        fn test_string(css: &str, value: &str) {
            let mut lexer = Lexer(css);
            assert_eq!(lexer.next(), Some(Token::Id(Cow::from(value))));
            assert_eq!(lexer.next(), None);
        }

        test_string(r#""abc""#, "abc");
        test_string(r#""a\"'\'bc""#, "a\"''bc");
        test_string(r#""a\22 bc""#, "a\" bc");
        test_string(r#""a\000022bc""#, "a\"bc");
        test_string(r#""a'bc""#, "a'bc");
        test_string(
            r#""\\\
xyzzy""#,
            "\\xyzzy",
        );

        test_string(r#"'abc'"#, "abc");
        test_string(r#"'a"\"\'bc'"#, "a\"\"'bc");
        test_string(r#"'a\22 bc'"#, "a\" bc");
        test_string(r#"'a\000022bc'"#, "a\"bc");
        test_string(r#"'a\'bc'"#, "a'bc");
        test_string(
            r#"'a\'bc\
xyz'"#,
            "a'bcxyz",
        );
        test_string(r#"'\\'"#, "\\");
    }

    #[test]
    fn style_from_css() {
        assert_eq!(FontStyle::from_css(""), FontStyle::default());
        assert_eq!(
            FontStyle::from_css(r#"p{color:ff0000}"#),
            FontStyle::default().with_fg(Color::RED)
        );
        assert_eq!(
            FontStyle::from_css("p {font-weight: bold; text-decoration: underline}"),
            FontStyle::default().with_bold(true).with_underline(true)
        );
        assert_eq!(
            FontStyle::from_css("p {font-family: Monospace}"),
            FontStyle::default().with_font("Monospace")
        );
        assert_eq!(
            FontStyle::from_css("p {font-size: 24}"),
            FontStyle::default().with_size(18)
        );
        assert_eq!(
            FontStyle::from_css(
                "<!--color: red; font-weight: bold; font-style: italic; text-decoration: underline; font-family: Serif-->"
            ),
            FontStyle::default()
                .with_fg(Color::RED)
                .with_bold(true)
                .with_italic(true)
                .with_underline(true)
                .with_font("Serif")
        );
    }

    #[test]
    fn style_to_css() {
        let base = FontStyle::default();
        assert_eq!(base.to_css(&base), None);
        assert_eq!(
            FontStyle::default().with_size(18).to_css(&base),
            Some("<!-- p { font-size: 24 } -->".into())
        );
        assert_eq!(
            FontStyle::default()
                .with_bold(true)
                .with_underline(true)
                .to_css(&base),
            Some("<!-- p { font-weight: bold; text-decoration: underline } -->".into())
        );
        assert_eq!(
            FontStyle::default().with_fg(Color::RED).to_css(&base),
            Some("<!-- p { color: #ff0000 } -->".into())
        );
        assert_eq!(
            FontStyle::default().with_font("Monospace").to_css(&base),
            Some("<!-- p { font-family: Monospace } -->".into())
        );
        assert_eq!(
            FontStyle::default()
                .with_font("Times New Roman")
                .to_css(&base),
            Some(r#"<!-- p { font-family: "Times New Roman" } -->"#.into())
        );
    }
}
