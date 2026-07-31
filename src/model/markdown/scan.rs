//! The line scanner. See the module docs in `mod.rs` for why this exists.

use super::{LineState, ListLevels, Parsed, Style};

/// Scan one line, and report the state the *next* line begins in.
///
/// `first` marks the note's opening line, which is the only place a `---` can
/// open frontmatter rather than draw a thematic break.
///
/// `next` is the line after this one, which only tables need: a row of pipes is
/// a table header if and only if a delimiter row follows it, and without that
/// lookahead every sentence containing a `|` becomes a table.
pub(super) fn line(
    chars: &[char],
    offset: usize,
    state: LineState,
    first: bool,
    next: Option<&[char]>,
    list: &mut ListLevels,
    parsed: &mut Parsed,
) -> LineState {
    match state {
        LineState::Frontmatter => {
            // Both delimiters and the metadata between them style as
            // frontmatter: it is one visually recessed block, not markup with
            // content inside it.
            parsed.push_span(offset, offset + chars.len(), Style::Frontmatter);
            if !first && is_delimiter(chars) {
                LineState::Normal
            } else {
                LineState::Frontmatter
            }
        }
        LineState::Fence => {
            if is_fence(chars) {
                parsed.push_marker(offset, offset + chars.len());
                LineState::Normal
            } else {
                parsed.push_span(offset, offset + chars.len(), Style::CodeBlock);
                LineState::Fence
            }
        }
        LineState::Table => {
            if is_table_delimiter(chars) {
                parsed.push_span(offset, offset + chars.len(), Style::TableDelimiter);
                LineState::Table
            } else if is_table_row(chars) {
                table_row(chars, offset, parsed);
                LineState::Table
            } else {
                // The first line that is not a row ends the table, and is then
                // scanned as whatever it actually is.
                line(chars, offset, LineState::Normal, false, next, list, parsed)
            }
        }
        LineState::Normal => {
            if is_fence(chars) {
                // The fence itself is syntax; the lines between are code.
                parsed.push_marker(offset, offset + chars.len());
                LineState::Fence
            } else if is_rule(chars) {
                parsed.push_span(offset, offset + chars.len(), Style::Rule);
                LineState::Normal
            } else if is_table_row(chars) && next.is_some_and(is_table_delimiter) {
                table_row(chars, offset, parsed);
                LineState::Table
            } else {
                content(chars, offset, list, parsed);
                LineState::Normal
            }
        }
    }
}

/// ``` or ~~~ , optionally with a language after it.
fn is_fence(line: &[char]) -> bool {
    let trimmed: Vec<char> = line.iter().copied().skip_while(|c| *c == ' ').collect();
    trimmed.starts_with(&['`', '`', '`']) || trimmed.starts_with(&['~', '~', '~'])
}

/// Exactly `---`, the frontmatter delimiter.
fn is_delimiter(line: &[char]) -> bool {
    line.iter().collect::<String>().trim_end() == "---"
}

/// A thematic break: three or more of `-`, `*` or `_` and nothing else.
fn is_rule(line: &[char]) -> bool {
    let trimmed: Vec<char> = line
        .iter()
        .copied()
        .filter(|c| !c.is_whitespace())
        .collect();
    if trimmed.len() < 3 {
        return false;
    }
    let first = trimmed[0];
    matches!(first, '-' | '*' | '_') && trimmed.iter().all(|&c| c == first)
}

/// A row of a table: `| a | b |`.
///
/// A leading pipe is required. GFM allows rows without one, but prose like
/// "yes | no" is far more common in notes than a borderless table, and reading
/// a sentence as a table is the worse failure.
fn is_table_row(line: &[char]) -> bool {
    let trimmed: Vec<char> = line.iter().copied().skip_while(|c| *c == ' ').collect();
    trimmed.first() == Some(&'|') && trimmed.iter().filter(|c| **c == '|').count() >= 2
}

/// The row under a table's header: `|---|:--:|`, dashes and alignment colons.
fn is_table_delimiter(line: &[char]) -> bool {
    if !is_table_row(line) {
        return false;
    }
    let text: String = line.iter().collect();
    let cells: Vec<&str> = text.trim().trim_matches('|').split('|').collect();
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let cell = cell.trim();
            cell.contains('-') && cell.chars().all(|c| c == '-' || c == ':')
        })
}

/// A table row: the pipes stay visible, the cells are styled inline.
///
/// Hiding the pipes would be wrong for the same reason hiding list bullets is:
/// a text view cannot draw column rules, so the pipes *are* the table.
fn table_row(chars: &[char], offset: usize, parsed: &mut Parsed) {
    parsed.push_span(offset, offset + chars.len(), Style::TableRow);
    inline(chars, offset, parsed);
}

/// A normal line: block prefix, then inline styling of what follows it.
fn content(line: &[char], offset: usize, list: &mut ListLevels, parsed: &mut Parsed) {
    if line.is_empty() {
        // A blank line separates items but does not end the list.
        return;
    }

    let indent = line.iter().take_while(|c| **c == ' ').count();
    let rest = &line[indent..];

    let is_item = bullet_len(rest).or_else(|| ordered_len(rest)).is_some();
    if !is_item && indent == 0 {
        // Anything else at the left edge ends the list; an indented line is a
        // continuation of the item above and leaves the nesting alone.
        list.clear();
    }

    let (content_start, block_style) = if let Some(level) = heading_level(rest) {
        // "### " — the hashes and the space are syntax.
        let marker_len = level as usize + 1;
        parsed.push_marker(offset + indent, offset + indent + marker_len);
        (indent + marker_len, Some(Style::Heading(level)))
    } else if rest.starts_with(&['>']) {
        let marker_len = if rest.get(1) == Some(&' ') { 2 } else { 1 };
        parsed.push_marker(offset + indent, offset + indent + marker_len);
        (indent + marker_len, Some(Style::Quote))
    } else if let Some(marker_len) = bullet_len(rest).or_else(|| ordered_len(rest)) {
        // The bullet stays *visible*: hiding it would delete the only thing
        // that makes a list look like a list, since a text view cannot
        // substitute a nicer glyph for it. The spaces in front of it are
        // syntax, though — the level's margin does that job now, and leaving
        // them in would indent a nested item twice over.
        parsed.push_marker(offset, offset + indent);
        let style = Style::ListItem(list.depth(indent));

        let after = &rest[marker_len..];
        match checkbox(after) {
            // The checkbox stays visible too, for the same reason, and because
            // the UI turns it into something clickable in place.
            Some((ticked, len)) => {
                parsed.push_span(
                    offset + indent + marker_len,
                    offset + indent + marker_len + len,
                    Style::Task(ticked),
                );
                (indent + marker_len + len, Some(style))
            }
            None => (indent + marker_len, Some(style)),
        }
    } else {
        (indent, None)
    };

    let content_start = content_start.min(line.len());
    if let Some(style) = block_style {
        parsed.push_span(offset + content_start, offset + line.len(), style);
    }

    inline(&line[content_start..], offset + content_start, parsed);
}

/// `#` to `######` followed by a space.
fn heading_level(line: &[char]) -> Option<u8> {
    let hashes = line.iter().take_while(|c| **c == '#').count();
    if (1..=6).contains(&hashes) && line.get(hashes) == Some(&' ') {
        Some(hashes as u8)
    } else {
        None
    }
}

/// `- `, `* ` or `+ `. Not `*emphasis*`, which has no space.
pub(super) fn bullet_len(line: &[char]) -> Option<usize> {
    match line.first() {
        Some('-') | Some('*') | Some('+') if line.get(1) == Some(&' ') => Some(2),
        _ => None,
    }
}

/// `1. ` / `12) `
pub(super) fn ordered_len(line: &[char]) -> Option<usize> {
    let digits = line.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    match (line.get(digits), line.get(digits + 1)) {
        (Some('.'), Some(' ')) | (Some(')'), Some(' ')) => Some(digits + 2),
        _ => None,
    }
}

/// `[ ] ` or `[x] ` immediately after a bullet. Returns ticked-ness and the
/// length of the brackets, which does not include the trailing space — the
/// space belongs to the content, so deleting the checkbox leaves clean text.
fn checkbox(line: &[char]) -> Option<(bool, usize)> {
    if line.first() != Some(&'[') || line.get(2) != Some(&']') || line.get(3) != Some(&' ') {
        return None;
    }
    match line.get(1) {
        Some(' ') => Some((false, 3)),
        Some('x') | Some('X') => Some((true, 3)),
        _ => None,
    }
}

/// Emphasis, code spans, links, wikilinks, embeds and tags within one line.
///
/// Each `try_*` returns the index to resume from, always greater than `i`, so
/// the loop cannot stall. An earlier version inferred progress from the last
/// span pushed, which spun forever on some inputs and skipped characters on
/// others — structural guarantees beat inference here.
///
/// Order is significance, not convenience: code wins over everything because
/// nothing inside it is formatting; embeds before wikilinks because `![[` is a
/// prefix problem; wikilinks before links because both start with a bracket.
fn inline(line: &[char], offset: usize, parsed: &mut Parsed) {
    let mut i = 0;
    while i < line.len() {
        let next = try_code(line, i, offset, parsed)
            .or_else(|| try_embed(line, i, offset, parsed))
            .or_else(|| try_wikilink(line, i, offset, parsed))
            .or_else(|| try_link(line, i, offset, parsed))
            .or_else(|| try_url(line, i, offset, parsed))
            .or_else(|| try_tag(line, i, offset, parsed))
            .or_else(|| try_emphasis(line, i, offset, parsed));
        i = next.unwrap_or(i + 1);
    }
}

/// `` `code` `` — wins over everything, since nothing inside it is formatting.
fn try_code(line: &[char], i: usize, offset: usize, parsed: &mut Parsed) -> Option<usize> {
    if line.get(i) != Some(&'`') {
        return None;
    }
    let close = find(line, i + 1, &['`'])?;
    parsed.push_marker(offset + i, offset + i + 1);
    parsed.push_span(offset + i + 1, offset + close, Style::Code);
    parsed.push_marker(offset + close, offset + close + 1);
    Some(close + 1)
}

/// `![[attachment.png]]` — the brackets are syntax, the filename is not.
///
/// The name stays legible because the rendered file appears on the line
/// beneath, and an image with no indication of which file it came from is
/// impossible to edit deliberately.
fn try_embed(line: &[char], i: usize, offset: usize, parsed: &mut Parsed) -> Option<usize> {
    if !line[i..].starts_with(&['!', '[', '[']) {
        return None;
    }
    let close = find(line, i + 3, &[']', ']'])?;
    if close == i + 3 {
        return None; // "![[]]" names nothing.
    }
    parsed.push_marker(offset + i, offset + i + 3);
    parsed.push_span(offset + i + 3, offset + close, Style::Embed);
    parsed.push_marker(offset + close, offset + close + 2);
    Some(close + 2)
}

/// `[[Target]]` or `[[Target|shown instead]]`.
fn try_wikilink(line: &[char], i: usize, offset: usize, parsed: &mut Parsed) -> Option<usize> {
    if !line[i..].starts_with(&['[', '[']) {
        return None;
    }
    let close = find(line, i + 2, &[']', ']'])?;
    if close == i + 2 {
        return None; // "[[]]" links nowhere.
    }
    // A pipe makes everything before it — target and separator alike — syntax,
    // leaving only the display text on show.
    let display_start = find(&line[..close], i + 2, &['|'])
        .map(|pipe| pipe + 1)
        .unwrap_or(i + 2);
    if display_start >= close {
        return None; // "[[Target|]]" has nothing to show.
    }
    parsed.push_marker(offset + i, offset + display_start);
    parsed.push_span(offset + display_start, offset + close, Style::WikiLink);
    parsed.push_marker(offset + close, offset + close + 2);
    Some(close + 2)
}

/// `[label](target)` — the label is what you read, the rest is syntax.
fn try_link(line: &[char], i: usize, offset: usize, parsed: &mut Parsed) -> Option<usize> {
    if line.get(i) != Some(&'[') {
        return None;
    }
    let label_end = find(line, i + 1, &[']'])?;
    if label_end == i + 1 || line.get(label_end + 1) != Some(&'(') {
        return None;
    }
    let close = find(line, label_end + 2, &[')'])?;
    parsed.push_marker(offset + i, offset + i + 1);
    parsed.push_span(offset + i + 1, offset + label_end, Style::Link);
    parsed.push_marker(offset + label_end, offset + close + 1);
    Some(close + 1)
}

/// A bare `https://…`, styled but not rewritten — there is no syntax to hide.
fn try_url(line: &[char], i: usize, offset: usize, parsed: &mut Parsed) -> Option<usize> {
    if !at_word_start(line, i) {
        return None;
    }
    if !line[i..].starts_with(&['h', 't', 't', 'p']) {
        return None;
    }
    let rest: String = line[i..].iter().collect();
    if !rest.starts_with("http://") && !rest.starts_with("https://") {
        return None;
    }
    let mut end = i + line[i..].iter().take_while(|c| !c.is_whitespace()).count();
    // Sentence punctuation after a URL belongs to the sentence.
    while end > i && matches!(line[end - 1], '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']') {
        end -= 1;
    }
    let scheme_len = if rest.starts_with("https://") { 8 } else { 7 };
    if end <= i + scheme_len {
        return None; // a scheme with no host is not a link yet.
    }
    parsed.push_span(offset + i, offset + end, Style::Link);
    Some(end)
}

/// `#tag`, `#nested/tag`.
///
/// The hash is styled, not hidden: it is what makes a tag recognisable as one,
/// and the chip background is drawn around it.
fn try_tag(line: &[char], i: usize, offset: usize, parsed: &mut Parsed) -> Option<usize> {
    if line.get(i) != Some(&'#') || !at_word_start(line, i) {
        return None;
    }
    // A digit first would make "#1 priority" and "#404" into tags, which is not
    // what anyone writing them meant.
    if !line.get(i + 1).is_some_and(|c| c.is_alphabetic()) {
        return None;
    }
    let mut end = i + 1;
    while line
        .get(end)
        .is_some_and(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '/'))
    {
        end += 1;
    }
    // "#project/" is a tag called "project" followed by a stray slash.
    while end > i + 1 && matches!(line[end - 1], '/' | '-') {
        end -= 1;
    }
    parsed.push_span(offset + i, offset + end, Style::Tag);
    Some(end)
}

/// Bold, strikethrough and italic. Two-character delimiters are tried first, or
/// `**x**` would read as an empty italic followed by stray asterisks.
fn try_emphasis(line: &[char], i: usize, offset: usize, parsed: &mut Parsed) -> Option<usize> {
    const DELIMITERS: [(&[char], Style); 5] = [
        (&['*', '*'], Style::Bold),
        (&['_', '_'], Style::Bold),
        (&['~', '~'], Style::Strikethrough),
        (&['*'], Style::Italic),
        (&['_'], Style::Italic),
    ];

    for (delimiter, style) in DELIMITERS {
        if !line[i..].starts_with(delimiter) {
            continue;
        }
        let content_start = i + delimiter.len();
        let Some(close) = find(line, content_start, delimiter) else {
            continue;
        };
        if close == content_start {
            continue; // "****" is not emphasis.
        }
        // Delimiters must hug their content, as in Markdown proper: an opener
        // may not be followed by a space, nor a closer preceded by one. Without
        // this, prose like "a * b * c" silently turns into italics, and any
        // note using asterisks as separators reformats itself.
        let opens = line.get(content_start).is_some_and(|c| !c.is_whitespace());
        let closes = line.get(close - 1).is_some_and(|c| !c.is_whitespace());
        if !opens || !closes {
            continue;
        }
        parsed.push_marker(offset + i, offset + content_start);
        parsed.push_span(offset + content_start, offset + close, style);
        parsed.push_marker(offset + close, offset + close + delimiter.len());
        return Some(close + delimiter.len());
    }
    None
}

/// Whether `i` begins a word, so that `C#` and `foo#bar` are not tags and a URL
/// glued to the end of a word is not a link.
fn at_word_start(line: &[char], i: usize) -> bool {
    match i.checked_sub(1).and_then(|prev| line.get(prev)) {
        None => true,
        Some(c) => c.is_whitespace() || matches!(c, '(' | '[' | '{' | '"' | '\'' | '>'),
    }
}

/// Index of the next occurrence of `needle` at or after `from`.
pub(super) fn find(line: &[char], from: usize, needle: &[char]) -> Option<usize> {
    (from..line.len().saturating_sub(needle.len() - 1))
        .find(|&index| line[index..].starts_with(needle))
}
