use super::*;

/// The text a span covers, for readable assertions.
fn text_of(source: &str, span: &Span) -> String {
    source
        .chars()
        .skip(span.start)
        .take(span.end - span.start)
        .collect()
}

fn styles(parsed: &Parsed) -> Vec<Style> {
    parsed.spans.iter().map(|s| s.style).collect()
}

/// What the note looks like with the markers hidden — the reading view.
fn rendered(source: &str) -> String {
    let parsed = parse(source);
    source
        .chars()
        .enumerate()
        .filter(|(index, _)| {
            !parsed
                .markers
                .iter()
                .any(|m| *index >= m.start && *index < m.end)
        })
        .map(|(_, c)| c)
        .collect()
}

// ---- inherited from Stickies: the subset that already worked ----

#[test]
fn plain_text_is_left_alone() {
    let parsed = parse("just a note");
    assert!(parsed.spans.is_empty());
    assert!(parsed.markers.is_empty());
    assert_eq!(rendered("just a note"), "just a note");
}

#[test]
fn empty_input_does_not_panic() {
    assert!(parse("").spans.is_empty());
    assert!(parse("\n\n\n").spans.is_empty());
}

#[test]
fn bold_hides_its_asterisks() {
    let source = "buy **oat milk** today";
    let parsed = parse(source);
    assert_eq!(styles(&parsed), vec![Style::Bold]);
    assert_eq!(text_of(source, &parsed.spans[0]), "oat milk");
    assert_eq!(rendered(source), "buy oat milk today");
}

#[test]
fn italic_and_strikethrough() {
    let source = "*soon* and ~~later~~";
    let parsed = parse(source);
    assert_eq!(styles(&parsed), vec![Style::Italic, Style::Strikethrough]);
    assert_eq!(rendered(source), "soon and later");
}

#[test]
fn bold_wins_over_italic_for_double_asterisks() {
    assert_eq!(styles(&parse("**x**")), vec![Style::Bold]);
}

#[test]
fn headings_are_levelled_and_their_hashes_hidden() {
    for level in 1..=6u8 {
        let source = format!("{} Title", "#".repeat(level as usize));
        let parsed = parse(&source);
        assert_eq!(styles(&parsed), vec![Style::Heading(level)]);
        assert_eq!(text_of(&source, &parsed.spans[0]), "Title");
        assert_eq!(rendered(&source), "Title");
    }
}

#[test]
fn seven_hashes_is_not_a_heading() {
    assert_eq!(rendered("####### nope"), "####### nope");
    assert!(!styles(&parse("####### nope")).contains(&Style::Heading(6)));
}

#[test]
fn bullets_stay_visible_because_they_are_the_rendering() {
    let source = "- milk\n- bread";
    let parsed = parse(source);
    assert_eq!(
        styles(&parsed),
        vec![Style::ListItem(0), Style::ListItem(0)]
    );
    assert_eq!(text_of(source, &parsed.spans[0]), "milk");
    assert_eq!(rendered(source), source);
}

#[test]
fn an_asterisk_bullet_is_not_italic() {
    assert_eq!(styles(&parse("* milk")), vec![Style::ListItem(0)]);
}

#[test]
fn numbered_lists_are_recognised() {
    assert_eq!(styles(&parse("1. first")), vec![Style::ListItem(0)]);
    assert_eq!(styles(&parse("12) twelfth")), vec![Style::ListItem(0)]);
    assert!(parse("1.no space").spans.is_empty());
}

#[test]
fn quotes_hide_their_marker() {
    let source = "> to be fair";
    assert_eq!(styles(&parse(source)), vec![Style::Quote]);
    assert_eq!(rendered(source), "to be fair");
}

#[test]
fn inline_code_hides_its_backticks() {
    let source = "run `cargo test` first";
    let parsed = parse(source);
    assert_eq!(styles(&parsed), vec![Style::Code]);
    assert_eq!(text_of(source, &parsed.spans[0]), "cargo test");
    assert_eq!(rendered(source), "run cargo test first");
}

#[test]
fn formatting_inside_code_is_literal() {
    let source = "`**not bold**`";
    let parsed = parse(source);
    assert_eq!(styles(&parsed), vec![Style::Code]);
    assert_eq!(text_of(source, &parsed.spans[0]), "**not bold**");
}

#[test]
fn fenced_blocks_style_their_contents_and_hide_the_fences() {
    let source = "```\nlet x = 1;\nlet y = 2;\n```";
    let parsed = parse(source);
    assert_eq!(styles(&parsed), vec![Style::CodeBlock, Style::CodeBlock]);
    assert_eq!(text_of(source, &parsed.spans[0]), "let x = 1;");
    assert_eq!(rendered(source), "\nlet x = 1;\nlet y = 2;\n");
}

#[test]
fn links_show_the_label_and_hide_the_target() {
    let source = "see [the docs](https://example.com) later";
    let parsed = parse(source);
    assert_eq!(styles(&parsed), vec![Style::Link]);
    assert_eq!(text_of(source, &parsed.spans[0]), "the docs");
    assert_eq!(rendered(source), "see the docs later");
}

#[test]
fn unmatched_markers_are_left_as_plain_text() {
    // Half-typed formatting is the normal state while writing, and must never
    // swallow the rest of the note.
    for source in [
        "**unfinished",
        "a * b * c *",
        "`unclosed",
        "[label](unclosed",
        "[label] (spaced)",
        "[[unclosed wikilink",
        "![[unclosed embed",
        "5 * 3 * 2",
    ] {
        assert_eq!(
            rendered(source),
            source,
            "{source:?} hid characters it should not have"
        );
    }
}

#[test]
fn asterisks_used_as_punctuation_do_not_italicise() {
    for source in ["a * b * c", "5 * 3 * 2 = 30", "note *"] {
        assert_eq!(rendered(source), source, "{source:?} was reformatted");
        assert!(parse(source).spans.is_empty(), "{source:?} was styled");
    }
    assert_eq!(rendered("2 * 3 and *this*"), "2 * 3 and this");
}

#[test]
fn an_unclosed_fence_does_not_swallow_the_note() {
    let source = "```\nstill visible";
    assert_eq!(rendered(source), "\nstill visible");
    assert_eq!(styles(&parse(source)), vec![Style::CodeBlock]);
}

#[test]
fn offsets_are_characters_not_bytes() {
    // The bug this guards: byte offsets would land mid-codepoint and either
    // panic in GtkTextBuffer or style the wrong characters.
    let source = "héllo **wörld** 🎉";
    let parsed = parse(source);
    assert_eq!(text_of(source, &parsed.spans[0]), "wörld");
    assert_eq!(rendered(source), "héllo wörld 🎉");

    let emoji = "🎉🎉 **b** 🎉";
    assert_eq!(text_of(emoji, &parse(emoji).spans[0]), "b");

    let tagged = "🎉 #café and [[Wörld]]";
    let parsed = parse(tagged);
    assert_eq!(text_of(tagged, &parsed.spans[0]), "#café");
    assert_eq!(text_of(tagged, &parsed.spans[1]), "Wörld");
}

#[test]
fn every_span_and_marker_is_within_the_text() {
    let source = "---\ntags: [a]\n---\n\n# Title\n\n- [x] **bold** item\n- `code` and \
                  [[Link|shown]]\n\n> quote #tag\n\n![[img.png]]\n\n---\n\n```\nfn x() {}\n```";
    let parsed = parse(source);
    let len = source.chars().count();
    for span in &parsed.spans {
        assert!(span.start < span.end && span.end <= len, "{span:?}");
    }
    for marker in &parsed.markers {
        assert!(marker.start < marker.end && marker.end <= len, "{marker:?}");
    }
}

#[test]
fn markers_never_overlap_each_other() {
    // Overlapping hidden ranges would double-count and could hide content.
    let source = "# **T** and `c` and [l](u)\n- *i* [[W|x]] ![[f]] #t";
    let mut markers = parse(source).markers;
    markers.sort_by_key(|m| m.start);
    for pair in markers.windows(2) {
        assert!(
            pair[0].end <= pair[1].start,
            "markers overlap: {:?} and {:?}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn parsing_is_stable_under_incremental_typing() {
    // Every prefix of a note gets typed at some point; none may panic.
    let source =
        "---\ntags: [a]\n---\n# T\n- [ ] **b** `c` [l](u) [[W|s]] ![[f]] #tag\n```\nx\n```";
    for length in 0..=source.chars().count() {
        let prefix: String = source.chars().take(length).collect();
        let parsed = parse(&prefix);
        let len = prefix.chars().count();
        assert!(parsed.spans.iter().all(|s| s.end <= len), "{prefix:?}");
        assert!(parsed.markers.iter().all(|m| m.end <= len), "{prefix:?}");
    }
}

// ---- strip ----

#[test]
fn strip_removes_markup_for_snippets() {
    assert_eq!(strip("# Heading Goes Here"), "Heading Goes Here");
    assert_eq!(strip("**bold** and *italic*"), "bold and italic");
    assert_eq!(strip("`code` here"), "code here");
    assert_eq!(strip("~~gone~~"), "gone");
    assert_eq!(strip("see [the docs](https://example.com)"), "see the docs");
    assert_eq!(strip("> quoted"), "quoted");
    assert_eq!(strip("- milk"), "milk");
    assert_eq!(strip("1. first"), "first");
}

#[test]
fn strip_keeps_what_a_reader_would_keep() {
    // A wikilink reads as its display text; a tag reads as itself.
    assert_eq!(strip("see [[Borrow checker]]"), "see Borrow checker");
    assert_eq!(strip("see [[Borrow checker|it]]"), "see it");
    assert_eq!(strip("about #rust today"), "about #rust today");
}

#[test]
fn strip_drops_frontmatter_and_rules() {
    let source = "---\ntags: [rust]\n---\n\n# Ownership\n\n---\n\nProse.";
    let stripped = strip(source);
    assert!(!stripped.contains("tags"), "{stripped:?} kept frontmatter");
    assert!(!stripped.contains("---"), "{stripped:?} kept a rule");
    assert!(stripped.contains("Ownership") && stripped.contains("Prose."));
}

#[test]
fn strip_leaves_plain_text_and_partial_markup_alone() {
    for source in [
        "just a note",
        "a * b * c",
        "**unfinished",
        "5 * 3",
        "C# notes",
    ] {
        assert_eq!(strip(source), source, "{source:?}");
    }
}

#[test]
fn strip_is_multibyte_safe() {
    assert_eq!(strip("# 🎉 Héllo **wörld**"), "🎉 Héllo wörld");
}

// ---- wikilinks, embeds, tags: the notebook syntax ----

#[test]
fn a_wikilink_hides_its_brackets() {
    let source = "see [[Borrow checker]] first";
    let parsed = parse(source);
    assert_eq!(styles(&parsed), vec![Style::WikiLink]);
    assert_eq!(text_of(source, &parsed.spans[0]), "Borrow checker");
    assert_eq!(rendered(source), "see Borrow checker first");
}

#[test]
fn a_piped_wikilink_shows_only_the_display_text() {
    let source = "see [[Borrow checker|the checker]] first";
    let parsed = parse(source);
    assert_eq!(styles(&parsed), vec![Style::WikiLink]);
    assert_eq!(text_of(source, &parsed.spans[0]), "the checker");
    assert_eq!(rendered(source), "see the checker first");
}

#[test]
fn an_embed_keeps_its_filename_legible() {
    // The picture is drawn beneath the line; an image with no indication of
    // which file it came from cannot be edited deliberately.
    let source = "![[diagram.png]]";
    let parsed = parse(source);
    assert_eq!(styles(&parsed), vec![Style::Embed]);
    assert_eq!(text_of(source, &parsed.spans[0]), "diagram.png");
    assert_eq!(rendered(source), "diagram.png");
}

#[test]
fn an_embed_is_not_read_as_a_link_first() {
    // "![[x]]" starts with "[" one character in, so ordering decides this.
    assert_eq!(styles(&parse("![[x.png]]")), vec![Style::Embed]);
    assert_eq!(styles(&parse("[[x]]")), vec![Style::WikiLink]);
}

#[test]
fn empty_brackets_link_nowhere() {
    for source in ["[[]]", "![[]]", "[[Target|]]"] {
        assert_eq!(rendered(source), source, "{source:?}");
    }
}

#[test]
fn tags_style_the_hash_because_it_is_part_of_the_chip() {
    let source = "about #rust and #project/brain today";
    let parsed = parse(source);
    assert_eq!(styles(&parsed), vec![Style::Tag, Style::Tag]);
    assert_eq!(text_of(source, &parsed.spans[0]), "#rust");
    assert_eq!(text_of(source, &parsed.spans[1]), "#project/brain");
    assert_eq!(rendered(source), source);
}

#[test]
fn things_that_look_like_tags_but_are_not() {
    // Each of these is something people write and none of them means a tag.
    for source in ["C# is a language", "issue #404", "a#b", "# Heading", "#"] {
        assert!(
            !styles(&parse(source)).contains(&Style::Tag),
            "{source:?} became a tag"
        );
    }
}

#[test]
fn a_tag_stops_before_trailing_punctuation() {
    let source = "about #rust.";
    let parsed = parse(source);
    assert_eq!(text_of(source, &parsed.spans[0]), "#rust");

    let slashed = "see #project/ here";
    assert_eq!(text_of(slashed, &parse(slashed).spans[0]), "#project");
}

#[test]
fn a_hash_inside_code_is_not_a_tag() {
    assert_eq!(styles(&parse("`#rust`")), vec![Style::Code]);
    assert_eq!(
        styles(&parse("```\n#rust\n```")),
        vec![Style::CodeBlock],
        "a tag in a fence is code, not a tag"
    );
}

#[test]
fn task_checkboxes_are_reported_with_their_state() {
    let source = "- [ ] milk\n- [x] bread\n- [X] jam";
    let parsed = parse(source);
    assert_eq!(
        styles(&parsed),
        vec![
            Style::Task(false),
            Style::ListItem(0),
            Style::Task(true),
            Style::ListItem(0),
            Style::Task(true),
            Style::ListItem(0),
        ]
    );
    assert_eq!(text_of(source, &parsed.spans[0]), "[ ]");
    // Checkboxes stay visible: the UI makes them clickable in place.
    assert_eq!(rendered(source), source);
}

#[test]
fn a_bracket_after_a_bullet_is_not_always_a_checkbox() {
    assert!(!styles(&parse("- [link](u) here")).contains(&Style::Task(false)));
    assert!(!styles(&parse("- [?] unknown")).contains(&Style::Task(false)));
}

#[test]
fn bare_urls_are_linked_without_hiding_anything() {
    let source = "see https://example.com/a_b for more";
    let parsed = parse(source);
    assert_eq!(styles(&parsed), vec![Style::Link]);
    assert_eq!(text_of(source, &parsed.spans[0]), "https://example.com/a_b");
    assert_eq!(rendered(source), source);
}

#[test]
fn a_url_does_not_swallow_the_sentence_that_follows_it() {
    let source = "see https://example.com.";
    assert_eq!(
        text_of(source, &parse(source).spans[0]),
        "https://example.com"
    );
}

#[test]
fn thematic_breaks_are_recognised_but_a_bullet_is_not_one() {
    // Below the first line, because a leading "---" is frontmatter.
    for rule in ["---", "***", "___", "- - -"] {
        let source = format!("Prose.\n{rule}");
        assert_eq!(styles(&parse(&source)), vec![Style::Rule], "{rule:?}");
    }
    assert_eq!(styles(&parse("- milk")), vec![Style::ListItem(0)]);
    assert!(!styles(&parse("Prose.\n--")).contains(&Style::Rule));
}

#[test]
fn a_note_that_is_only_a_delimiter_is_unterminated_frontmatter() {
    // Documented consequence of "--- on line one opens frontmatter". It is
    // also what you are looking at for one keystroke while typing frontmatter.
    assert_eq!(parse("---").line_states, vec![LineState::Frontmatter]);
}

// ---- frontmatter as a block ----

#[test]
fn frontmatter_at_the_top_is_one_recessed_block() {
    let source = "---\ntags: [rust]\n---\n\n# Title";
    let parsed = parse(source);
    assert_eq!(
        parsed.line_states,
        vec![
            LineState::Frontmatter,
            LineState::Frontmatter,
            LineState::Frontmatter,
            LineState::Normal,
            LineState::Normal,
        ]
    );
    assert_eq!(
        styles(&parsed),
        vec![
            Style::Frontmatter,
            Style::Frontmatter,
            Style::Frontmatter,
            Style::Heading(1),
        ]
    );
}

#[test]
fn a_rule_below_the_first_line_does_not_open_frontmatter() {
    // Otherwise every horizontal rule near the top would turn the prose under
    // it into metadata.
    let source = "Prose.\n\n---\n\nMore prose.";
    let parsed = parse(source);
    assert!(!parsed.line_states.contains(&LineState::Frontmatter));
    assert_eq!(styles(&parsed), vec![Style::Rule]);
}

#[test]
fn unterminated_frontmatter_does_not_style_the_whole_note_as_prose() {
    let parsed = parse("---\ntags: [a]\nstill metadata");
    assert!(parsed
        .line_states
        .iter()
        .all(|s| *s == LineState::Frontmatter));
}

// ---- the incremental path ----

#[test]
fn scanning_one_line_matches_scanning_the_whole_note() {
    // The editor's per-keystroke path must not disagree with a full re-scan,
    // or styling drifts as you type and only a reload fixes it.
    let source = "# Title\nsome **bold** and [[a link]]\n- [x] done #tag\n\
                  - nested below\n    - deeper still\n- back out\n\
                  | a | b |\n|---|---|\n| 1 | **2** |\n\n> quoted";
    let whole = parse(source);

    let lines: Vec<&str> = source.split('\n').collect();
    let mut offset = 0usize;
    let mut state = LineState::Normal;
    let mut list = ListLevels::default();
    let mut spans = Vec::new();
    let mut markers = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let (parsed, next, next_list) =
            scan_line(line, offset, state, list, lines.get(index + 1).copied());
        spans.extend(parsed.spans);
        markers.extend(parsed.markers);
        state = next;
        list = next_list;
        offset += line.chars().count() + 1;
    }

    assert_eq!(spans, whole.spans);
    assert_eq!(markers, whole.markers);
}

#[test]
fn a_line_reports_the_state_the_next_one_begins_in() {
    // This is what tells the editor whether one line is enough.
    assert_eq!(
        scan_line("plain", 0, LineState::Normal, ListLevels::default(), None).1,
        LineState::Normal
    );
    assert_eq!(
        scan_line("```", 0, LineState::Normal, ListLevels::default(), None).1,
        LineState::Fence
    );
    assert_eq!(
        scan_line("code", 0, LineState::Fence, ListLevels::default(), None).1,
        LineState::Fence
    );
    assert_eq!(
        scan_line("```", 0, LineState::Fence, ListLevels::default(), None).1,
        LineState::Normal
    );
    assert_eq!(
        scan_line(
            "---",
            0,
            LineState::Frontmatter,
            ListLevels::default(),
            None
        )
        .1,
        LineState::Normal
    );
}

// ---- extraction for the index ----

#[test]
fn extraction_finds_links_embeds_and_tags() {
    let extracted = extract("See [[Borrow checker]] and [[Rust|it]].\n\n![[d.png]] #rust #a/b");

    assert_eq!(
        extracted
            .links
            .iter()
            .map(|l| (l.target.as_str(), l.display.as_deref()))
            .collect::<Vec<_>>(),
        vec![("Borrow checker", None), ("Rust", Some("it"))]
    );
    assert_eq!(
        extracted
            .embeds
            .iter()
            .map(|e| e.target.as_str())
            .collect::<Vec<_>>(),
        vec!["d.png"]
    );
    assert_eq!(
        extracted
            .tags
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        vec!["rust", "a/b"]
    );
}

#[test]
fn extraction_reports_ranges_that_cover_the_whole_link() {
    // The editor uses these to decide what a click landed on and what a rename
    // has to rewrite, so they must include the brackets.
    let source = "see [[Target|shown]] here";
    let link = &extract(source).links[0];
    let text: String = source
        .chars()
        .skip(link.start)
        .take(link.end - link.start)
        .collect();
    assert_eq!(text, "[[Target|shown]]");
}

#[test]
fn extraction_ignores_links_inside_code() {
    // One definition of what a link is: if the scanner did not style it, it is
    // not an edge in the graph either.
    let extracted = extract("`[[Not a link]]` and\n```\n[[Nor this]] #nor-this\n```");
    assert!(extracted.links.is_empty(), "{:?}", extracted.links);
    assert!(extracted.tags.is_empty(), "{:?}", extracted.tags);
}

#[test]
fn an_embed_size_hint_is_not_part_of_the_filename() {
    let embed = &extract("![[diagram.png|300]]").embeds[0];
    assert_eq!(embed.target, "diagram.png");
    assert_eq!(embed.display.as_deref(), Some("300"));
}

// ---- tables ----

#[test]
fn a_table_needs_a_delimiter_row_to_be_a_table() {
    // Without the lookahead, every sentence containing a pipe becomes one.
    let source = "| a | b |\n|---|---|\n| 1 | 2 |";
    assert_eq!(
        styles(&parse(source)),
        vec![Style::TableRow, Style::TableDelimiter, Style::TableRow]
    );

    for prose in ["| a | b |", "yes | no", "| a | b |\nplain text"] {
        assert!(
            !styles(&parse(prose)).contains(&Style::TableRow),
            "{prose:?} became a table"
        );
    }
}

#[test]
fn table_pipes_stay_visible_because_they_are_the_table() {
    // A text view cannot draw column rules.
    let source = "| a | b |\n|---|---|\n| 1 | 2 |";
    assert_eq!(rendered(source), source);
}

#[test]
fn cells_are_styled_inline() {
    let source = "| a | b |\n|---|---|\n| **bold** | [[Link]] |";
    let parsed = parse(source);
    assert!(styles(&parsed).contains(&Style::Bold));
    assert!(styles(&parsed).contains(&Style::WikiLink));
    assert_eq!(rendered(source), "| a | b |\n|---|---|\n| bold | Link |");
}

#[test]
fn alignment_colons_are_part_of_the_delimiter_row() {
    let source = "| a | b | c |\n|:--|:-:|--:|\n| 1 | 2 | 3 |";
    assert_eq!(
        styles(&parse(source)),
        vec![Style::TableRow, Style::TableDelimiter, Style::TableRow]
    );
}

#[test]
fn a_table_ends_at_the_first_line_that_is_not_a_row() {
    let source = "| a |\n|---|\n| 1 |\n\n# After";
    let parsed = parse(source);
    assert_eq!(
        parsed.line_states,
        vec![
            LineState::Normal,
            LineState::Table,
            LineState::Table,
            LineState::Table,
            LineState::Normal,
        ]
    );
    assert!(styles(&parsed).contains(&Style::Heading(1)));
}

#[test]
fn a_table_inside_a_fence_is_code() {
    let source = "```\n| a | b |\n|---|---|\n```";
    assert_eq!(
        styles(&parse(source)),
        vec![Style::CodeBlock, Style::CodeBlock]
    );
}

#[test]
fn a_delimiter_row_alone_is_not_a_table() {
    // Half-typed markup, which is the normal state while writing one.
    assert!(!styles(&parse("|---|---|")).contains(&Style::TableDelimiter));
}

#[test]
fn a_table_reads_as_its_cells_in_a_snippet() {
    // Padding that lined the columns up would otherwise leave gaping runs of
    // spaces mid-sentence.
    let source = "| Name   | Role     |\n|--------|----------|\n| Ada    | Engineer |";
    assert_eq!(strip(source), " Name Role \n\n Ada Engineer ");
}

#[test]
fn tables_are_multibyte_safe() {
    let source = "| 🎉 | café |\n|---|---|\n| **wörld** | b |";
    let parsed = parse(source);
    let bold = parsed
        .spans
        .iter()
        .find(|span| span.style == Style::Bold)
        .expect("bold");
    assert_eq!(text_of(source, bold), "wörld");
}

// ---- nested lists ----

fn levels(source: &str) -> Vec<u8> {
    parse(source)
        .spans
        .iter()
        .filter_map(|span| match span.style {
            Style::ListItem(level) => Some(level),
            _ => None,
        })
        .collect()
}

#[test]
fn nesting_is_counted_from_the_indents_a_note_actually_uses() {
    // Not from a fixed number of spaces per level: two-space and four-space
    // notes both nest one level at a time.
    assert_eq!(levels("- a\n  - b\n    - c"), [0, 1, 2]);
    assert_eq!(levels("- a\n    - b\n        - c"), [0, 1, 2]);
    // And a note that mixes them still nests monotonically.
    assert_eq!(levels("- a\n  - b\n      - c\n  - d\n- e"), [0, 1, 2, 1, 0]);
}

#[test]
fn nesting_stops_deepening_past_the_styled_maximum() {
    // The scan stays honest about the structure; the styling stops indenting.
    let source = "- a\n - b\n  - c\n   - d\n    - e\n     - f\n      - g";
    let deepest = *levels(source).iter().max().expect("levels");
    assert_eq!(deepest, MAX_LIST_DEPTH);
}

#[test]
fn a_blank_line_separates_items_without_ending_the_list() {
    assert_eq!(levels("- a\n  - b\n\n  - c"), [0, 1, 1]);
}

#[test]
fn prose_at_the_left_edge_ends_the_list() {
    // Otherwise the next list in the note keeps the previous one's nesting.
    assert_eq!(levels("- a\n  - b\nProse.\n  - c"), [0, 1, 0]);
}

#[test]
fn an_indented_line_continues_the_item_above_it() {
    // A wrapped continuation is not a new list and must not reset the nesting.
    assert_eq!(levels("- a\n  - b\n    continued\n  - c"), [0, 1, 1]);
}

#[test]
fn the_indent_before_a_bullet_is_syntax() {
    // The level's margin does the indenting now; leaving the spaces in would
    // indent a nested item twice over.
    let source = "- a\n  - b";
    assert_eq!(rendered(source), "- a\n- b");
    // The bullet itself still shows: a text view has no glyph to put in its
    // place.
    assert!(rendered(source).contains("- b"));
}

#[test]
fn a_nested_task_keeps_both_its_level_and_its_checkbox() {
    let source = "- [ ] top\n  - [x] nested";
    let parsed = parse(source);
    assert_eq!(
        styles(&parsed),
        vec![
            Style::Task(false),
            Style::ListItem(0),
            Style::Task(true),
            Style::ListItem(1),
        ]
    );
}

#[test]
fn ordered_and_bulleted_items_nest_the_same_way() {
    assert_eq!(levels("1. a\n  2. b\n    3. c"), [0, 1, 2]);
    assert_eq!(levels("- a\n  1. b"), [0, 1]);
}
