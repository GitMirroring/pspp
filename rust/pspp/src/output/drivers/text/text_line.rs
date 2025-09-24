// PSPP - a program for statistical analysis.
// Copyright (C) 2025 Free Software Foundation, Inc.
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE.  See the GNU General Public License for more
// details.
//
// You should have received a copy of the GNU General Public License along with
// this program.  If not, see <http://www.gnu.org/licenses/>.

use nodit::{Interval, NoditMap, interval::ie};
use smallstr::SmallString;
use std::{
    cmp::Ordering,
    fmt::{Debug, Display, Write as _},
    ops::Range,
};

use unicode_width::UnicodeWidthChar;

use crate::output::pivot::look::{Color, FontStyle};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Default for Attribute {
    fn default() -> Self {
        Self {
            fg: Color::BLACK,
            bg: Color::WHITE,
            bold: false,
            italic: false,
            underline: false,
        }
    }
}

impl Attribute {
    pub fn affixes(&self) -> (&'static str, &'static str) {
        match (self.bold, self.italic, self.underline) {
            (false, false, false) => ("", ""),
            (false, false, true) => ("_", "_"),
            (false, true, false) => ("/", "/"),
            (false, true, true) => ("_/", "/_"),
            (true, false, false) => ("*", "*"),
            (true, true, false) => ("*/", "/*"),
            (true, false, true) => ("_*", "*_"),
            (true, true, true) => ("_/*", "*/_"),
        }
    }

    pub fn overstrike<'a>(&self, content: &'a str) -> Overstrike<'a> {
        Overstrike {
            bold: self.bold,
            underline: self.underline,
            content,
        }
    }

    pub fn sgr<T>(&self, content: T) -> Sgr<'_, T> {
        Sgr {
            attribute: self,
            content,
        }
    }

    pub fn for_style(font_style: &FontStyle) -> Self {
        Self {
            fg: font_style.fg,
            bg: font_style.bg,
            bold: font_style.bold,
            italic: font_style.italic,
            underline: font_style.underline,
        }
    }
}

pub struct Overstrike<'a> {
    bold: bool,
    underline: bool,
    content: &'a str,
}

impl<'a> Display for Overstrike<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.bold && !self.underline {
            return f.write_str(self.content);
        }
        for c in self.content.chars() {
            f.write_char(c)?;
            if let Some(width) = c.width() {
                if self.bold {
                    for _ in 0..width {
                        f.write_char('\x08')?;
                    }
                    f.write_char(c)?;
                }
                if self.underline {
                    for _ in 0..width {
                        f.write_char('\x08')?;
                    }
                    for _ in 0..width {
                        f.write_char('_')?;
                    }
                }
            }
        }
        Ok(())
    }
}

pub struct Sgr<'a, T> {
    attribute: &'a Attribute,
    content: T,
}

impl<'a, T> Display for Sgr<'a, T>
where
    T: Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = SmallString::<[u8; 32]>::new();

        let (r, g, b) = self.attribute.fg.into_rgb();
        write!(&mut s, "38;2;{r};{g};{b};").unwrap();

        let (r, g, b) = self.attribute.bg.into_rgb();
        write!(&mut s, "48;2;{r};{g};{b};").unwrap();
        if self.attribute.bold {
            write!(&mut s, "1;").unwrap();
        }
        if self.attribute.italic {
            write!(&mut s, "3;").unwrap();
        }
        if self.attribute.underline {
            write!(&mut s, "4;").unwrap();
        }
        s.pop();
        write!(f, "\x1b[{s}m{}\x1b[0m", &self.content)
    }
}

/// A line of text, encoded in UTF-8, with support functions that properly
/// handle double-width characters and backspaces.
///
/// Designed to make appending text fast, and access and modification of other
/// column positions possible.
#[derive(Clone, Default, Debug)]
pub struct TextLine {
    /// Content.
    string: String,

    /// Attributes.
    attributes: NoditMap<usize, Interval<usize>, Attribute>,

    /// Display width, in character positions.
    width: usize,
}

impl TextLine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.string.clear();
        self.width = 0;
    }

    /// Changes the width of this line to `x` columns.  If `x` is longer than
    /// the current width, extends the line with spaces.  If `x` is shorter than
    /// the current width, removes trailing characters.
    pub fn resize(&mut self, x: usize) {
        match x.cmp(&self.width) {
            Ordering::Greater => self.string.extend((self.width..x).map(|_| ' ')),
            Ordering::Less => {
                let pos = self.find_pos(x);
                self.string.truncate(pos.offsets.start);
                if x > pos.offsets.start {
                    self.string.extend((pos.offsets.start..x).map(|_| '?'));
                }
            }
            Ordering::Equal => return,
        }
        self.width = x;
    }

    fn put_closure<F>(&mut self, x0: usize, w: usize, push_str: F, attribute: Option<Attribute>)
    where
        F: FnOnce(&mut String),
    {
        let x1 = x0 + w;
        if w == 0 {
            // Nothing to do.
        } else if x0 >= self.width {
            // The common case: adding new characters at the end of a line.
            self.string.extend((self.width..x0).map(|_| ' '));
            push_str(&mut self.string);
            self.width = x1;
        } else if x1 >= self.width {
            let p0 = self.find_pos(x0);

            // If a double-width character occupies both `x0 - 1` and `x0`, then
            // replace its first character width by `?`.
            self.string.truncate(p0.offsets.start);
            self.string.extend((p0.columns.start..x0).map(|_| '?'));
            push_str(&mut self.string);
            self.width = x1;
        } else {
            let span = self.find_span(x0, x1);
            let tail = self.string.split_off(span.offsets.end);
            self.string.truncate(span.offsets.start);
            self.string.extend((span.columns.start..x0).map(|_| '?'));
            push_str(&mut self.string);
            self.string.extend((x1..span.columns.end).map(|_| '?'));
            self.string.push_str(&tail);
        }
        if w > 0 {
            let interval = ie(x0, x1);
            let _ = self.attributes.cut(&interval);
            if let Some(attribute) = attribute {
                self.attributes
                    .insert_merge_touching_if_values_equal(interval, attribute)
                    .expect("interval was cut");
            }
        }
    }

    pub fn put(&mut self, x0: usize, s: &str, attribute: Option<Attribute>) {
        self.string.reserve(s.len());
        self.put_closure(x0, Widths::new(s).sum(), |dst| dst.push_str(s), attribute);
    }

    pub fn put_multiple(&mut self, x0: usize, c: char, n: usize, attribute: Option<Attribute>) {
        self.string.reserve(c.len_utf8() * n);
        self.put_closure(
            x0,
            c.width().unwrap() * n,
            |dst| (0..n).for_each(|_| dst.push(c)),
            attribute,
        );
    }

    fn find_span(&self, x0: usize, x1: usize) -> Position {
        debug_assert!(x1 > x0);
        let p0 = self.find_pos(x0);
        let p1 = self.find_pos(x1 - 1);
        Position {
            columns: p0.columns.start..p1.columns.end,
            offsets: p0.offsets.start..p1.offsets.end,
        }
    }

    // Returns the [Position] that contains column `target_x`.
    fn find_pos(&self, target_x: usize) -> Position {
        let mut x = 0;
        let mut ofs = 0;
        let mut widths = Widths::new(&self.string);
        while let Some(w) = widths.next() {
            if x + w > target_x {
                return Position {
                    columns: x..x + w,
                    offsets: ofs..widths.offset(),
                };
            }
            ofs = widths.offset();
            x += w;
        }

        // This can happen if there are non-printable characters in a line.
        Position {
            columns: x..x,
            offsets: ofs..ofs,
        }
    }

    pub fn str(&self) -> &str {
        &self.string
    }

    pub fn display_overstrike(&self) -> DisplayOverstrike<'_> {
        DisplayOverstrike(self)
    }

    pub fn display_sgr(&self) -> DisplaySgr<'_> {
        DisplaySgr {
            line: self,
            width: 0,
        }
    }

    pub fn display_wiki(&self) -> DisplayWiki<'_> {
        DisplayWiki(self)
    }

    fn iter(&self) -> impl Iterator<Item = (&'_ str, Option<Attribute>)> {
        TextIter {
            next: None,
            line: self,
            attr: self.attributes.iter(),
            ofs: 0,
            x: 0,
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }
}

impl Display for TextLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.string.trim_end())
    }
}

pub struct DisplayWiki<'a>(&'a TextLine);

impl<'a> Display for DisplayWiki<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (s, attribute) in self.0.iter() {
            let (prefix, suffix) = if let Some(attribute) = attribute {
                attribute.affixes()
            } else {
                ("", "")
            };
            write!(f, "{prefix}{s}{suffix}")?;
        }
        Ok(())
    }
}

pub struct DisplayOverstrike<'a>(&'a TextLine);

impl<'a> Display for DisplayOverstrike<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let default = Attribute::default();
        for (s, attribute) in self.0.iter() {
            write!(
                f,
                "{}",
                attribute.as_ref().unwrap_or(&default).overstrike(s)
            )?;
        }
        Ok(())
    }
}

pub struct DisplaySgr<'a> {
    line: &'a TextLine,
    width: usize,
}

impl<'a> DisplaySgr<'a> {
    pub fn with_width(self, width: usize) -> Self {
        Self { width, ..self }
    }
}

impl<'a> Display for DisplaySgr<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let default = Attribute::default();
        for (s, attribute) in self.line.iter() {
            write!(f, "{}", attribute.as_ref().unwrap_or(&default).sgr(s))?;
        }
        if let Some(pad) = self.width.checked_sub(self.line.width) {
            struct Spaces(usize);
            impl Display for Spaces {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    for _ in 0..self.0 {
                        f.write_char(' ')?;
                    }
                    Ok(())
                }
            }
            write!(f, "{}", default.sgr(Spaces(pad)))?;
        }
        Ok(())
    }
}

struct TextIter<'a, I>
where
    I: DoubleEndedIterator<Item = (&'a Interval<usize>, &'a Attribute)>,
{
    line: &'a TextLine,
    next: Option<(&'a str, Option<Attribute>)>,
    attr: I,
    ofs: usize,
    x: usize,
}

impl<'a, I> TextIter<'a, I>
where
    I: DoubleEndedIterator<Item = (&'a Interval<usize>, &'a Attribute)>,
{
    fn take_up_to(&mut self, column: usize) -> &'a str {
        let mut x = self.x;
        for (index, c) in self.line.string[self.ofs..].char_indices() {
            let w = c.width().unwrap_or_default();
            if x + w > column {
                let result = &self.line.string[self.ofs..self.ofs + index];
                self.ofs += index;
                self.x = column;
                return result;
            }
            x += w;
        }
        let result = &self.line.string[self.ofs..];
        self.x = x;
        self.ofs = self.line.string.len();
        result
    }
}

impl<'a, I> Iterator for TextIter<'a, I>
where
    I: DoubleEndedIterator<Item = (&'a Interval<usize>, &'a Attribute)>,
{
    type Item = (&'a str, Option<Attribute>);

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(next) = self.next.take() {
            return Some(next);
        }

        match self.attr.next() {
            Some((interval, attribute)) => {
                let start = *interval.start();
                let end = *interval.end() + 1;
                if start > self.x {
                    let this = self.take_up_to(start);
                    self.next = Some((self.take_up_to(end), Some(*attribute)));
                    Some((this, None))
                } else {
                    Some((self.take_up_to(end), Some(*attribute)))
                }
            }
            None => {
                let rest = &self.line.string[self.ofs..];
                self.ofs = self.line.string.len();
                if rest.is_empty() {
                    None
                } else {
                    Some((rest, None))
                }
            }
        }
    }
}

/// Position of one or more characters within a [TextLine].
#[derive(Debug)]
struct Position {
    /// 0-based display columns.
    columns: Range<usize>,

    /// Byte offests.
    offsets: Range<usize>,
}

/// Iterates through the column widths in a string.
struct Widths<'a> {
    s: &'a str,
    base: &'a str,
}

impl<'a> Widths<'a> {
    fn new(s: &'a str) -> Self {
        Self { s, base: s }
    }

    /// Returns the amount of the string remaining to be visited.
    fn as_str(&self) -> &str {
        self.s
    }

    // Returns the offset into the original string of the characters remaining
    // to be visited.
    fn offset(&self) -> usize {
        self.base.len() - self.s.len()
    }
}

impl Iterator for Widths<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        let mut iter = self.s.char_indices();
        let (_, mut c) = iter.next()?;
        while iter.as_str().starts_with('\x08') {
            iter.next();
            c = match iter.next() {
                Some((_, c)) => c,
                _ => {
                    self.s = iter.as_str();
                    return Some(0);
                }
            };
        }

        let w = c.width().unwrap_or_default();
        if w == 0 {
            self.s = iter.as_str();
            return Some(0);
        }

        for (index, c) in iter {
            if c.width().is_some_and(|width| width > 0) {
                self.s = &self.s[index..];
                return Some(w);
            }
        }
        self.s = "";
        Some(w)
    }
}

pub fn clip_text<'a>(
    text: &'a str,
    bb: &Range<isize>,
    clip: &Range<isize>,
) -> Option<(isize, &'a str)> {
    let mut x = bb.start;
    let mut width = bb.len() as isize;

    let mut iter = text.chars();
    while x < clip.start {
        let c = iter.next()?;
        if let Some(w) = c.width() {
            let w = w as isize;
            x += w;
            width -= w;
            if width < 0 {
                return None;
            }
        }
    }
    if x + width > clip.end {
        if x >= clip.end {
            return None;
        }

        while x + width > clip.end {
            let c = iter.next_back()?;
            if let Some(w) = c.width() {
                width -= w as isize;
                if width < 0 {
                    return None;
                }
            }
        }
    }
    Some((x, iter.as_str()))
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;

    use crate::output::{drivers::text::text_line::Attribute, pivot::look::FontStyle};

    use super::TextLine;
    use enum_iterator::{Sequence, all};
    use itertools::Itertools;

    #[derive(Copy, Clone, Default, PartialEq, Eq, Sequence)]
    pub struct Emphasis {
        pub bold: bool,
        pub italic: bool,
        pub underline: bool,
    }

    impl From<&FontStyle> for Emphasis {
        fn from(style: &FontStyle) -> Self {
            Self {
                bold: style.bold,
                italic: style.italic,
                underline: style.underline,
            }
        }
    }

    impl From<Emphasis> for Attribute {
        fn from(value: Emphasis) -> Self {
            Attribute {
                bold: value.bold,
                italic: value.italic,
                underline: value.underline,
                ..Attribute::default()
            }
        }
    }

    impl Emphasis {
        fn plain() -> Self {
            Self::default()
        }
        pub fn is_plain(&self) -> bool {
            *self == Self::plain()
        }
        pub fn apply<'a>(&self, s: &'a str) -> String {
            let (prefix, suffix) = Attribute::from(*self).affixes();
            format!("{prefix}{s}{suffix}")
        }
    }

    impl Debug for Emphasis {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let mut attributes = Vec::new();
            if self.bold {
                attributes.push("bold");
            }
            if self.italic {
                attributes.push("italic");
            }
            if self.underline {
                attributes.push("underline");
            }
            if attributes.is_empty() {
                write!(f, "plain")
            } else {
                write!(f, "{}", attributes.into_iter().format("+"))
            }
        }
    }

    #[test]
    fn overwrite_rest_of_line() {
        for (x0, prefix) in [(0, ""), (1, " "), (2, "  ")] {
            for lowercase in all::<Emphasis>() {
                for uppercase in all::<Emphasis>() {
                    let mut line = TextLine::new();
                    line.put(x0, "abc", Some(lowercase.into()));
                    line.put(x0 + 1, "BCD", Some(uppercase.into()));

                    println!("{}", line.display_sgr());
                    assert_eq!(
                        line.display_wiki().to_string(),
                        if lowercase == uppercase {
                            format!("{prefix}{}", lowercase.apply("aBCD"))
                        } else {
                            format!("{prefix}{}{}", lowercase.apply("a"), uppercase.apply("BCD"))
                        },
                        "prefix={prefix:?} uppercase={uppercase:?} lowercase={lowercase:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn overwrite_partial_line() {
        for lowercase in all::<Emphasis>() {
            for uppercase in all::<Emphasis>() {
                let mut line = TextLine::new();
                // Produces `AbCDEf`.
                line.put(0, "abcdef", Some(lowercase.into()));
                line.put(0, "A", Some(uppercase.into()));
                line.put(2, "CDE", Some(uppercase.into()));
                assert_eq!(
                    line.display_wiki().to_string(),
                    if lowercase == uppercase {
                        lowercase.apply("AbCDEf")
                    } else {
                        format!(
                            "{}{}{}{}",
                            uppercase.apply("A"),
                            lowercase.apply("b"),
                            uppercase.apply("CDE"),
                            lowercase.apply("f")
                        )
                    },
                    "uppercase={uppercase:?} lowercase={lowercase:?}"
                );
            }
        }
    }

    #[test]
    fn overwrite_rest_with_double_width() {
        for lowercase in all::<Emphasis>() {
            for hiragana in all::<Emphasis>() {
                let mut line = TextLine::new();
                // Produces `kaきくけ`.
                line.put(0, "kakiku", Some(lowercase.into()));
                line.put(2, "きくけ", Some(hiragana.into()));

                assert_eq!(
                    line.display_wiki().to_string(),
                    if lowercase == hiragana {
                        lowercase.apply("kaきくけ")
                    } else {
                        lowercase.apply("ka") + &hiragana.apply("きくけ")
                    },
                    "lowercase={lowercase:?} hiragana={hiragana:?}"
                );
            }
        }
    }

    #[test]
    fn overwrite_partial_with_double_width() {
        for lowercase in all::<Emphasis>() {
            for hiragana in all::<Emphasis>() {
                let mut line = TextLine::new();
                // Produces `かkiくけko`.
                line.put(0, "kakikukeko", Some(lowercase.into()));
                line.put(0, "か", Some(hiragana.into()));
                line.put(4, "くけ", Some(hiragana.into()));
                assert_eq!(
                    line.display_wiki().to_string(),
                    if lowercase == hiragana {
                        lowercase.apply("かkiくけko")
                    } else {
                        hiragana.apply("か")
                            + &lowercase.apply("ki")
                            + &hiragana.apply("くけ")
                            + &lowercase.apply("ko")
                    },
                    "lowercase={lowercase:?} hiragana={hiragana:?}"
                );
            }
        }
    }

    /// Overwrite rest of line, aligned double-width over double-width
    #[test]
    fn aligned_double_width_rest_of_line() {
        for bottom in all::<Emphasis>() {
            for top in all::<Emphasis>() {
                let mut line = TextLine::new();
                // Produces `あきくけ`.
                line.put(0, "あいう", Some(bottom.into()));
                line.put(2, "きくけ", Some(top.into()));

                assert_eq!(
                    line.display_wiki().to_string(),
                    if top == bottom {
                        top.apply("あきくけ")
                    } else {
                        bottom.apply("あ") + &top.apply("きくけ")
                    },
                    "bottom={bottom:?} top={top:?}"
                );
            }
        }
    }

    /// Overwrite rest of line, misaligned double-width over double-width
    #[test]
    fn misaligned_double_width_rest_of_line() {
        for bottom in all::<Emphasis>() {
            for top in all::<Emphasis>() {
                let mut line = TextLine::new();
                // Produces `あきくけ`.
                line.put(0, "あいう", Some(bottom.into()));
                line.put(3, "きくけ", Some(top.into()));

                assert_eq!(
                    line.display_wiki().to_string(),
                    if top == bottom {
                        top.apply("あ?きくけ")
                    } else {
                        bottom.apply("あ?") + &top.apply("きくけ")
                    },
                    "bottom={bottom:?} top={top:?}"
                );
            }
        }
    }

    /// Overwrite partial line, aligned double-width over double-width
    #[test]
    fn aligned_double_width_partial() {
        for bottom in all::<Emphasis>() {
            for top in all::<Emphasis>() {
                let mut line = TextLine::new();
                // Produces `かいくけお`.
                line.put(0, "あいうえお", Some(bottom.into()));
                line.put(0, "か", Some(top.into()));
                line.put(4, "くけ", Some(top.into()));
                assert_eq!(
                    line.display_wiki().to_string(),
                    if top == bottom {
                        top.apply("かいくけお")
                    } else {
                        top.apply("か")
                            + &bottom.apply("い")
                            + &top.apply("くけ")
                            + &bottom.apply("お")
                    },
                    "bottom={bottom:?} top={top:?}"
                );
            }
        }
    }

    /// Overwrite partial line, misaligned double-width over double-width
    #[test]
    fn misaligned_double_width_partial() {
        for bottom in all::<Emphasis>() {
            for top in all::<Emphasis>() {
                let mut line = TextLine::new();
                // Produces `?か?うえおさ`.
                line.put(0, "あいうえおさ", Some(bottom.into()));
                line.put(1, "か", Some(top.into()));
                assert_eq!(
                    line.display_wiki().to_string(),
                    if top == bottom {
                        top.apply("?か?うえおさ")
                    } else {
                        bottom.apply("?") + &top.apply("か") + &bottom.apply("?うえおさ")
                    },
                    "bottom={bottom:?} top={top:?}"
                );

                // Produces `?か??くけ?さ`.
                line.put(5, "くけ", Some(top.into()));

                assert_eq!(
                    line.display_wiki().to_string(),
                    if top == bottom {
                        top.apply("?か??くけ?さ")
                    } else {
                        bottom.apply("?")
                            + &top.apply("か")
                            + &bottom.apply("??")
                            + &top.apply("くけ")
                            + &bottom.apply("?さ")
                    },
                    "bottom={bottom:?} top={top:?}"
                );
            }
        }
    }

    /// Overwrite rest of line, aligned single-width over double-width.
    #[test]
    fn aligned_rest_single_over_double() {
        for bottom in all::<Emphasis>() {
            for top in all::<Emphasis>() {
                let mut line = TextLine::new();
                // Produces `あkikuko`.
                line.put(0, "あいう", Some(bottom.into()));
                line.put(2, "kikuko", Some(top.into()));
                assert_eq!(
                    line.display_wiki().to_string(),
                    if top == bottom {
                        top.apply("あkikuko")
                    } else {
                        bottom.apply("あ") + &top.apply("kikuko")
                    },
                    "bottom={bottom:?} top={top:?}"
                );
            }
        }
    }

    /// Overwrite rest of line, misaligned single-width over double-width.
    #[test]
    fn misaligned_rest_single_over_double() {
        for bottom in all::<Emphasis>() {
            for top in all::<Emphasis>() {
                let mut line = TextLine::new();
                // Produces `あ?kikuko`.
                line.put(0, "あいう", Some(bottom.into()));
                line.put(3, "kikuko", Some(top.into()));
                assert_eq!(
                    line.display_wiki().to_string(),
                    if top == bottom {
                        top.apply("あ?kikuko")
                    } else {
                        bottom.apply("あ?") + &top.apply("kikuko")
                    },
                    "bottom={bottom:?} top={top:?}"
                );
            }
        }
    }

    /// Overwrite partial line, aligned single-width over double-width.
    #[test]
    fn aligned_partial_single_over_double() {
        for bottom in all::<Emphasis>() {
            for top in all::<Emphasis>() {
                let mut line = TextLine::new();
                // Produces `kaいうえお`.
                line.put(0, "あいうえお", Some(bottom.into()));
                line.put(0, "ka", Some(top.into()));

                assert_eq!(
                    line.display_wiki().to_string(),
                    if top == bottom {
                        top.apply("kaいうえお")
                    } else {
                        top.apply("ka") + &bottom.apply("いうえお")
                    },
                    "bottom={bottom:?} top={top:?}"
                );
                // Produces `kaいkukeお`.
                line.put(4, "kuke", Some(top.into()));
                assert_eq!(
                    line.display_wiki().to_string(),
                    if top == bottom {
                        top.apply("kaいkukeお")
                    } else {
                        top.apply("ka")
                            + &bottom.apply("い")
                            + &top.apply("kuke")
                            + &bottom.apply("お")
                    },
                    "bottom={bottom:?} top={top:?}"
                );
            }
        }
    }

    /// Overwrite partial line, misaligned single-width over double-width.
    #[test]
    fn misaligned_partial_single_over_double() {
        for bottom in all::<Emphasis>() {
            for top in all::<Emphasis>() {
                let mut line = TextLine::new();
                // Produces `?aいうえおさ`.
                line.put(0, "あいうえおさ", Some(bottom.into()));
                line.put(1, "a", Some(top.into()));
                assert_eq!(
                    line.display_wiki().to_string(),
                    if top == bottom {
                        top.apply("?aいうえおさ")
                    } else {
                        bottom.apply("?") + &top.apply("a") + &bottom.apply("いうえおさ")
                    },
                    "bottom={bottom:?} top={top:?}"
                );

                // Produces `?aい?kuke?さ`.
                line.put(5, "kuke", Some(top.into()));
                assert_eq!(
                    line.display_wiki().to_string(),
                    if top == bottom {
                        top.apply("?aい?kuke?さ")
                    } else {
                        bottom.apply("?")
                            + &top.apply("a")
                            + &bottom.apply("い?")
                            + &top.apply("kuke")
                            + &bottom.apply("?さ")
                    },
                    "bottom={bottom:?} top={top:?}"
                );
            }
        }
    }
}
