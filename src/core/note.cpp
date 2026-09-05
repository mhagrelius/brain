#include "note.h"

namespace brain {

NoteId NoteId::fromRelative(const QString &path) {
    NoteId id;
    QStringList parts;
    for (const QString &part : path.split(u'/', Qt::SkipEmptyParts)) {
        if (part == u".") continue;
        parts.append(part);
    }
    id.m_path = parts.join(u'/');
    return id;
}

QString NoteId::title() const {
    const QString name = m_path.section(u'/', -1);
    return name.endsWith(QStringLiteral(".md")) ? name.left(name.size() - 3) : name;
}

QString NoteId::folder() const {
    const int slash = m_path.lastIndexOf(u'/');
    return slash < 0 ? QString() : m_path.left(slash);
}

Note Note::fromText(const NoteId &id, const QString &text) {
    Note note;
    note.id = id;
    auto [frontmatter, body] = Frontmatter::split(text);
    note.frontmatter = frontmatter;
    note.body = body;
    return note;
}

QString Note::toText() const {
    return frontmatter ? frontmatter->render() + body : body;
}

QStringList Note::aliases() const { return frontmatter ? frontmatter->aliases : QStringList(); }

QStringList Note::tags() const {
    QStringList tags = frontmatter ? frontmatter->tags : QStringList();
    for (const md::TagRef &tag : md::extract(body).tags) {
        bool have = false;
        for (const QString &existing : tags) if (existing.compare(tag.name, Qt::CaseInsensitive) == 0) { have = true; break; }
        if (!have) tags.append(tag.name);
    }
    return tags;
}

QString excerptOf(const QString &text, const QString &title, int limit) {
    QString line;
    for (const QString &raw : text.split(u'\n')) {
        const QString t = raw.trimmed();
        if (t.isEmpty() || t.compare(title, Qt::CaseInsensitive) == 0) continue;
        line = t;
        break;
    }
    if (line.size() <= limit) return line;
    QString cut = line.left(limit);
    if (!cut.isEmpty() && cut.back().isHighSurrogate()) cut.chop(1);
    return cut + QChar(0x2026);
}

QString Note::excerpt(int limit) const {
    return excerptOf(md::strip(body), title(), limit);
}

} // namespace brain
