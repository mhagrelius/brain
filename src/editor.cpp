#include "editor.h"

#include "palette.h"

#include <QDateTime>
#include <QDir>
#include <QFileInfo>
#include <QImageReader>
#include <QTextBlock>
#include <QTextCursor>
#include <QTimer>

#include <algorithm>

using namespace brain;

namespace {

int clampTo(int v, int hi) { return std::max(0, std::min(v, hi)); }

// The construct a caret at `cursor` is typing a [[link into: the offset of
// the `[[`, or -1. Only on the caret's line, and not once it has been closed.
int openLinkStart(const QString &text, int cursor) {
    const int lineStart = text.lastIndexOf(u'\n', cursor - 1 < 0 ? 0 : cursor - 1) + 1;
    const int open = text.lastIndexOf(QStringLiteral("[["), cursor - 2 < 0 ? 0 : cursor - 2);
    if (open < lineStart || open < 0) return -1;
    if (cursor - open < 2) return -1;
    const QString between = text.mid(open + 2, cursor - open - 2);
    if (between.contains(QStringLiteral("]]")) || between.contains(u'\n')) return -1;
    return open;
}

} // namespace

Editor::Editor(Palette *palette, QObject *parent) : QObject(parent), m_palette(palette) {
    m_toSource = {0};
    m_toDisplay = {0};
}

void Editor::setDocument(QQuickTextDocument *document) {
    if (m_document) disconnect(m_document, nullptr, this, nullptr);
    m_quickDocument = document;
    m_document = document ? document->textDocument() : nullptr;
    if (m_document) {
        m_document->setUndoRedoEnabled(false);
        connect(m_document, &QTextDocument::contentsChange, this, &Editor::onContentsChange);
    }
    emit documentChanged();
    rebuild(true);
}

void Editor::setTextEdit(QQuickItem *item) {
    m_textEdit = item;
    emit documentChanged();
    syncCaret();
}

void Editor::setReading(bool reading) {
    if (m_reading == reading) return;
    m_reading = reading;
    emit readingChanged();
    rebuild();
}

void Editor::setContentWidth(qreal width) {
    if (qFuzzyCompare(m_contentWidth, width)) return;
    m_contentWidth = width;
    rebuild();
}

void Editor::setVaultRoot(const QString &root) {
    if (m_vaultRoot == root) return;
    m_vaultRoot = root;
    m_imageSizes.clear();
    rebuild();
}

int Editor::wordCount() const {
    int count = 0;
    bool inWord = false;
    for (QChar c : md::strip(m_source)) {
        if (c.isSpace()) inWord = false;
        else if (!inWord) { inWord = true; ++count; }
    }
    return count;
}

void Editor::load(const QString &body, int caret) {
    m_source = body;
    m_caret = clampTo(caret, m_source.size());
    m_undo.clear();
    m_redo.clear();
    emit sourceChanged();
    rebuild(true);
    updateCompletion();
}

int Editor::sourceOffset(int displayPosition) const {
    return m_toSource.value(clampTo(displayPosition, m_toSource.size() - 1), m_source.size());
}

int Editor::displayOffset(int sourceOffset) const {
    return m_toDisplay.value(clampTo(sourceOffset, m_toDisplay.size() - 1), m_display.size());
}

QVector<Editor::Hidden> Editor::hiddenRanges() const {
    QVector<Hidden> out;
    for (const md::Marker &m : m_parsed.markers) {
        if (!m_reading && m.revealedBy(m_caret)) continue;
        // An embed's filename stays on show: the picture beneath names the
        // file, but a file that is missing, or not a picture, still has to
        // say what it is. Only the brackets hide.
        bool filename = false;
        for (const md::Span &s : m_parsed.spans) if (s.style == md::Style::Embed && s.start == m.start && s.end == m.end) { filename = true; break; }
        if (filename) continue;
        out.append(Hidden{m.start, m.end});
    }
    std::sort(out.begin(), out.end(), [](const Hidden &a, const Hidden &b) { return a.start < b.start; });
    QVector<Hidden> merged;
    for (const Hidden &h : out) {
        if (!merged.isEmpty() && h.start <= merged.last().end) merged.last().end = std::max(merged.last().end, h.end);
        else merged.append(h);
    }
    return merged;
}

bool Editor::isImage(const QString &name) const {
    const QString ext = QFileInfo(name).suffix().toLower();
    return ext == u"png" || ext == u"jpg" || ext == u"jpeg" || ext == u"gif" || ext == u"webp" || ext == u"svg" || ext == u"bmp";
}

void Editor::refreshHidden() {
    m_parsed = md::parse(m_source);
    m_hidden = hiddenRanges();
}

// Build the display and the maps from m_hidden — the ranges the document
// reflects right now, which is not always what the caret would hide.
void Editor::recompute() {
    m_parsed = md::parse(m_source);
    const QVector<Hidden> &hidden = m_hidden;
    const int n = m_source.size();
    m_display.clear();
    m_display.reserve(n);
    m_toSource.resize(0);
    m_toDisplay.resize(n + 1);
    int h = 0;
    for (int si = 0; si < n; ++si) {
        while (h < hidden.size() && hidden[h].end <= si) ++h;
        const bool isHidden = h < hidden.size() && si >= hidden[h].start && si < hidden[h].end;
        m_toDisplay[si] = m_display.size();
        if (isHidden) continue;
        m_display.append(m_source[si]);
        m_toSource.append(si);
    }
    m_toSource.append(n);
    m_toDisplay[n] = m_display.size();

    // Decorations: contiguous quote, table and code lines, in display coords.
    m_decorations.clear();
    m_embeds.clear();
    int lineStart = 0;
    int lineIndex = 0;
    QString runKind;
    int runFrom = 0, runTo = 0;
    auto flushRun = [&]() {
        if (!runKind.isEmpty()) m_decorations.append(QVariantMap{{QStringLiteral("kind"), runKind}, {QStringLiteral("from"), runFrom}, {QStringLiteral("to"), runTo}});
        runKind.clear();
    };
    const md::Extracted extracted = md::extractWith(m_source, m_parsed);
    while (lineStart <= n) {
        int lineEnd = m_source.indexOf(u'\n', lineStart);
        if (lineEnd < 0) lineEnd = n;
        QString kind;
        for (const md::Span &s : m_parsed.spans) {
            if (s.end <= lineStart || s.start >= lineEnd + 1) continue;
            if (s.start > lineEnd) continue;
            if (s.style == md::Style::Quote) kind = QStringLiteral("quote");
            else if (s.style == md::Style::TableRow || s.style == md::Style::TableDelimiter) kind = QStringLiteral("table");
            else if (s.style == md::Style::CodeBlock) kind = QStringLiteral("code");
        }
        if (kind.isEmpty() && lineIndex < m_parsed.lineStates.size()) {
            const md::LineState state = m_parsed.lineStates[lineIndex];
            const bool nextInFence = lineIndex + 1 < m_parsed.lineStates.size() && m_parsed.lineStates[lineIndex + 1] == md::LineState::Fence;
            if (state == md::LineState::Fence || nextInFence) kind = QStringLiteral("code");
        }
        if (kind != runKind) { flushRun(); if (!kind.isEmpty()) { runKind = kind; runFrom = m_toDisplay[lineStart]; } }
        if (!kind.isEmpty()) runTo = m_toDisplay[lineEnd];
        // Embedded images sit under the line naming them.
        for (const md::WikiLink &embed : extracted.embeds) {
            if (embed.start < lineStart || embed.start > lineEnd) continue;
            if (!isImage(embed.target)) continue;
            const QString path = m_vaultRoot + u'/' + QLatin1String(ATTACHMENTS_DIR) + u'/' + embed.target;
            QSize size = m_imageSizes.value(path);
            if (!size.isValid()) { size = QImageReader(path).size(); m_imageSizes.insert(path, size); }
            if (!size.isValid() || size.isEmpty()) continue;
            const qreal scale = m_palette->textScale();
            qreal w = std::min(qreal(size.width()) * scale, std::max(m_contentWidth, 40.0));
            qreal hgt = w * size.height() / size.width();
            m_embeds.append(QVariantMap{{QStringLiteral("pos"), m_toDisplay[lineEnd]}, {QStringLiteral("line"), lineIndex}, {QStringLiteral("path"), path}, {QStringLiteral("name"), embed.target}, {QStringLiteral("width"), w}, {QStringLiteral("height"), hgt}});
        }
        if (lineEnd >= n) break;
        lineStart = lineEnd + 1;
        ++lineIndex;
    }
    flushRun();
}

QString Editor::documentText() const {
    if (!m_document) return QString();
    QString out;
    for (QTextBlock block = m_document->begin(); block.isValid(); block = block.next()) {
        if (block.blockNumber() > 0) out.append(u'\n');
        out.append(block.text());
    }
    return out;
}

void Editor::syncDocument(bool force) {
    m_syncPending = false;
    if (!m_document) { emit displayChanged(); return; }
    const QString current = documentText();
    const bool changed = current != m_display;
    if (changed) {
        int prefix = 0;
        const int max = std::min(current.size(), m_display.size());
        while (prefix < max && current[prefix] == m_display[prefix]) ++prefix;
        int suffix = 0;
        while (suffix < max - prefix && current[current.size() - 1 - suffix] == m_display[m_display.size() - 1 - suffix]) ++suffix;
        m_syncing = true;
        QTextCursor cursor(m_document);
        cursor.beginEditBlock();
        cursor.setPosition(prefix);
        cursor.setPosition(current.size() - suffix, QTextCursor::KeepAnchor);
        cursor.insertText(m_display.mid(prefix, m_display.size() - prefix - suffix));
        cursor.endEditBlock();
        m_syncing = false;
    }
    if (qEnvironmentVariableIsSet("BRAIN_DEBUG_EDITOR")) {
        const QString after = documentText();
        if (after != m_display) qWarning() << "editor: document" << after.size() << "display" << m_display.size() << "changed" << changed;
    }
    if (changed || force) applyFormats();
    syncCaret();
    emit displayChanged();
}

void Editor::rebuild(bool force) {
    refreshHidden();
    recompute();
    syncDocument(force);
}

void Editor::syncCaret() {
    if (!m_textEdit) return;
    const int dp = displayOffset(m_caret);
    const int have = m_textEdit->property("cursorPosition").toInt();
    if (qEnvironmentVariableIsSet("BRAIN_DEBUG_EDITOR")) qWarning() << "syncCaret caret" << m_caret << "want" << dp << "have" << have;
    if (have == dp) return;
    m_syncing = true;
    m_textEdit->setProperty("cursorPosition", dp);
    m_syncing = false;
    if (qEnvironmentVariableIsSet("BRAIN_DEBUG_EDITOR")) qWarning() << "  after set:" << m_textEdit->property("cursorPosition").toInt();
}

void Editor::cursorMoved(int displayPosition) {
    if (qEnvironmentVariableIsSet("BRAIN_DEBUG_EDITOR")) qWarning() << "cursorMoved" << displayPosition << "syncing" << m_syncing << "caret" << m_caret << "->" << sourceOffset(displayPosition);
    if (m_syncing) return;
    const int caret = sourceOffset(displayPosition);
    if (caret == m_caret) return;
    // The reveal set depends on the caret; rebuild only when it changed.
    m_caret = caret;
    m_parsed = md::parse(m_source);
    const QVector<Hidden> after = hiddenRanges();
    bool same = m_hidden.size() == after.size();
    for (int i = 0; same && i < after.size(); ++i) same = m_hidden[i].start == after[i].start && m_hidden[i].end == after[i].end;
    if (!same) rebuild();
    updateCompletion();
}

void Editor::onContentsChange(int position, int removed, int added) {
    if (m_syncing || !m_document) return;
    const int srcStart = sourceOffset(position);
    const int srcEnd = sourceOffset(position + removed);
    if (qEnvironmentVariableIsSet("BRAIN_DEBUG_EDITOR")) qWarning() << "contentsChange" << position << removed << added << "src" << srcStart << srcEnd;
    QTextCursor cursor(m_document);
    cursor.setPosition(position);
    cursor.setPosition(position + added, QTextCursor::KeepAnchor);
    QString inserted = cursor.selectedText();
    inserted.replace(QChar(0x2029), u'\n');
    pushUndo();
    const int removedSrc = std::max(0, srcEnd - srcStart);
    m_source.replace(srcStart, removedSrc, inserted);
    m_caret = srcStart + inserted.size();
    // Carry the document's hidden ranges across the edit, so the map keeps
    // describing the document as it is — the reveal set is only recomputed
    // when the document is patched to match it.
    const int delta = inserted.size() - removedSrc;
    QVector<Hidden> kept;
    for (const Hidden &h : m_hidden) {
        if (h.end <= srcStart) kept.append(h);
        else if (h.start >= srcEnd) kept.append(Hidden{h.start + delta, h.end + delta});
        // ranges inside the removed span went with it
    }
    m_hidden = kept;
    recompute();
    emit sourceChanged();
    emit edited();
    // Patching the document from inside its own change signal is asking for
    // trouble; the mapping is already right, so the display catches up on
    // the next turn of the loop.
    if (!m_syncPending) {
        m_syncPending = true;
        QTimer::singleShot(0, this, [this]() { rebuild(true); updateCompletion(); });
    }
}

void Editor::pushUndo() {
    const qint64 now = QDateTime::currentMSecsSinceEpoch();
    if (!m_undo.isEmpty() && now - m_lastEditMs < 700 && m_undo.last().source.size() > 0 && std::abs(m_undo.last().caret - m_caret) <= 2) {
        m_lastEditMs = now;
        return;
    }
    m_undo.append(Snapshot{m_source, m_caret});
    if (m_undo.size() > 200) m_undo.removeFirst();
    m_redo.clear();
    m_lastEditMs = now;
}

void Editor::undo() {
    if (m_undo.isEmpty()) return;
    m_redo.append(Snapshot{m_source, m_caret});
    const Snapshot s = m_undo.takeLast();
    m_source = s.source;
    m_caret = clampTo(s.caret, m_source.size());
    m_lastEditMs = 0;
    emit sourceChanged();
    emit edited();
    rebuild(true);
    updateCompletion();
}

void Editor::redo() {
    if (m_redo.isEmpty()) return;
    m_undo.append(Snapshot{m_source, m_caret});
    const Snapshot s = m_redo.takeLast();
    m_source = s.source;
    m_caret = clampTo(s.caret, m_source.size());
    emit sourceChanged();
    emit edited();
    rebuild(true);
    updateCompletion();
}

void Editor::replaceSource(int start, int end, const QString &text, int caretAfter) {
    pushUndo();
    m_lastEditMs = 0;
    start = clampTo(start, m_source.size());
    end = clampTo(end, m_source.size());
    m_source.replace(start, std::max(0, end - start), text);
    m_caret = clampTo(caretAfter, m_source.size());
    emit sourceChanged();
    emit edited();
    rebuild(true);
    updateCompletion();
}

QString Editor::lineOf(int sourceOffset, int *lineStart, int *lineEnd) const {
    const int at = clampTo(sourceOffset, m_source.size());
    const int start = m_source.lastIndexOf(u'\n', at - 1 < 0 ? 0 : at - 1) + 1;
    int end = m_source.indexOf(u'\n', at);
    if (end < 0) end = m_source.size();
    if (at == 0 && !m_source.isEmpty() && m_source[0] == u'\n') { end = 0; }
    if (lineStart) *lineStart = start;
    if (lineEnd) *lineEnd = end;
    return m_source.mid(start, end - start);
}

void Editor::applyRenumber() {
    const QVector<md::Renumber> edits = md::renumber(m_source);
    if (edits.isEmpty()) return;
    QString source = m_source;
    int caret = m_caret;
    for (int i = edits.size() - 1; i >= 0; --i) {
        const md::Renumber &e = edits[i];
        const QString number = QString::number(e.number);
        source.replace(e.start, e.end - e.start, number);
        if (caret >= e.end) caret += number.size() - (e.end - e.start);
    }
    if (source == m_source) return;
    m_source = source;
    m_caret = clampTo(caret, m_source.size());
    emit sourceChanged();
    emit edited();
    rebuild(true);
}

bool Editor::continueList(int displayPosition) {
    if (m_reading) return false;
    placeCaret(displayPosition);
    int lineStart = 0, lineEnd = 0;
    const QString line = lineOf(m_caret, &lineStart, &lineEnd);
    if (qEnvironmentVariableIsSet("BRAIN_DEBUG_EDITOR")) qWarning() << "continueList display" << displayPosition << "caret" << m_caret << "line" << line;
    const auto le = md::listEnter(line);
    if (!le) return false;
    // Enter inside the marker itself — before the content starts — moves the
    // item down a line rather than splicing a new marker into it.
    int indent = 0;
    while (indent < line.size() && line[indent] == u' ') ++indent;
    const QStringView rest(line.constData() + indent, line.size() - indent);
    const int markerLen = md::bulletLen(rest) ? md::bulletLen(rest) : md::orderedLen(rest);
    if (!le->endList && m_caret < lineStart + indent + markerLen) {
        int boxLen = 0;
        const QStringView after = rest.mid(markerLen);
        if (after.size() >= 4 && after[0] == u'[' && after[2] == u']' && after[3] == u' ') boxLen = 4;
        replaceSource(lineStart, lineStart, QStringLiteral("\n"), lineStart + 1 + indent + markerLen + boxLen);
        applyRenumber();
        return true;
    }
    if (le->endList) {
        replaceSource(lineStart, lineEnd, QString(), lineStart);
    } else {
        replaceSource(m_caret, m_caret, u'\n' + le->prefix, m_caret + 1 + le->prefix.size());
        applyRenumber();
    }
    return true;
}

bool Editor::backspaceBullet(int displayPosition) {
    if (m_reading) return false;
    placeCaret(displayPosition);
    int lineStart = 0, lineEnd = 0;
    const QString line = lineOf(m_caret, &lineStart, &lineEnd);
    int indent = 0;
    while (indent < line.size() && line[indent] == u' ') ++indent;
    const QStringView rest(line.constData() + indent, line.size() - indent);
    const int markerLen = md::bulletLen(rest) ? md::bulletLen(rest) : md::orderedLen(rest);
    if (markerLen == 0) return false;
    int boxLen = 0;
    const QStringView after = rest.mid(markerLen);
    if (after.size() >= 4 && after[0] == u'[' && after[2] == u']' && after[3] == u' ' && (after[1] == u' ' || after[1] == u'x' || after[1] == u'X')) boxLen = 4;
    const int contentStart = lineStart + indent + markerLen + boxLen;
    if (m_caret != contentStart) return false;
    if (!line.mid(indent + markerLen + boxLen).trimmed().isEmpty()) return false;
    replaceSource(lineStart + indent, contentStart, QString(), lineStart + indent);
    return true;
}

void Editor::applyFormat(const QString &label, int selectionStart, int selectionEnd) {
    if (m_reading) return;
    std::optional<md::Format> found;
    for (md::Format f : md::allFormats()) if (md::labelOf(f) == label) found = f;
    if (!found) return;
    const md::Edit edit = md::editFor(*found);
    int s = sourceOffset(std::min(selectionStart, selectionEnd));
    int e = sourceOffset(std::max(selectionStart, selectionEnd));
    if (selectionStart == selectionEnd) { placeCaret(selectionStart); s = e = m_caret; }

    if (edit.kind == md::Edit::Wrap) {
        // Already inside one of these: take the markers off rather than nesting.
        if (const auto style = md::styleOf(*found)) {
            for (const md::Span &span : m_parsed.spans) {
                if (span.style != *style) continue;
                const bool inside = s >= span.start && e <= span.end;
                if (!inside) continue;
                const md::Marker *opener = nullptr, *closer = nullptr;
                for (const md::Marker &m : m_parsed.markers) {
                    if (m.end == span.start && m.revealStart == m.start) opener = &m;
                    if (m.start == span.end && m.revealEnd == m.end) closer = &m;
                }
                if (!opener || !closer) break;
                const QString before = m_source.mid(opener->start, opener->end - opener->start);
                if (before != edit.before) break;
                QString source = m_source;
                source.remove(closer->start, closer->end - closer->start);
                source.remove(opener->start, opener->end - opener->start);
                const int caret = std::max(opener->start, m_caret - (opener->end - opener->start));
                pushUndo();
                m_lastEditMs = 0;
                m_source = source;
                m_caret = clampTo(caret, m_source.size());
                emit sourceChanged();
                emit edited();
                rebuild(true);
                return;
            }
        }
        const QString text = m_source.mid(s, e - s);
        replaceSource(s, e, edit.before + text + edit.after, s + edit.before.size() + text.size() + (text.isEmpty() ? 0 : edit.after.size()));
        return;
    }
    if (edit.kind == md::Edit::Prefix) {
        int firstStart = 0, lastEnd = 0;
        lineOf(s, &firstStart, nullptr);
        lineOf(std::max(s, e > s ? e - 1 : e), nullptr, &lastEnd);
        const QString block = m_source.mid(firstStart, lastEnd - firstStart);
        QStringList lines = block.split(u'\n');
        bool all = true;
        for (const QString &line : lines) if (!line.startsWith(edit.prefix)) { all = false; break; }
        QStringList out;
        for (const QString &line : lines) out.append(all ? line.mid(edit.prefix.size()) : edit.prefix + line);
        const int delta = all ? -edit.prefix.size() : edit.prefix.size();
        replaceSource(firstStart, lastEnd, out.join(u'\n'), std::max(firstStart, m_caret + delta));
        return;
    }
    // Block: on lines of its own.
    int lineStart = 0;
    lineOf(m_caret, &lineStart, nullptr);
    QString text = edit.text;
    int at = m_caret;
    int caretOffset = edit.caret;
    if (at != lineStart) { text = u'\n' + text; caretOffset += 1; }
    replaceSource(at, at, text, at + caretOffset);
}

QString Editor::linkAt(int displayPosition) const {
    const int sp = sourceOffset(displayPosition);
    for (const md::WikiLink &link : md::extractWith(m_source, m_parsed).links)
        if (sp >= link.start && sp <= link.end) return link.target;
    return QString();
}

QString Editor::urlAt(int displayPosition) const {
    const int sp = sourceOffset(displayPosition);
    for (const md::Span &span : m_parsed.spans) {
        if (span.style != md::Style::Link) continue;
        if (sp < span.start - 1 || sp > span.end + 1) continue;
        const QString text = m_source.mid(span.start, span.end - span.start);
        if (text.startsWith(QStringLiteral("http://")) || text.startsWith(QStringLiteral("https://"))) return text;
        // [label](url): the url sits in the closing marker.
        const int open = m_source.indexOf(u'(', span.end);
        const int close = open < 0 ? -1 : m_source.indexOf(u')', open);
        if (open == span.end + 1 && close > open) return m_source.mid(open + 1, close - open - 1).trimmed();
    }
    return QString();
}

QString Editor::tagAt(int displayPosition) const {
    const int sp = sourceOffset(displayPosition);
    for (const md::Span &span : m_parsed.spans)
        if (span.style == md::Style::Tag && sp >= span.start && sp <= span.end) {
            QString name = m_source.mid(span.start, span.end - span.start);
            while (name.startsWith(u'#')) name.remove(0, 1);
            return name;
        }
    return QString();
}

bool Editor::toggleTaskAt(int displayPosition) {
    const int sp = sourceOffset(displayPosition);
    for (const md::Span &span : m_parsed.spans) {
        if (span.style != md::Style::Task) continue;
        if (sp < span.start - 2 || sp > span.end + 1) continue;
        const QString box = span.ticked ? QStringLiteral("[ ]") : QStringLiteral("[x]");
        const int caret = m_caret;
        replaceSource(span.start, span.end, box, caret);
        return true;
    }
    return false;
}

void Editor::updateCompletion() {
    const int start = m_reading ? -1 : openLinkStart(m_source, m_caret);
    const bool completing = start >= 0;
    const QString query = completing ? m_source.mid(start + 2, m_caret - start - 2) : QString();
    const int anchor = completing ? displayOffset(start) : 0;
    if (completing == m_completing && query == m_completionQuery && anchor == m_completionAnchor) return;
    m_completing = completing;
    m_completionQuery = query;
    m_completionAnchor = anchor;
    emit completionChanged();
}

void Editor::acceptCompletion(const QString &title) {
    const int start = openLinkStart(m_source, m_caret);
    if (start < 0) return;
    const QString text = title + QStringLiteral("]]");
    replaceSource(start + 2, m_caret, text, start + 2 + text.size());
}

void Editor::dismissCompletion() {
    if (!m_completing) return;
    m_completing = false;
    m_completionQuery.clear();
    emit completionChanged();
}

void Editor::insertAtCaret(const QString &text, int displayPosition) {
    if (m_reading) return;
    if (displayPosition >= 0) placeCaret(displayPosition);
    replaceSource(m_caret, m_caret, text, m_caret + text.size());
}

void Editor::insertEmbed(const QString &name) {
    if (m_reading) return;
    int lineStart = 0;
    lineOf(m_caret, &lineStart, nullptr);
    QString text = QStringLiteral("![[") + name + QStringLiteral("]]");
    if (m_caret != lineStart) text = u'\n' + text;
    replaceSource(m_caret, m_caret, text + u'\n', m_caret + text.size() + 1);
}

void Editor::refreshStyles() { rebuild(true); }

// ---- formats ----------------------------------------------------------------

void Editor::applyFormats() {
    if (!m_document) return;
    const qreal scale = m_palette->textScale();
    const qreal ppp = m_palette->pointsPerPixel();
    auto pt = [&](qreal px) { return px * scale * ppp; };
    auto px = [&](qreal v) { return v * scale; };
    const QString sans = m_palette->sansFamily();
    const QString mono = m_palette->monoFamily();
    auto role = [&](const char *name) { return m_palette->role(QLatin1String(name)); };

    m_syncing = true;
    QTextCursor cursor(m_document);
    cursor.beginEditBlock();

    QTextCharFormat base;
    base.setFontFamilies({sans});
    base.setFontPointSize(pt(14.5));
    base.setFontWeight(QFont::Normal);
    base.setFontItalic(false);
    base.setFontStrikeOut(false);
    base.setFontUnderline(false);
    base.setForeground(role("text"));
    base.setBackground(Qt::NoBrush);
    base.setUnderlineStyle(QTextCharFormat::NoUnderline);
    QTextBlockFormat baseBlock;
    baseBlock.setLineHeight(180, QTextBlockFormat::ProportionalHeight);
    baseBlock.setTopMargin(0);
    baseBlock.setBottomMargin(0);
    baseBlock.setLeftMargin(0);
    baseBlock.setTextIndent(0);
    cursor.select(QTextCursor::Document);
    cursor.setCharFormat(base);
    cursor.setBlockFormat(baseBlock);
    m_document->setDefaultFont(base.font());

    auto range = [&](int s, int e, const QTextCharFormat &f) {
        if (e <= s) return;
        QTextCursor c(m_document);
        c.setPosition(clampTo(s, m_display.size()));
        c.setPosition(clampTo(e, m_display.size()), QTextCursor::KeepAnchor);
        c.mergeCharFormat(f);
    };
    auto blockAt = [&](int displayPos, const QTextBlockFormat &f) {
        QTextCursor c(m_document);
        c.setPosition(clampTo(displayPos, m_display.size()));
        c.mergeBlockFormat(f);
    };

    QTextCharFormat monoDim;
    monoDim.setFontFamilies({mono});
    monoDim.setFontPointSize(pt(12.5));
    monoDim.setForeground(role("dimmest"));

    // Revealed markers: mono, dim.
    for (const md::Marker &m : m_parsed.markers) {
        if (m_reading || !m.revealedBy(m_caret)) continue;
        range(displayOffset(m.start), displayOffset(m.end), monoDim);
    }

    // Line-level styling and bullets.
    int lineStart = 0;
    const int n = m_source.size();
    while (lineStart <= n) {
        int lineEnd = m_source.indexOf(u'\n', lineStart);
        if (lineEnd < 0) lineEnd = n;
        const QString line = m_source.mid(lineStart, lineEnd - lineStart);
        int indent = 0;
        while (indent < line.size() && line[indent] == u' ') ++indent;
        const QStringView rest(line.constData() + indent, line.size() - indent);
        const int itemLen = md::bulletLen(rest) ? md::bulletLen(rest) : md::orderedLen(rest);
        if (itemLen) {
            QTextCharFormat bullet;
            bullet.setFontFamilies({mono});
            bullet.setFontPointSize(pt(12.5));
            bullet.setForeground(role("faint"));
            range(displayOffset(lineStart + indent), displayOffset(lineStart + indent + itemLen - 1), bullet);
        }
        if (lineEnd >= n) break;
        lineStart = lineEnd + 1;
    }

    for (const md::Span &span : m_parsed.spans) {
        const int ds = displayOffset(span.start), de = displayOffset(span.end);
        QTextCharFormat f;
        QTextBlockFormat b;
        bool hasBlock = false;
        switch (span.style) {
        case md::Style::Heading: {
            const qreal size = span.level == 1 ? 24 : span.level == 2 ? 18 : span.level == 3 ? 16 : 14.5;
            f.setFontPointSize(pt(size));
            f.setFontWeight(QFont::Bold);
            b.setLineHeight(span.level == 1 ? 130 : 150, QTextBlockFormat::ProportionalHeight);
            b.setTopMargin(span.level == 1 ? 0 : px(span.level == 2 ? 22 : 14));
            hasBlock = true;
            break;
        }
        case md::Style::Bold: f.setFontWeight(QFont::Bold); break;
        case md::Style::Italic: f.setFontItalic(true); break;
        case md::Style::Strikethrough: f.setFontStrikeOut(true); break;
        case md::Style::Code:
            f.setFontFamilies({mono});
            f.setFontPointSize(pt(12.5));
            f.setForeground(role("positive"));
            f.setBackground(role("track"));
            break;
        case md::Style::CodeBlock:
            f.setFontFamilies({mono});
            f.setFontPointSize(pt(12.5));
            f.setForeground(role("text2"));
            b.setLeftMargin(px(12));
            b.setLineHeight(160, QTextBlockFormat::ProportionalHeight);
            hasBlock = true;
            break;
        case md::Style::Quote:
            f.setFontItalic(true);
            f.setForeground(role("muted"));
            b.setLeftMargin(px(26));
            hasBlock = true;
            break;
        case md::Style::ListItem:
            b.setLeftMargin(px(26 + 24 * std::min(span.level, md::MAX_LIST_DEPTH)));
            b.setTextIndent(-px(15));
            hasBlock = true;
            break;
        case md::Style::Link:
            f.setForeground(role("accent"));
            f.setUnderlineStyle(QTextCharFormat::SingleUnderline);
            f.setUnderlineColor(QColor(role("accent").red(), role("accent").green(), role("accent").blue(), 100));
            break;
        case md::Style::WikiLink:
            f.setForeground(role("accent"));
            f.setUnderlineStyle(QTextCharFormat::SingleUnderline);
            f.setUnderlineColor(QColor(role("accent").red(), role("accent").green(), role("accent").blue(), 100));
            break;
        case md::Style::Embed:
            f.setFontFamilies({mono});
            f.setFontPointSize(pt(11));
            f.setForeground(role("accent"));
            break;
        case md::Style::Tag:
            f.setFontFamilies({mono});
            f.setFontPointSize(pt(12.5));
            f.setForeground(role("teal"));
            break;
        case md::Style::Task: {
            f.setFontFamilies({mono});
            f.setFontPointSize(pt(12.5));
            f.setForeground(span.ticked ? role("positive") : role("warning"));
            if (span.ticked) {
                // The text dims; never struck through.
                int lineEnd = m_source.indexOf(u'\n', span.end);
                if (lineEnd < 0) lineEnd = n;
                QTextCharFormat dim;
                dim.setForeground(role("faint"));
                range(de, displayOffset(lineEnd), dim);
            }
            break;
        }
        case md::Style::Rule:
            f.setFontFamilies({mono});
            f.setForeground(role("dimmest"));
            break;
        case md::Style::TableRow:
            f.setFontFamilies({mono});
            f.setFontPointSize(pt(12.5));
            b.setLeftMargin(px(12));
            b.setLineHeight(170, QTextBlockFormat::ProportionalHeight);
            hasBlock = true;
            break;
        case md::Style::TableDelimiter:
            f.setFontFamilies({mono});
            f.setFontPointSize(pt(12.5));
            f.setForeground(role("dimmest"));
            b.setLeftMargin(px(12));
            b.setLineHeight(170, QTextBlockFormat::ProportionalHeight);
            hasBlock = true;
            break;
        case md::Style::Frontmatter:
            f.setFontFamilies({mono});
            f.setFontPointSize(pt(12));
            f.setForeground(role("faint"));
            break;
        }
        range(ds, de, f);
        if (hasBlock) blockAt(ds, b);
    }

    // Reserve room under a line for the image drawn beneath it.
    for (const QVariant &v : m_embeds) {
        const QVariantMap e = v.toMap();
        QTextBlockFormat b;
        b.setBottomMargin(e.value(QStringLiteral("height")).toReal() + px(14));
        blockAt(e.value(QStringLiteral("pos")).toInt(), b);
    }

    cursor.endEditBlock();
    m_syncing = false;
}
