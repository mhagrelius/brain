#include "index.h"

namespace brain {

QStringList tagAncestors(const QString &tag) {
    QStringList out;
    QString current;
    for (const QString &part : tag.split(u'/')) {
        if (!current.isEmpty()) current += u'/';
        current += part;
        out.append(current);
    }
    return out;
}

Index::Entry Index::Entry::of(const Note &note) {
    const md::Parsed parsed = md::parse(note.body);
    const md::Extracted extracted = md::extractWith(note.body, parsed);
    Entry e;
    e.text = md::stripWith(note.body, parsed);
    e.tags = note.frontmatter ? note.frontmatter->tags : QStringList();
    for (const md::TagRef &tag : extracted.tags) {
        bool have = false;
        for (const QString &existing : e.tags) if (existing.compare(tag.name, Qt::CaseInsensitive) == 0) { have = true; break; }
        if (!have) e.tags.append(tag.name);
    }
    e.title = note.title();
    e.aliases = note.aliases();
    e.links = extracted.links;
    e.embeds = extracted.embeds;
    e.body = note.body;
    e.excerpt = excerptOf(e.text, note.title(), 120);
    return e;
}

QString Index::Entry::contextAt(int offset) const {
    const int at = std::min(offset, int(body.size()));
    const int start = body.lastIndexOf(u'\n', at - 1 < 0 ? 0 : at - 1) + 1;
    int end = body.indexOf(u'\n', start);
    if (end < 0) end = body.size();
    return md::strip(body.mid(start, end - start)).trimmed();
}

Index Index::build(const QVector<Note> &notes) {
    Index index;
    for (const Note &note : notes) index.m_entries.insert(note.id, Entry::of(note));
    index.rebuildDerived();
    return index;
}

void Index::update(const Note &note) {
    m_entries.insert(note.id, Entry::of(note));
    rebuildDerived();
}

void Index::remove(const NoteId &id) {
    m_entries.remove(id);
    rebuildDerived();
}

void Index::rename(const NoteId &from, const NoteId &to) {
    if (!m_entries.contains(from)) return;
    Entry entry = m_entries.take(from);
    entry.title = to.title();
    m_entries.insert(to, entry);
    rebuildDerived();
}

void Index::rebuildDerived() {
    m_names.clear();
    m_tags.clear();
    m_backlinks.clear();
    m_missing.clear();
    for (auto it = m_entries.cbegin(); it != m_entries.cend(); ++it) {
        m_names[it->title.toLower()].append(it.key());
        for (const QString &alias : it->aliases) {
            QVector<NoteId> &list = m_names[alias.toLower()];
            if (!list.contains(it.key())) list.append(it.key());
        }
        for (const QString &tag : it->tags) {
            QVector<NoteId> &list = m_tags[tag.toLower()];
            if (!list.contains(it.key())) list.append(it.key());
        }
    }
    for (auto it = m_entries.cbegin(); it != m_entries.cend(); ++it) {
        for (const md::WikiLink &link : it->links) {
            const NoteId from = it.key();
            const Resolution r = resolve(link.target, &from);
            if (r.kind == Resolution::NoteFound) {
                m_backlinks[r.note].append(Backlink{from, it->contextAt(link.start)});
            } else if (r.kind == Resolution::Missing) {
                QVector<NoteId> &list = m_missing[link.target.trimmed()];
                if (!list.contains(from)) list.append(from);
            }
        }
    }
}

QVector<NoteId> Index::ids() const { return QVector<NoteId>(m_entries.keyBegin(), m_entries.keyEnd()); }
QString Index::excerpt(const NoteId &id) const { const auto it = m_entries.find(id); return it == m_entries.end() ? QString() : it->excerpt; }
QString Index::text(const NoteId &id) const { const auto it = m_entries.find(id); return it == m_entries.end() ? QString() : it->text; }
QString Index::body(const NoteId &id) const { const auto it = m_entries.find(id); return it == m_entries.end() ? QString() : it->body; }
QStringList Index::tagsOf(const NoteId &id) const { const auto it = m_entries.find(id); return it == m_entries.end() ? QStringList() : it->tags; }
QStringList Index::aliasesOf(const NoteId &id) const { const auto it = m_entries.find(id); return it == m_entries.end() ? QStringList() : it->aliases; }
QVector<md::WikiLink> Index::linksOf(const NoteId &id) const { const auto it = m_entries.find(id); return it == m_entries.end() ? QVector<md::WikiLink>() : it->links; }
QVector<md::WikiLink> Index::embedsOf(const NoteId &id) const { const auto it = m_entries.find(id); return it == m_entries.end() ? QVector<md::WikiLink>() : it->embeds; }

Resolution Index::resolve(const QString &rawTarget, const NoteId *from) const {
    Resolution r;
    const QString target = rawTarget.trimmed();
    if (target.isEmpty()) return r;
    const NoteId asPath = NoteId::fromRelative(target.endsWith(QStringLiteral(".md")) ? target : target + QStringLiteral(".md"));
    if (m_entries.contains(asPath)) { r.kind = Resolution::NoteFound; r.note = asPath; return r; }
    const auto it = m_names.find(target.toLower());
    if (it == m_names.end() || it->isEmpty()) return r;
    if (it->size() == 1) { r.kind = Resolution::NoteFound; r.note = it->first(); return r; }
    if (from && it->contains(*from)) { r.kind = Resolution::NoteFound; r.note = *from; return r; }
    r.kind = Resolution::Ambiguous;
    r.candidates = *it;
    return r;
}

QVector<Backlink> Index::backlinks(const NoteId &id) const { return m_backlinks.value(id); }

QVector<std::pair<QString, int>> Index::tags() const {
    QMap<QString, QSet<NoteId>> counts;
    for (auto it = m_tags.cbegin(); it != m_tags.cend(); ++it)
        for (const QString &ancestor : tagAncestors(it.key()))
            for (const NoteId &id : it.value()) counts[ancestor].insert(id);
    QVector<std::pair<QString, int>> out;
    for (auto it = counts.cbegin(); it != counts.cend(); ++it) out.append({it.key(), it->size()});
    return out;
}

QVector<NoteId> Index::notesTagged(const QString &tag) const {
    QString wanted = tag;
    while (wanted.startsWith(u'#')) wanted.remove(0, 1);
    wanted = wanted.toLower();
    QSet<NoteId> ids;
    for (auto it = m_tags.cbegin(); it != m_tags.cend(); ++it)
        if (it.key() == wanted || it.key().startsWith(wanted + u'/'))
            for (const NoteId &id : it.value()) ids.insert(id);
    QVector<NoteId> out(ids.begin(), ids.end());
    std::sort(out.begin(), out.end());
    return out;
}

QSet<QString> Index::referencedAttachments() const {
    QSet<QString> out;
    for (const Entry &e : m_entries) for (const md::WikiLink &embed : e.embeds) out.insert(embed.target);
    return out;
}

} // namespace brain
