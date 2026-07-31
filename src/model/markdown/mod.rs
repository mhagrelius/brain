//! A Markdown scanner for live-styled editing.
//!
//! # What this is for
//!
//! Notes are always shown as source and always styled. The syntax characters
//! are hidden everywhere except in the block holding the cursor, which is one
//! `GtkTextView` throughout — not two widgets swapped — so the cursor lands
//! where you clicked and the scroll position never jumps. The view achieves it
//! by applying tags for style and marking the syntax characters with a tag
//! whose `invisible` property is removed from the cursor's block.
//!
//! So this scanner has an unusual requirement: as well as *what* is styled, it
//! must report exactly **which characters are syntax**, so they can be hidden.
//! A general Markdown library reports the former and not the latter, which is
//! why this is written here rather than pulled from crates.io. See DESIGN.md.
//!
//! # Offsets
//!
//! Everything is in **character** offsets, because that is what
//! `GtkTextBuffer::iter_at_offset` takes. Byte offsets would silently corrupt
//! any note containing an accent or an emoji.
//!
//! # Scope
//!
//! A subset of CommonMark plus the notebook syntax: wikilinks, embeds, tags and
//! task checkboxes. Not in it: reference links, nested emphasis, setext
//! headings, tables, HTML. Anything unrecognised is left as plain text rather
//! than half-styled, because half-typed formatting is the normal state while
//! writing and must never swallow the rest of the note.
//!
//! # Lines
//!
//! Scanning is line-based, and inline styling therefore does not cross a line
//! break. That is a deliberate trade: it is what makes [`scan_line`] — re-scan
//! exactly one line on a keystroke — possible, and the editor wraps rather than
//! hard-wraps, so emphasis spanning a newline is rare in practice.

mod links;
mod scan;

pub use links::{extract, extract_with, rewrite_target, Extracted, TagRef, WikiLink};

/// A styled region of the note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// `# ` … `###### `, level 1–6.
    Heading(u8),
    Bold,
    Italic,
    Strikethrough,
    /// `` `inline` ``
    Code,
    /// A line inside a ``` fence.
    CodeBlock,
    /// `> quoted`
    Quote,
    /// The content of a `- ` or `1. ` item. The level is the nesting depth,
    /// counted from zero.
    ListItem(u8),
    /// The visible text of `[text](url)`, or a bare URL.
    Link,
    /// The visible text of `[[Target]]` or `[[Target|shown]]`.
    WikiLink,
    /// `![[attachment]]`. The UI renders the file beneath the line.
    Embed,
    /// `#tag`, including the hash — it is part of the chip, not syntax.
    Tag,
    /// The `[ ]` or `[x]` of a task item. `true` when ticked.
    Task(bool),
    /// `---`, `***` or `___` alone on a line.
    Rule,
    /// A `| a | b |` row. The pipes stay visible: a text view cannot draw
    /// column rules, so they are the table.
    TableRow,
    /// The `|---|---|` row under a header, which is pure syntax.
    TableDelimiter,
    /// A line between the opening and closing `---` of frontmatter.
    Frontmatter,
}

/// Deepest nesting level with an indent of its own.
///
/// Past a handful of levels the margin would eat the note, so the styling stops
/// deepening; the scan is still honest about the structure.
pub const MAX_LIST_DEPTH: u8 = 4;

/// The indent widths of the list the scanner is currently inside.
///
/// Depth is taken from the widths already seen in this list rather than from a
/// fixed number of spaces per level, so two-space and four-space notes both
/// nest one level at a time — and a note that mixes them still nests
/// monotonically.
///
/// A fixed-size stack rather than a `Vec`, so this stays `Copy` and can ride
/// alongside [`LineState`] through the editor's per-line cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListLevels {
    widths: [u8; MAX_LIST_DEPTH as usize + 2],
    len: u8,
}

impl ListLevels {
    /// The nesting depth of an item indented by `indent` spaces, recording it
    /// as a level if it is a new one.
    pub(crate) fn depth(&mut self, indent: usize) -> u8 {
        // Indents deeper than this are pathological and would not fit the
        // stack; treating them as the deepest known level is closer to what
        // was meant than wrapping round.
        let indent = indent.min(u8::MAX as usize) as u8;

        while self.len > 0 && self.widths[self.len as usize - 1] > indent {
            self.len -= 1;
        }
        let known = self.len > 0 && self.widths[self.len as usize - 1] == indent;
        if !known && (self.len as usize) < self.widths.len() {
            self.widths[self.len as usize] = indent;
            self.len += 1;
        }
        self.len.saturating_sub(1).min(MAX_LIST_DEPTH)
    }

    /// Leave the list. The next item starts counting again.
    pub(crate) fn clear(&mut self) {
        self.len = 0;
    }
}

/// A run of characters to style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Inclusive character offset of the first character.
    pub start: usize,
    /// Exclusive character offset of the end.
    pub end: usize,
    pub style: Style,
}

/// A run of characters that is syntax rather than content, and is hidden unless
/// the cursor is in this block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    pub start: usize,
    pub end: usize,
}

/// What a line's meaning depends on before its first character is read.
///
/// The editor caches one of these per line. A keystroke that leaves the line's
/// *outgoing* state unchanged needs only that line re-scanned; one that changes
/// it — opening a fence, touching a frontmatter delimiter — escalates to a full
/// re-scan of the note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineState {
    #[default]
    Normal,
    /// Inside a ``` or ~~~ fence.
    Fence,
    /// Inside the leading `---` block.
    Frontmatter,
    /// Inside a table, after a header and its delimiter row.
    Table,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    pub spans: Vec<Span>,
    pub markers: Vec<Marker>,
    /// The state each line begins in. `line_states.len()` is the line count.
    pub line_states: Vec<LineState>,
    /// The list nesting each line begins in, alongside `line_states`.
    pub line_lists: Vec<ListLevels>,
}

impl Parsed {
    pub(crate) fn push_span(&mut self, start: usize, end: usize, style: Style) {
        if end > start {
            self.spans.push(Span { start, end, style });
        }
    }

    pub(crate) fn push_marker(&mut self, start: usize, end: usize) {
        if end > start {
            self.markers.push(Marker { start, end });
        }
    }
}

/// Parse a whole note into styled spans and hideable syntax markers.
pub fn parse(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let chars: Vec<char> = text.chars().collect();

    let mut line_start = 0usize;
    // Frontmatter is only frontmatter at the very top of the file.
    let mut state = if starts_frontmatter(&chars) {
        LineState::Frontmatter
    } else {
        LineState::Normal
    };
    let mut list = ListLevels::default();
    let mut first = true;

    loop {
        let line_end = chars[line_start..]
            .iter()
            .position(|&c| c == '\n')
            .map(|offset| line_start + offset)
            .unwrap_or(chars.len());

        // The line after this one, which table headers need in order to know
        // they are headers.
        let next = (line_end < chars.len()).then(|| {
            let start = line_end + 1;
            let end = chars[start..]
                .iter()
                .position(|&c| c == '\n')
                .map(|offset| start + offset)
                .unwrap_or(chars.len());
            &chars[start..end]
        });

        parsed.line_states.push(state);
        parsed.line_lists.push(list);
        // The opening `---` is consumed as the frontmatter delimiter, not as a
        // thematic break, which is why the first line is special-cased.
        state = scan::line(
            &chars[line_start..line_end],
            line_start,
            state,
            first,
            next,
            &mut list,
            &mut parsed,
        );
        first = false;

        if line_end >= chars.len() {
            break;
        }
        line_start = line_end + 1;
    }

    parsed
}

/// Re-scan a single line, for the editor's per-keystroke path.
///
/// `offset` is the character offset of the line's first character within the
/// note, so the spans come back in note coordinates and can be applied
/// directly. `state` is the cached [`LineState`] for this line, and `next` is
/// the line following it, which table headers need. The returned state is what
/// the *next* line begins in: if it differs from the cached one, the caller
/// must re-scan the rest of the note.
pub fn scan_line(
    line: &str,
    offset: usize,
    state: LineState,
    list: ListLevels,
    next: Option<&str>,
) -> (Parsed, LineState, ListLevels) {
    let chars: Vec<char> = line.chars().collect();
    let following: Option<Vec<char>> = next.map(|line| line.chars().collect());
    let mut list = list;
    let mut parsed = Parsed::default();
    parsed.line_states.push(state);
    parsed.line_lists.push(list);
    // `first` is false: a re-scanned line is never the note's opening `---`,
    // because touching that delimiter escalates to a full re-scan by design.
    let outgoing = scan::line(
        &chars,
        offset,
        state,
        false,
        following.as_deref(),
        &mut list,
        &mut parsed,
    );
    (parsed, outgoing, list)
}

/// The note's text with Markdown syntax removed.
///
/// For anywhere a note is *named* rather than shown: search results, link
/// autocompletion, the window subtitle. A snippet reading "# Shopping"
/// advertises the file format; it should read "Shopping".
///
/// Goes further than hiding markers in the editor. List bullets stay visible
/// while editing, because a text view has no glyph to put in their place — but
/// in a one-line snippet "- milk" is noise, so they are dropped here too.
pub fn strip(text: &str) -> String {
    strip_with(text, &parse(text))
}

/// [`strip`], reusing a scan the caller already has.
///
/// The index derives four things from every note and parsing once for each was
/// most of the cost of opening a vault.
pub fn strip_with(text: &str, parsed: &Parsed) -> String {
    let chars: Vec<char> = text.chars().collect();

    let mut hidden = vec![false; chars.len()];
    let hide = |from: usize, to: usize, hidden: &mut Vec<bool>| {
        for flag in hidden.iter_mut().take(to.min(chars.len())).skip(from) {
            *flag = true;
        }
    };

    for marker in &parsed.markers {
        hide(marker.start, marker.end, &mut hidden);
    }
    // Frontmatter is metadata, never prose. It has no markers of its own
    // because the editor styles it in place, so it is dropped here explicitly.
    // A table's delimiter row goes the same way, and its pipes below.
    for span in &parsed.spans {
        if matches!(
            span.style,
            Style::Frontmatter | Style::Rule | Style::TableDelimiter
        ) {
            hide(span.start, span.end, &mut hidden);
        }
    }

    // Bullets are content in the editor and clutter in a snippet.
    let mut line_start = 0usize;
    while line_start < chars.len() {
        let line_end = chars[line_start..]
            .iter()
            .position(|&c| c == '\n')
            .map(|offset| line_start + offset)
            .unwrap_or(chars.len());
        let line = &chars[line_start..line_end];
        let indent = line.iter().take_while(|c| **c == ' ').count();
        let rest = &line[indent..];
        if let Some(len) = scan::bullet_len(rest).or_else(|| scan::ordered_len(rest)) {
            hide(line_start, line_start + indent + len, &mut hidden);
        }
        line_start = line_end + 1;
    }

    // A table row reads as its cells. The pipes are structure the editor needs
    // and a one-line snippet does not, so they go — and with them the padding
    // that lined the columns up, which would otherwise leave gaping runs of
    // spaces mid-sentence.
    let mut in_row = vec![false; chars.len()];
    for span in &parsed.spans {
        if span.style == Style::TableRow {
            for at in span.start..span.end.min(chars.len()) {
                in_row[at] = true;
                if chars[at] == '|' {
                    hidden[at] = true;
                }
            }
        }
    }

    let mut out = String::with_capacity(chars.len());
    for (at, character) in chars.iter().enumerate() {
        if hidden[at] {
            continue;
        }
        if in_row[at] && *character == ' ' && out.ends_with(' ') {
            continue;
        }
        out.push(*character);
    }
    out
}

/// Whether the note opens with a frontmatter delimiter.
///
/// Exactly `---` on the first line. Not `----`, and not after a blank line —
/// otherwise every thematic break at the top of a note would silently turn the
/// prose beneath it into metadata.
fn starts_frontmatter(chars: &[char]) -> bool {
    let end = chars.iter().position(|&c| c == '\n').unwrap_or(chars.len());
    chars[..end].iter().collect::<String>().trim_end() == "---"
}

#[cfg(test)]
mod tests;

/// A formatting action the UI can apply to a selection or a line.
///
/// Named here rather than in the UI because what each one *means* — which
/// characters wrap a bold run, what prefixes a task — is the scanner's
/// business, and the two must agree or a button would write syntax the editor
/// does not style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Bold,
    Italic,
    Strikethrough,
    Code,
    /// `# ` … `###### `
    Heading(u8),
    Quote,
    Bullet,
    Task,
    /// `[[ ]]`, left for the completion popover to fill in.
    WikiLink,
    Link,
    CodeBlock,
    Table,
    Rule,
}

/// How a [`Format`] is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// Put `before` and `after` either side of the selection. With nothing
    /// selected the caret lands between them.
    Wrap { before: String, after: String },
    /// Put `prefix` at the start of each selected line, or remove it if every
    /// one already has it.
    Prefix { prefix: String },
    /// Insert a block of text on lines of its own. `caret` is how many
    /// characters into it the cursor should land.
    Block { text: String, caret: usize },
}

impl Format {
    pub fn edit(self) -> Edit {
        let wrap = |marker: &str| Edit::Wrap {
            before: marker.to_string(),
            after: marker.to_string(),
        };
        match self {
            Self::Bold => wrap("**"),
            Self::Italic => wrap("*"),
            Self::Strikethrough => wrap("~~"),
            Self::Code => wrap("`"),
            Self::WikiLink => Edit::Wrap {
                before: "[[".into(),
                after: "]]".into(),
            },
            Self::Link => Edit::Wrap {
                before: "[".into(),
                after: "](https://)".into(),
            },
            Self::Heading(level) => Edit::Prefix {
                prefix: format!("{} ", "#".repeat(level.clamp(1, 6) as usize)),
            },
            Self::Quote => Edit::Prefix {
                prefix: "> ".into(),
            },
            Self::Bullet => Edit::Prefix {
                prefix: "- ".into(),
            },
            Self::Task => Edit::Prefix {
                prefix: "- [ ] ".into(),
            },
            Self::CodeBlock => Edit::Block {
                text: "```\n\n```\n".into(),
                caret: 4,
            },
            Self::Table => Edit::Block {
                text: "| Column | Column |\n|--------|--------|\n|        |        |\n".into(),
                caret: 2,
            },
            Self::Rule => Edit::Block {
                text: "---\n".into(),
                caret: 4,
            },
        }
    }

    /// The style this format produces, for the inline ones.
    ///
    /// The editor uses it to ask the scanner whether the cursor is already
    /// inside one of these, so pressing the button again takes it off rather
    /// than nesting a second pair of markers.
    pub fn style(self) -> Option<Style> {
        match self {
            Self::Bold => Some(Style::Bold),
            Self::Italic => Some(Style::Italic),
            Self::Strikethrough => Some(Style::Strikethrough),
            Self::Code => Some(Style::Code),
            Self::WikiLink => Some(Style::WikiLink),
            Self::Link => Some(Style::Link),
            _ => None,
        }
    }

    /// What the button says.
    pub fn label(self) -> &'static str {
        match self {
            Self::Bold => "Bold",
            Self::Italic => "Italic",
            Self::Strikethrough => "Strikethrough",
            Self::Code => "Code",
            Self::Heading(1) => "Heading 1",
            Self::Heading(2) => "Heading 2",
            Self::Heading(_) => "Heading 3",
            Self::Quote => "Quote",
            Self::Bullet => "List",
            Self::Task => "Task",
            Self::WikiLink => "Link to Note",
            Self::Link => "Web Link",
            Self::CodeBlock => "Code Block",
            Self::Table => "Table",
            Self::Rule => "Separator",
        }
    }

    /// The syntax it writes, shown beside the label so the panel teaches the
    /// Markdown rather than hiding it.
    pub fn syntax(self) -> &'static str {
        match self {
            Self::Bold => "**text**",
            Self::Italic => "*text*",
            Self::Strikethrough => "~~text~~",
            Self::Code => "`code`",
            Self::Heading(level) => match level {
                1 => "# ",
                2 => "## ",
                _ => "### ",
            },
            Self::Quote => "> ",
            Self::Bullet => "- ",
            Self::Task => "- [ ] ",
            Self::WikiLink => "[[Note]]",
            Self::Link => "[text](…)",
            Self::CodeBlock => "```",
            Self::Table => "| a | b |",
            Self::Rule => "---",
        }
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;

    /// Every format must write syntax this scanner actually styles, or a
    /// button would produce text the editor renders as plain prose.
    #[test]
    fn every_format_writes_syntax_the_scanner_recognises() {
        let cases: &[(Format, Style)] = &[
            (Format::Bold, Style::Bold),
            (Format::Italic, Style::Italic),
            (Format::Strikethrough, Style::Strikethrough),
            (Format::Code, Style::Code),
            (Format::Heading(1), Style::Heading(1)),
            (Format::Heading(2), Style::Heading(2)),
            (Format::Heading(3), Style::Heading(3)),
            (Format::Quote, Style::Quote),
            (Format::Bullet, Style::ListItem(0)),
            (Format::Task, Style::Task(false)),
            (Format::WikiLink, Style::WikiLink),
            (Format::Link, Style::Link),
        ];

        for (format, expected) in cases {
            let written = match format.edit() {
                Edit::Wrap { before, after } => format!("{before}sample{after}"),
                Edit::Prefix { prefix } => format!("{prefix}sample"),
                Edit::Block { text, .. } => text,
            };
            let styles: Vec<Style> = parse(&written).spans.iter().map(|s| s.style).collect();
            assert!(
                styles.contains(expected),
                "{format:?} wrote {written:?}, which parses as {styles:?}"
            );
        }
    }

    #[test]
    fn block_formats_parse_as_their_block() {
        let Edit::Block { text, .. } = Format::Table.edit() else {
            panic!("a block");
        };
        let styles: Vec<Style> = parse(&text).spans.iter().map(|s| s.style).collect();
        assert!(styles.contains(&Style::TableRow), "{styles:?}");

        // A freshly inserted code block is empty, so there is no line inside
        // it to style — what proves it is a fence is the state it opens.
        let Edit::Block { text, .. } = Format::CodeBlock.edit() else {
            panic!("a block");
        };
        assert!(
            parse(&text).line_states.contains(&LineState::Fence),
            "the inserted fence does not open a code block"
        );

        // A rule is only a rule below the first line, since a leading `---`
        // opens frontmatter.
        let Edit::Block { text, .. } = Format::Rule.edit() else {
            panic!("a block");
        };
        let styles: Vec<Style> = parse(&format!("Prose.\n{text}"))
            .spans
            .iter()
            .map(|s| s.style)
            .collect();
        assert!(styles.contains(&Style::Rule), "{styles:?}");
    }

    #[test]
    fn the_caret_lands_inside_the_block_it_wrote() {
        for format in [Format::CodeBlock, Format::Table, Format::Rule] {
            let Edit::Block { text, caret } = format.edit() else {
                panic!("a block");
            };
            assert!(
                caret <= text.chars().count(),
                "{format:?} puts the caret past the end of what it wrote"
            );
        }
    }
}
