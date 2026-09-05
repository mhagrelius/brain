#include "scanner.h"

#include <QHash>
#include <QStringList>

#include <algorithm>

namespace brain::md {

namespace {

bool isSpaceChar(QChar c) { return c.isSpace(); }

// Index of the next occurrence of `needle` at or after `from`, or -1.
int find(QStringView line, int from, QStringView needle) {
    const int last = line.size() - needle.size();
    for (int i = from; i <= last; ++i)
        if (line.mid(i, needle.size()) == needle) return i;
    return -1;
}

QStringView skipSpaces(QStringView line) {
    int i = 0;
    while (i < line.size() && line[i] == u' ') ++i;
    return line.mid(i);
}

// A fence: three backticks or three tildes, optionally with a language.
bool isFence(QStringView line) {
    const QStringView t = skipSpaces(line);
    return t.startsWith(u"```") || t.startsWith(u"~~~");
}

// Exactly `---`, the frontmatter delimiter.
bool isDelimiter(QStringView line) { return line.trimmed() == u"---" && !line.startsWith(u' '); }

// A thematic break: three or more of `-`, `*` or `_` and nothing else.
bool isRule(QStringView line) {
    QString t;
    for (QChar c : line) if (!c.isSpace()) t.append(c);
    if (t.size() < 3) return false;
    const QChar first = t[0];
    if (first != u'-' && first != u'*' && first != u'_') return false;
    return std::all_of(t.begin(), t.end(), [first](QChar c) { return c == first; });
}

// A row of a table: `| a | b |`. A leading pipe is required.
bool isTableRow(QStringView line) {
    const QStringView t = skipSpaces(line);
    if (t.isEmpty() || t[0] != u'|') return false;
    return std::count(t.begin(), t.end(), QChar(u'|')) >= 2;
}

// The row under a table's header: `|---|:--:|`.
bool isTableDelimiter(QStringView line) {
    if (!isTableRow(line)) return false;
    QString text = line.trimmed().toString();
    while (text.startsWith(u'|')) text.remove(0, 1);
    while (text.endsWith(u'|')) text.chop(1);
    const QStringList cells = text.split(u'|');
    if (cells.isEmpty()) return false;
    for (const QString &raw : cells) {
        const QString cell = raw.trimmed();
        if (!cell.contains(u'-')) return false;
        for (QChar c : cell) if (c != u'-' && c != u':') return false;
    }
    return true;
}

int headingLevel(QStringView rest) {
    int hashes = 0;
    while (hashes < rest.size() && rest[hashes] == u'#') ++hashes;
    if (hashes >= 1 && hashes <= 6 && hashes < rest.size() && rest[hashes] == u' ') return hashes;
    return 0;
}

// `[ ] ` or `[x] ` immediately after a bullet: (ticked, length 3) or length 0.
int checkbox(QStringView after, bool *ticked) {
    if (after.size() < 4 || after[0] != u'[' || after[2] != u']' || after[3] != u' ') return 0;
    if (after[1] == u' ') { *ticked = false; return 3; }
    if (after[1] == u'x' || after[1] == u'X') { *ticked = true; return 3; }
    return 0;
}

bool atWordStart(QStringView line, int i) {
    if (i == 0) return true;
    const QChar c = line[i - 1];
    return c.isSpace() || c == u'(' || c == u'[' || c == u'{' || c == u'"' || c == u'\'' || c == u'>';
}

void inlineScan(QStringView line, int offset, Parsed &parsed);

void tableRow(QStringView chars, int offset, Parsed &parsed) {
    parsed.pushSpan(offset, offset + chars.size(), Style::TableRow);
    inlineScan(chars, offset, parsed);
}

void content(QStringView line, int offset, ListLevels &list, Parsed &parsed) {
    if (line.isEmpty()) return;  // a blank line separates items but does not end the list
    const int wholeStart = offset, wholeEnd = offset + line.size();

    int indent = 0;
    while (indent < line.size() && line[indent] == u' ') ++indent;
    const QStringView rest = line.mid(indent);

    const int itemLen = bulletLen(rest) ? bulletLen(rest) : orderedLen(rest);
    if (itemLen == 0 && indent == 0) list.clear();

    int contentStart = indent;
    bool hasBlock = false;
    Style blockStyle = Style::Quote;
    int blockLevel = 0;

    if (const int level = headingLevel(rest)) {
        const int markerLen = level + 1;
        parsed.pushMarker(offset + indent, offset + indent + markerLen, wholeStart, wholeEnd);
        contentStart = indent + markerLen;
        hasBlock = true; blockStyle = Style::Heading; blockLevel = level;
    } else if (rest.startsWith(u'>')) {
        const int markerLen = (rest.size() > 1 && rest[1] == u' ') ? 2 : 1;
        parsed.pushMarker(offset + indent, offset + indent + markerLen, wholeStart, wholeEnd);
        contentStart = indent + markerLen;
        hasBlock = true; blockStyle = Style::Quote;
    } else if (itemLen > 0) {
        // The bullet stays visible; the spaces in front of it are syntax.
        parsed.pushMarker(offset, offset + indent, wholeStart, wholeEnd);
        blockLevel = list.depth(indent);
        hasBlock = true; blockStyle = Style::ListItem;
        const QStringView after = rest.mid(itemLen);
        bool ticked = false;
        if (const int len = checkbox(after, &ticked)) {
            parsed.pushSpan(offset + indent + itemLen, offset + indent + itemLen + len, Style::Task, 0, ticked);
            contentStart = indent + itemLen + len;
        } else {
            contentStart = indent + itemLen;
        }
    }

    contentStart = std::min(contentStart, int(line.size()));
    if (hasBlock) {
        const int start = contentStart < line.size() ? contentStart : indent;
        parsed.pushSpan(offset + start, offset + line.size(), blockStyle, blockLevel);
    }
    inlineScan(line.mid(contentStart), offset + contentStart, parsed);
}

// ---- inline ---------------------------------------------------------------

int tryCode(QStringView line, int i, int offset, Parsed &parsed) {
    if (line[i] != u'`') return -1;
    const int close = find(line, i + 1, u"`");
    if (close < 0) return -1;
    parsed.pushMarker(offset + i, offset + i + 1, offset + i, offset + close + 1);
    parsed.pushSpan(offset + i + 1, offset + close, Style::Code);
    parsed.pushMarker(offset + close, offset + close + 1, offset + i, offset + close + 1);
    return close + 1;
}

int tryEmbed(QStringView line, int i, int offset, Parsed &parsed) {
    if (!line.mid(i).startsWith(u"![[")) return -1;
    const int close = find(line, i + 3, u"]]");
    if (close < 0 || close == i + 3) return -1;
    const int rs = offset + i, re = offset + close + 2;
    parsed.pushMarker(offset + i, offset + i + 3, rs, re);
    parsed.pushMarker(offset + i + 3, offset + close, rs, re);
    parsed.pushMarker(offset + close, offset + close + 2, rs, re);
    parsed.pushSpan(offset + i + 3, offset + close, Style::Embed);
    return close + 2;
}

int tryWikiLink(QStringView line, int i, int offset, Parsed &parsed) {
    if (!line.mid(i).startsWith(u"[[")) return -1;
    const int close = find(line, i + 2, u"]]");
    if (close < 0 || close == i + 2) return -1;
    int displayStart = i + 2;
    const int pipe = find(line.left(close), i + 2, u"|");
    if (pipe >= 0) displayStart = pipe + 1;
    if (displayStart >= close) return -1;
    const int rs = offset + i, re = offset + close + 2;
    parsed.pushMarker(offset + i, offset + displayStart, rs, re);
    parsed.pushSpan(offset + displayStart, offset + close, Style::WikiLink);
    parsed.pushMarker(offset + close, offset + close + 2, rs, re);
    return close + 2;
}

int tryLink(QStringView line, int i, int offset, Parsed &parsed) {
    if (line[i] != u'[') return -1;
    const int labelEnd = find(line, i + 1, u"]");
    if (labelEnd < 0 || labelEnd == i + 1) return -1;
    if (labelEnd + 1 >= line.size() || line[labelEnd + 1] != u'(') return -1;
    const int close = find(line, labelEnd + 2, u")");
    if (close < 0) return -1;
    const int rs = offset + i, re = offset + close + 1;
    parsed.pushMarker(offset + i, offset + i + 1, rs, re);
    parsed.pushSpan(offset + i + 1, offset + labelEnd, Style::Link);
    parsed.pushMarker(offset + labelEnd, offset + close + 1, rs, re);
    return close + 1;
}

int tryUrl(QStringView line, int i, int offset, Parsed &parsed) {
    if (!atWordStart(line, i)) return -1;
    const QStringView rest = line.mid(i);
    if (!rest.startsWith(u"http")) return -1;
    const bool https = rest.startsWith(u"https://");
    if (!https && !rest.startsWith(u"http://")) return -1;
    int end = i;
    while (end < line.size() && !line[end].isSpace()) ++end;
    while (end > i) {
        const QChar c = line[end - 1];
        if (c == u'.' || c == u',' || c == u';' || c == u':' || c == u'!' || c == u'?' || c == u')' || c == u']') --end;
        else break;
    }
    const int schemeLen = https ? 8 : 7;
    if (end <= i + schemeLen) return -1;
    parsed.pushSpan(offset + i, offset + end, Style::Link);
    return end;
}

int tryTag(QStringView line, int i, int offset, Parsed &parsed) {
    if (line[i] != u'#' || !atWordStart(line, i)) return -1;
    if (i + 1 >= line.size() || !line[i + 1].isLetter()) return -1;
    int end = i + 1;
    while (end < line.size()) {
        const QChar c = line[end];
        if (c.isLetterOrNumber() || c == u'-' || c == u'_' || c == u'/') ++end;
        else break;
    }
    while (end > i + 1 && (line[end - 1] == u'/' || line[end - 1] == u'-')) --end;
    parsed.pushSpan(offset + i, offset + end, Style::Tag);
    return end;
}

int tryBothEmphases(QStringView line, int i, int offset, Parsed &parsed) {
    const QChar d = line[i];
    if (d != u'*' && d != u'_') return -1;
    const QString run(3, d);
    if (!line.mid(i).startsWith(run)) return -1;
    const int contentStart = i + 3;
    const int close = find(line, contentStart, run);
    if (close < 0 || close == contentStart) return -1;
    const bool opens = contentStart < line.size() && !line[contentStart].isSpace();
    const bool closes = close > 0 && !line[close - 1].isSpace();
    if (!opens || !closes) return -1;
    const int rs = offset + i, re = offset + close + 3;
    parsed.pushMarker(offset + i, offset + contentStart, rs, re);
    parsed.pushSpan(offset + contentStart, offset + close, Style::Bold);
    parsed.pushSpan(offset + contentStart, offset + close, Style::Italic);
    parsed.pushMarker(offset + close, offset + close + 3, rs, re);
    return close + 3;
}

int tryEmphasis(QStringView line, int i, int offset, Parsed &parsed) {
    struct D { QStringView delim; Style style; };
    static const D delimiters[] = {{u"**", Style::Bold}, {u"__", Style::Bold}, {u"~~", Style::Strikethrough}, {u"*", Style::Italic}, {u"_", Style::Italic}};
    for (const D &d : delimiters) {
        if (!line.mid(i).startsWith(d.delim)) continue;
        const int contentStart = i + d.delim.size();
        const int close = find(line, contentStart, d.delim);
        if (close < 0 || close == contentStart) continue;
        const bool opens = contentStart < line.size() && !line[contentStart].isSpace();
        const bool closes = close > 0 && !line[close - 1].isSpace();
        if (!opens || !closes) continue;
        const int rs = offset + i, re = offset + close + d.delim.size();
        parsed.pushMarker(offset + i, offset + contentStart, rs, re);
        parsed.pushSpan(offset + contentStart, offset + close, d.style);
        parsed.pushMarker(offset + close, offset + close + d.delim.size(), rs, re);
        return close + d.delim.size();
    }
    return -1;
}

// Each try returns the index to resume from, always greater than i.
void inlineScan(QStringView line, int offset, Parsed &parsed) {
    int i = 0;
    while (i < line.size()) {
        int next = tryCode(line, i, offset, parsed);
        if (next < 0) next = tryEmbed(line, i, offset, parsed);
        if (next < 0) next = tryWikiLink(line, i, offset, parsed);
        if (next < 0) next = tryLink(line, i, offset, parsed);
        if (next < 0) next = tryUrl(line, i, offset, parsed);
        if (next < 0) next = tryTag(line, i, offset, parsed);
        if (next < 0) next = tryBothEmphases(line, i, offset, parsed);
        if (next < 0) next = tryEmphasis(line, i, offset, parsed);
        i = next < 0 ? i + 1 : next;
    }
}

// Scan one line and report the state the next line begins in.
LineState scanLine(QStringView chars, int offset, LineState state, bool first, std::optional<QStringView> next, ListLevels &list, Parsed &parsed) {
    switch (state) {
    case LineState::Frontmatter:
        parsed.pushSpan(offset, offset + chars.size(), Style::Frontmatter);
        return (!first && isDelimiter(chars)) ? LineState::Normal : LineState::Frontmatter;
    case LineState::Fence:
        if (isFence(chars)) {
            parsed.pushMarker(offset, offset + chars.size(), offset, offset + chars.size());
            return LineState::Normal;
        }
        parsed.pushSpan(offset, offset + chars.size(), Style::CodeBlock);
        return LineState::Fence;
    case LineState::Table:
        if (isTableDelimiter(chars)) {
            parsed.pushSpan(offset, offset + chars.size(), Style::TableDelimiter);
            return LineState::Table;
        }
        if (isTableRow(chars)) {
            tableRow(chars, offset, parsed);
            return LineState::Table;
        }
        return scanLine(chars, offset, LineState::Normal, false, next, list, parsed);
    case LineState::Normal:
        if (isFence(chars)) {
            parsed.pushMarker(offset, offset + chars.size(), offset, offset + chars.size());
            return LineState::Fence;
        }
        if (isRule(chars)) {
            parsed.pushSpan(offset, offset + chars.size(), Style::Rule);
            return LineState::Normal;
        }
        if (isTableRow(chars) && next && isTableDelimiter(*next)) {
            tableRow(chars, offset, parsed);
            return LineState::Table;
        }
        content(chars, offset, list, parsed);
        return LineState::Normal;
    }
    return LineState::Normal;
}

bool startsFrontmatter(QStringView text) {
    int end = text.indexOf(u'\n');
    if (end < 0) end = text.size();
    return text.left(end).trimmed() == u"---" && !text.startsWith(u' ');
}

QString cleanTarget(QStringView s) { return s.trimmed().toString(); }

} // namespace

int bulletLen(QStringView line) {
    if (line.size() >= 2 && (line[0] == u'-' || line[0] == u'*' || line[0] == u'+') && line[1] == u' ') return 2;
    return 0;
}

int orderedLen(QStringView line) {
    int digits = 0;
    while (digits < line.size() && line[digits].isDigit() && line[digits].unicode() < 128) ++digits;
    if (digits == 0 || digits + 1 >= line.size()) return 0;
    const QChar p = line[digits];
    if ((p == u'.' || p == u')') && line[digits + 1] == u' ') return digits + 2;
    return 0;
}

int ListLevels::depth(int indent) {
    const quint8 ind = quint8(std::min(indent, 255));
    while (len > 0 && widths[len - 1] > ind) --len;
    const bool known = len > 0 && widths[len - 1] == ind;
    if (!known && len < int(sizeof(widths))) { widths[len] = ind; ++len; }
    return std::min(int(len) - 1 < 0 ? 0 : int(len) - 1, MAX_LIST_DEPTH);
}

void Parsed::pushSpan(int start, int end, Style style, int level, bool ticked) {
    if (end > start) spans.append(Span{start, end, style, level, ticked});
}

void Parsed::pushMarker(int start, int end, int revealStart, int revealEnd) {
    if (end > start) markers.append(Marker{start, end, revealStart, revealEnd});
}

Parsed parse(const QString &text) {
    Parsed parsed;
    const QStringView all(text);
    int lineStart = 0;
    LineState state = startsFrontmatter(all) ? LineState::Frontmatter : LineState::Normal;
    ListLevels list;
    bool first = true;
    for (;;) {
        int lineEnd = all.indexOf(u'\n', lineStart);
        if (lineEnd < 0) lineEnd = all.size();
        std::optional<QStringView> next;
        if (lineEnd < all.size()) {
            const int s = lineEnd + 1;
            int e = all.indexOf(u'\n', s);
            if (e < 0) e = all.size();
            next = all.mid(s, e - s);
        }
        parsed.lineStates.append(state);
        parsed.lineLists.append(list);
        state = scanLine(all.mid(lineStart, lineEnd - lineStart), lineStart, state, first, next, list, parsed);
        first = false;
        if (lineEnd >= all.size()) break;
        lineStart = lineEnd + 1;
    }
    return parsed;
}

QString strip(const QString &text) { return stripWith(text, parse(text)); }

QString stripWith(const QString &text, const Parsed &parsed) {
    const int n = text.size();
    QVector<bool> hidden(n, false), inRow(n, false);
    auto hide = [&](int from, int to) { for (int i = std::max(0, from); i < std::min(to, n); ++i) hidden[i] = true; };
    for (const Marker &m : parsed.markers) hide(m.start, m.end);
    for (const Span &s : parsed.spans)
        if (s.style == Style::Frontmatter || s.style == Style::Rule || s.style == Style::TableDelimiter) hide(s.start, s.end);
    int lineStart = 0;
    const QStringView all(text);
    while (lineStart < n) {
        int lineEnd = all.indexOf(u'\n', lineStart);
        if (lineEnd < 0) lineEnd = n;
        const QStringView line = all.mid(lineStart, lineEnd - lineStart);
        int indent = 0;
        while (indent < line.size() && line[indent] == u' ') ++indent;
        const QStringView rest = line.mid(indent);
        const int len = bulletLen(rest) ? bulletLen(rest) : orderedLen(rest);
        if (len) hide(lineStart, lineStart + indent + len);
        lineStart = lineEnd + 1;
    }
    for (const Span &s : parsed.spans) {
        if (s.style != Style::TableRow) continue;
        for (int at = s.start; at < std::min(s.end, n); ++at) {
            inRow[at] = true;
            if (text[at] == u'|') hidden[at] = true;
        }
    }
    QString out;
    out.reserve(n);
    for (int at = 0; at < n; ++at) {
        if (hidden[at]) continue;
        if (inRow[at] && text[at] == u' ' && out.endsWith(u' ')) continue;
        out.append(text[at]);
    }
    return out;
}

Extracted extract(const QString &text) { return extractWith(text, parse(text)); }

Extracted extractWith(const QString &text, const Parsed &parsed) {
    QHash<int, int> opener;  // marker end → marker start
    for (const Marker &m : parsed.markers) opener.insert(m.end, m.start);
    const int n = text.size();
    auto slice = [&](int from, int to) { from = std::min(from, n); to = std::min(to, n); return to > from ? text.mid(from, to - from) : QString(); };

    Extracted out;
    for (const Span &span : parsed.spans) {
        if (span.style == Style::WikiLink) {
            const int start = opener.value(span.start, span.start);
            const QString shown = slice(span.start, span.end);
            QString piped = slice(start, span.start);
            while (piped.startsWith(u'[')) piped.remove(0, 1);
            while (piped.endsWith(u'|')) piped.chop(1);
            piped = piped.trimmed();
            QString target;
            std::optional<QString> display;
            if (piped.isEmpty()) target = shown.trimmed();
            else { target = piped; display = shown; }
            if (target.isEmpty()) continue;
            out.links.append(WikiLink{target, display, start, span.end + 2});
        } else if (span.style == Style::Embed) {
            const int start = opener.value(span.start, span.start);
            const QString inner = slice(span.start, span.end);
            QString target;
            std::optional<QString> display;
            const int pipe = inner.indexOf(u'|');
            if (pipe >= 0) { target = inner.left(pipe).trimmed(); display = inner.mid(pipe + 1).trimmed(); }
            else target = inner.trimmed();
            if (target.isEmpty()) continue;
            out.embeds.append(WikiLink{target, display, start, span.end + 2});
        } else if (span.style == Style::Tag) {
            QString name = slice(span.start, span.end);
            while (name.startsWith(u'#')) name.remove(0, 1);
            if (name.isEmpty()) continue;
            out.tags.append(TagRef{name, span.start, span.end});
        }
    }
    return out;
}

std::optional<QString> rewriteTarget(const QString &body, const QString &from, const QString &to) {
    auto matches = [&](const QString &raw) {
        const QString target = raw.trimmed();
        if (target.compare(from, Qt::CaseInsensitive) == 0) return true;
        if (target.compare(from + QStringLiteral(".md"), Qt::CaseInsensitive) == 0) return true;
        const QString tail = target.section(u'/', -1);
        return tail.compare(from, Qt::CaseInsensitive) == 0;
    };
    const Extracted extracted = extract(body);
    struct E { int start; int end; QString replacement; };
    QVector<E> edits;
    auto consider = [&](const WikiLink &link, bool embed) {
        if (!matches(link.target)) return;
        const QString prefix = embed ? QStringLiteral("![[") : QStringLiteral("[[");
        const QString replacement = link.display ? prefix + to + u'|' + *link.display + QStringLiteral("]]") : prefix + to + QStringLiteral("]]");
        edits.append(E{link.start, link.end, replacement});
    };
    for (const WikiLink &l : extracted.links) consider(l, false);
    for (const WikiLink &l : extracted.embeds) consider(l, true);
    if (edits.isEmpty()) return std::nullopt;
    std::sort(edits.begin(), edits.end(), [](const E &a, const E &b) { return a.start < b.start; });
    QString out;
    out.reserve(body.size());
    int at = 0;
    const int n = body.size();
    for (const E &e : edits) {
        const int s = std::min(e.start, n);
        if (s > at) out.append(body.mid(at, s - at));
        out.append(e.replacement);
        at = std::min(e.end, n);
    }
    out.append(body.mid(at));
    return out;
}

std::optional<ListEnter> listEnter(const QString &line) {
    const QStringView all(line);
    int indent = 0;
    while (indent < all.size() && all[indent] == u' ') ++indent;
    const QStringView rest = all.mid(indent);
    const int markerLen = bulletLen(rest) ? bulletLen(rest) : orderedLen(rest);
    if (markerLen == 0) return std::nullopt;
    const QStringView after = rest.mid(markerLen);
    bool ticked = false;
    int boxLen = 0;
    QString box;
    if (checkbox(after, &ticked)) { boxLen = 4; box = QStringLiteral("[ ] "); }
    const QStringView tail = after.mid(std::min(boxLen, int(after.size())));
    if (std::all_of(tail.begin(), tail.end(), isSpaceChar)) return ListEnter{true, QString()};
    QString prefix(indent, u' ');
    if (bulletLen(rest)) {
        prefix.append(rest[0]);
    } else {
        int digits = 0;
        while (digits < rest.size() && rest[digits].isDigit()) ++digits;
        bool ok = false;
        const quint32 n = rest.left(digits).toString().toUInt(&ok);
        const quint32 next = ok ? (n == 0xFFFFFFFFu ? n : n + 1) : 1;
        prefix.append(QString::number(next));
        prefix.append(rest[digits]);
    }
    prefix.append(u' ');
    prefix.append(box);
    return ListEnter{false, prefix};
}

namespace {
struct Level { int width; std::optional<quint32> counter; };
std::optional<quint32> *counterAt(QVector<Level> &levels, int indent) {
    while (!levels.isEmpty() && levels.last().width > indent) levels.removeLast();
    if (levels.isEmpty() || levels.last().width != indent) levels.append(Level{indent, std::nullopt});
    return &levels.last().counter;
}
} // namespace

QVector<Renumber> renumber(const QString &text) {
    QVector<Renumber> edits;
    QVector<Level> levels;
    const QStringView all(text);
    int lineStart = 0;
    bool inFence = false;
    while (lineStart <= all.size()) {
        int lineEnd = all.indexOf(u'\n', lineStart);
        if (lineEnd < 0) lineEnd = all.size();
        const QStringView line = all.mid(lineStart, lineEnd - lineStart);
        int indent = 0;
        while (indent < line.size() && line[indent] == u' ') ++indent;
        const QStringView rest = line.mid(indent);
        if (isFence(line)) {
            inFence = !inFence;
        } else if (inFence || line.isEmpty()) {
        } else if (bulletLen(rest)) {
            *counterAt(levels, indent) = std::nullopt;
        } else if (orderedLen(rest)) {
            int digits = 0;
            while (digits < rest.size() && rest[digits].isDigit()) ++digits;
            bool ok = false;
            const quint32 w = rest.left(digits).toString().toUInt(&ok);
            std::optional<quint32> written = ok ? std::optional<quint32>(w) : std::nullopt;
            std::optional<quint32> *counter = counterAt(levels, indent);
            if (*counter) {
                const quint32 expected = **counter;
                if (written != expected) edits.append(Renumber{lineStart + indent, lineStart + indent + digits, expected});
                *counter = expected == 0xFFFFFFFFu ? expected : expected + 1;
            } else if (written) {
                *counter = *written == 0xFFFFFFFFu ? *written : *written + 1;
            }
        } else if (indent == 0) {
            levels.clear();
        }
        if (lineEnd >= all.size()) break;
        lineStart = lineEnd + 1;
    }
    return edits;
}

const QVector<Format> &allFormats() {
    static const QVector<Format> all = {Format::Bold, Format::Italic, Format::Strikethrough, Format::Code, Format::Heading1, Format::Heading2, Format::Heading3, Format::Quote, Format::Bullet, Format::Task, Format::WikiLink, Format::Link, Format::CodeBlock, Format::Table, Format::Rule};
    return all;
}

Edit editFor(Format f) {
    auto wrap = [](const QString &m) { Edit e; e.kind = Edit::Wrap; e.before = m; e.after = m; return e; };
    auto prefix = [](const QString &p) { Edit e; e.kind = Edit::Prefix; e.prefix = p; return e; };
    auto block = [](const QString &t, int caret) { Edit e; e.kind = Edit::Block; e.text = t; e.caret = caret; return e; };
    switch (f) {
    case Format::Bold: return wrap(QStringLiteral("**"));
    case Format::Italic: return wrap(QStringLiteral("*"));
    case Format::Strikethrough: return wrap(QStringLiteral("~~"));
    case Format::Code: return wrap(QStringLiteral("`"));
    case Format::WikiLink: { Edit e; e.kind = Edit::Wrap; e.before = QStringLiteral("[["); e.after = QStringLiteral("]]"); return e; }
    case Format::Link: { Edit e; e.kind = Edit::Wrap; e.before = QStringLiteral("["); e.after = QStringLiteral("](https://)"); return e; }
    case Format::Heading1: return prefix(QStringLiteral("# "));
    case Format::Heading2: return prefix(QStringLiteral("## "));
    case Format::Heading3: return prefix(QStringLiteral("### "));
    case Format::Quote: return prefix(QStringLiteral("> "));
    case Format::Bullet: return prefix(QStringLiteral("- "));
    case Format::Task: return prefix(QStringLiteral("- [ ] "));
    case Format::CodeBlock: return block(QStringLiteral("```\n\n```\n"), 4);
    case Format::Table: return block(QStringLiteral("| Column | Column |\n|--------|--------|\n|        |        |\n"), 2);
    case Format::Rule: return block(QStringLiteral("---\n"), 4);
    }
    return wrap(QString());
}

QString labelOf(Format f) {
    switch (f) {
    case Format::Bold: return QStringLiteral("Bold");
    case Format::Italic: return QStringLiteral("Italic");
    case Format::Strikethrough: return QStringLiteral("Strikethrough");
    case Format::Code: return QStringLiteral("Code");
    case Format::Heading1: return QStringLiteral("Heading 1");
    case Format::Heading2: return QStringLiteral("Heading 2");
    case Format::Heading3: return QStringLiteral("Heading 3");
    case Format::Quote: return QStringLiteral("Quote");
    case Format::Bullet: return QStringLiteral("List");
    case Format::Task: return QStringLiteral("Task");
    case Format::WikiLink: return QStringLiteral("Link to Note");
    case Format::Link: return QStringLiteral("Web Link");
    case Format::CodeBlock: return QStringLiteral("Code Block");
    case Format::Table: return QStringLiteral("Table");
    case Format::Rule: return QStringLiteral("Separator");
    }
    return QString();
}

QString syntaxOf(Format f) {
    switch (f) {
    case Format::Bold: return QStringLiteral("**text**");
    case Format::Italic: return QStringLiteral("*text*");
    case Format::Strikethrough: return QStringLiteral("~~text~~");
    case Format::Code: return QStringLiteral("`code`");
    case Format::Heading1: return QStringLiteral("# ");
    case Format::Heading2: return QStringLiteral("## ");
    case Format::Heading3: return QStringLiteral("### ");
    case Format::Quote: return QStringLiteral("> ");
    case Format::Bullet: return QStringLiteral("- ");
    case Format::Task: return QStringLiteral("- [ ] ");
    case Format::WikiLink: return QStringLiteral("[[Note]]");
    case Format::Link: return QStringLiteral("[text](…)");
    case Format::CodeBlock: return QStringLiteral("```");
    case Format::Table: return QStringLiteral("| a | b |");
    case Format::Rule: return QStringLiteral("---");
    }
    return QString();
}

QString keybindOf(Format f) {
    switch (f) {
    case Format::Bold: return QStringLiteral("ctrl+b");
    case Format::Italic: return QStringLiteral("ctrl+i");
    case Format::Code: return QStringLiteral("ctrl+`");
    case Format::Task: return QStringLiteral("ctrl+shift+t");
    case Format::WikiLink: return QStringLiteral("ctrl+l");
    default: return QString();
    }
}

std::optional<Style> styleOf(Format f) {
    switch (f) {
    case Format::Bold: return Style::Bold;
    case Format::Italic: return Style::Italic;
    case Format::Strikethrough: return Style::Strikethrough;
    case Format::Code: return Style::Code;
    case Format::WikiLink: return Style::WikiLink;
    case Format::Link: return Style::Link;
    default: return std::nullopt;
    }
}

} // namespace brain::md
