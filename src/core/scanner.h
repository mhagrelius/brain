#pragma once

// A Markdown scanner for live-styled editing. A port of `quill`, the scanner
// the GTK app used: a note is always shown as source and always styled, and
// the syntax characters are hidden everywhere except in the construct holding
// the caret. So as well as *what* is styled this reports exactly *which*
// characters are syntax, so they can be hidden. Offsets are QString (UTF-16)
// indices, which is what QTextDocument positions are.

#include <QString>
#include <QStringView>
#include <QVector>

#include <optional>

namespace brain::md {

enum class Style {
    Heading,       // level 1–6 in Span::level
    Bold,
    Italic,
    Strikethrough,
    Code,          // `inline`
    CodeBlock,     // a line inside a ``` fence
    Quote,
    ListItem,      // nesting depth in Span::level, from zero
    Link,          // the visible text of [text](url), or a bare URL
    WikiLink,      // the visible text of [[Target]] or [[Target|shown]]
    Embed,         // ![[attachment]]
    Tag,           // #tag, hash included
    Task,          // the [ ] or [x]; Span::ticked
    Rule,
    TableRow,      // pipes stay visible
    TableDelimiter,
    Frontmatter,
    Comment,       // <!-- an HTML comment on a line of its own -->
};

struct Span {
    int start = 0;
    int end = 0;
    Style style = Style::Bold;
    int level = 0;
    bool ticked = false;
    bool operator==(const Span &) const = default;
};

// A run of characters that is syntax, hidden unless the cursor is inside the
// construct it belongs to. Reveal bounds are inclusive at both ends so a caret
// resting immediately before the opener or after the closer still shows it.
struct Marker {
    int start = 0;
    int end = 0;
    int revealStart = 0;
    int revealEnd = 0;
    bool revealedBy(int cursor) const { return cursor >= revealStart && cursor <= revealEnd; }
    bool operator==(const Marker &) const = default;
};

constexpr int MAX_LIST_DEPTH = 4;

// The indent widths of the list the scanner is currently inside, so two-space
// and four-space notes both nest one level at a time.
struct ListLevels {
    quint8 widths[MAX_LIST_DEPTH + 2] = {};
    quint8 len = 0;
    int depth(int indent);
    void clear() { len = 0; }
    bool operator==(const ListLevels &o) const {
        if (len != o.len) return false;
        for (int i = 0; i < len; ++i) if (widths[i] != o.widths[i]) return false;
        return true;
    }
};

enum class LineState { Normal, Fence, Frontmatter, Table };

struct Parsed {
    QVector<Span> spans;
    QVector<Marker> markers;
    QVector<LineState> lineStates;  // the state each line begins in; size = line count
    QVector<ListLevels> lineLists;
    void pushSpan(int start, int end, Style style, int level = 0, bool ticked = false);
    void pushMarker(int start, int end, int revealStart, int revealEnd);
};

Parsed parse(const QString &text);
QString strip(const QString &text);
QString stripWith(const QString &text, const Parsed &parsed);

struct WikiLink {
    QString target;
    std::optional<QString> display;
    int start = 0;
    int end = 0;
};
struct TagRef {
    QString name;
    int start = 0;
    int end = 0;
};
struct Extracted {
    QVector<WikiLink> links;
    QVector<WikiLink> embeds;
    QVector<TagRef> tags;
};
Extracted extract(const QString &text);
Extracted extractWith(const QString &text, const Parsed &parsed);
// Rewrite every link pointing at `from` so it points at `to`; nullopt when
// nothing matched, so a rename skips the notes it does not affect.
std::optional<QString> rewriteTarget(const QString &body, const QString &from, const QString &to);

struct ListEnter {
    bool endList = false;
    QString prefix;
};
std::optional<ListEnter> listEnter(const QString &line);

struct Renumber {
    int start = 0;
    int end = 0;
    quint32 number = 0;
};
QVector<Renumber> renumber(const QString &text);

// A formatting action the UI can apply. What each one means is the scanner's
// business, so a menu never writes syntax the editor does not style.
enum class Format { Bold, Italic, Strikethrough, Code, Heading1, Heading2, Heading3, Quote, Bullet, Task, WikiLink, Link, CodeBlock, Table, Rule };

struct Edit {
    enum Kind { Wrap, Prefix, Block } kind = Wrap;
    QString before;   // Wrap
    QString after;    // Wrap
    QString prefix;   // Prefix
    QString text;     // Block
    int caret = 0;    // Block
};

const QVector<Format> &allFormats();
Edit editFor(Format format);
QString labelOf(Format format);
QString syntaxOf(Format format);
QString keybindOf(Format format);
std::optional<Style> styleOf(Format format);

// Low-level helpers shared with the editor.
int bulletLen(QStringView rest);
int orderedLen(QStringView rest);

} // namespace brain::md
