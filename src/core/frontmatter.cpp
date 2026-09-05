#include "frontmatter.h"

namespace brain {

std::optional<Frontmatter::Key> Frontmatter::parseKey(const QString &name) {
    if (name == u"tags") return Key::Tags;
    if (name == u"aliases") return Key::Aliases;
    if (name == u"created") return Key::Created;
    if (name == u"updated") return Key::Updated;
    return std::nullopt;
}

QString Frontmatter::keyName(Key key) {
    switch (key) {
    case Key::Tags: return QStringLiteral("tags");
    case Key::Aliases: return QStringLiteral("aliases");
    case Key::Created: return QStringLiteral("created");
    case Key::Updated: return QStringLiteral("updated");
    }
    return QString();
}

std::pair<std::optional<Frontmatter>, QString> Frontmatter::split(const QString &text) {
    if (text.isEmpty()) return {std::nullopt, text};
    int firstEnd = text.indexOf(u'\n');
    const bool firstHasNewline = firstEnd >= 0;
    if (firstEnd < 0) firstEnd = text.size();
    const QString first = text.left(firstEnd);
    // Rust's trim_end trims all whitespace; the delimiter is exactly "---".
    QString firstTrimmed = first;
    while (!firstTrimmed.isEmpty() && firstTrimmed.back().isSpace()) firstTrimmed.chop(1);
    if (firstTrimmed != u"---") return {std::nullopt, text};

    QStringList raw;
    std::optional<QString> close;
    bool closeNewline = false;
    int consumed = firstEnd + (firstHasNewline ? 1 : 0);
    int at = consumed;
    while (at < text.size()) {
        int end = text.indexOf(u'\n', at);
        const bool hasNewline = end >= 0;
        if (end < 0) end = text.size();
        const QString content = text.mid(at, end - at);
        consumed = end + (hasNewline ? 1 : 0);
        at = consumed;
        QString trimmed = content;
        while (!trimmed.isEmpty() && trimmed.back().isSpace()) trimmed.chop(1);
        if (trimmed == u"---") {
            close = content;
            closeNewline = hasNewline;
            break;
        }
        raw.append(content);
    }
    // An unterminated block is not frontmatter.
    if (!close) return {std::nullopt, text};

    Frontmatter fm;
    fm.m_entries = group(raw);
    fm.m_open = first;
    fm.m_close = close;
    fm.m_closeNewline = closeNewline;
    fm.readValues();
    fm.m_original = Original{fm.tags, fm.aliases, fm.created, fm.updated};
    return {fm, text.mid(consumed)};
}

QVector<Frontmatter::Entry> Frontmatter::group(const QStringList &raw) {
    QVector<Entry> entries;
    int index = 0;
    while (index < raw.size()) {
        const QString &line = raw.at(index);
        const std::optional<QString> name = keyNameOf(line);
        const std::optional<Key> key = name ? parseKey(*name) : std::nullopt;
        QStringList lines{line};
        ++index;
        if (name) {
            while (index < raw.size() && isContinuation(raw.at(index))) {
                lines.append(raw.at(index));
                ++index;
            }
        }
        Entry entry;
        entry.known = key.has_value();
        if (key) entry.key = *key;
        entry.lines = lines;
        entries.append(entry);
    }
    return entries;
}

std::optional<QString> Frontmatter::keyNameOf(const QString &line) {
    if (line.isEmpty() || line.front().isSpace()) return std::nullopt;
    const int colon = line.indexOf(u':');
    if (colon < 0) return std::nullopt;
    const QString name = line.left(colon);
    if (name.isEmpty()) return std::nullopt;
    for (QChar c : name) if (!c.isLetterOrNumber() && c != u'_') return std::nullopt;
    return name;
}

bool Frontmatter::isContinuation(const QString &line) {
    if (!line.isEmpty() && line.front().isSpace()) return true;
    QString t = line;
    while (!t.isEmpty() && t.front().isSpace()) t.remove(0, 1);
    return t.startsWith(QStringLiteral("- "));
}

QString Frontmatter::clean(const QString &raw) {
    QString value = raw.trimmed();
    if (value.size() >= 2 && ((value.startsWith(u'"') && value.endsWith(u'"')) || (value.startsWith(u'\'') && value.endsWith(u'\''))))
        value = value.mid(1, value.size() - 2);
    return value.trimmed();
}

QStringList Frontmatter::listValue(const QStringList &lines) {
    const QString &first = lines.first();
    const int colon = first.indexOf(u':');
    const QString inlineValue = colon >= 0 ? first.mid(colon + 1).trimmed() : QString();
    QStringList out;
    if (!inlineValue.isEmpty()) {
        QString inner = inlineValue;
        if (inner.startsWith(u'[') && inner.endsWith(u']')) inner = inner.mid(1, inner.size() - 2);
        for (const QString &part : inner.split(u',')) {
            const QString c = clean(part);
            if (!c.isEmpty()) out.append(c);
        }
        return out;
    }
    for (int i = 1; i < lines.size(); ++i) {
        const QString t = lines.at(i).trimmed();
        if (!t.startsWith(QStringLiteral("- "))) continue;
        const QString c = clean(t.mid(2));
        if (!c.isEmpty()) out.append(c);
    }
    return out;
}

std::optional<QDate> Frontmatter::dateValue(const QStringList &lines) {
    const QString &first = lines.first();
    const int colon = first.indexOf(u':');
    if (colon < 0) return std::nullopt;
    const QString value = clean(first.mid(colon + 1));
    const QDate date = QDate::fromString(value, QStringLiteral("yyyy-MM-dd"));
    if (!date.isValid() || value.size() != 10) return std::nullopt;
    return date;
}

void Frontmatter::readValues() {
    for (const Entry &entry : m_entries) {
        if (!entry.known) continue;
        switch (entry.key) {
        case Key::Tags: tags = listValue(entry.lines); break;
        case Key::Aliases: aliases = listValue(entry.lines); break;
        case Key::Created: created = dateValue(entry.lines); break;
        case Key::Updated: updated = dateValue(entry.lines); break;
        }
    }
}

bool Frontmatter::changed(Key key) const {
    if (!m_original) return true;
    switch (key) {
    case Key::Tags: return tags != m_original->tags;
    case Key::Aliases: return aliases != m_original->aliases;
    case Key::Created: return created != m_original->created;
    case Key::Updated: return updated != m_original->updated;
    }
    return true;
}

bool Frontmatter::isSet(Key key) const {
    switch (key) {
    case Key::Tags: return !tags.isEmpty();
    case Key::Aliases: return !aliases.isEmpty();
    case Key::Created: return created.has_value();
    case Key::Updated: return updated.has_value();
    }
    return false;
}

bool Frontmatter::isEmpty() const {
    if (!tags.isEmpty() || !aliases.isEmpty() || created || updated) return false;
    for (const Entry &entry : m_entries) {
        if (entry.known) {
            if (!(!isSet(entry.key) && changed(entry.key))) return false;
        } else {
            for (const QString &line : entry.lines) if (!line.trimmed().isEmpty()) return false;
        }
    }
    return true;
}

std::optional<QString> Frontmatter::lineFor(Key key) const {
    QString value;
    switch (key) {
    case Key::Tags: if (tags.isEmpty()) return std::nullopt; value = u'[' + tags.join(QStringLiteral(", ")) + u']'; break;
    case Key::Aliases: if (aliases.isEmpty()) return std::nullopt; value = u'[' + aliases.join(QStringLiteral(", ")) + u']'; break;
    case Key::Created: if (!created) return std::nullopt; value = created->toString(QStringLiteral("yyyy-MM-dd")); break;
    case Key::Updated: if (!updated) return std::nullopt; value = updated->toString(QStringLiteral("yyyy-MM-dd")); break;
    }
    return keyName(key) + QStringLiteral(": ") + value;
}

QString Frontmatter::render() const {
    if (isEmpty()) return QString();
    QString out = m_open + u'\n';
    QVector<Key> written;
    for (const Entry &entry : m_entries) {
        if (!entry.known) {
            for (const QString &line : entry.lines) out += line + u'\n';
            continue;
        }
        written.append(entry.key);
        if (!changed(entry.key)) {
            for (const QString &line : entry.lines) out += line + u'\n';
        } else if (const auto line = lineFor(entry.key)) {
            out += *line + u'\n';
        }
    }
    for (Key key : {Key::Tags, Key::Aliases, Key::Created, Key::Updated}) {
        if (written.contains(key) || !isSet(key)) continue;
        if (const auto line = lineFor(key)) out += *line + u'\n';
    }
    out += m_close.value_or(QStringLiteral("---"));
    if (m_closeNewline) out += u'\n';
    return out;
}

bool Frontmatter::operator==(const Frontmatter &o) const {
    return tags == o.tags && aliases == o.aliases && created == o.created && updated == o.updated && render() == o.render();
}

} // namespace brain
