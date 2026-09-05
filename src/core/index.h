#pragma once

// Everything derived from the notes: titles, aliases, links, backlinks, tags,
// stripped text. The vault is canonical; this is rebuilt from it.

#include "note.h"

#include <QHash>
#include <QMap>
#include <QSet>
#include <QString>
#include <QStringList>
#include <QVector>

namespace brain {

struct Resolution {
    enum Kind { NoteFound, Ambiguous, Missing } kind = Missing;
    NoteId note;
    QVector<NoteId> candidates;
};

struct Backlink {
    NoteId from;
    QString context;   // the line holding the link, markup stripped
};

class Index {
public:
    static Index build(const QVector<Note> &notes);
    void update(const Note &note);
    void remove(const NoteId &id);
    void rename(const NoteId &from, const NoteId &to);

    int size() const { return m_entries.size(); }
    bool isEmpty() const { return m_entries.isEmpty(); }
    QVector<NoteId> ids() const;   // sorted
    bool contains(const NoteId &id) const { return m_entries.contains(id); }
    QString excerpt(const NoteId &id) const;
    QString text(const NoteId &id) const;   // markup stripped
    QString body(const NoteId &id) const;
    QStringList tagsOf(const NoteId &id) const;
    QStringList aliasesOf(const NoteId &id) const;
    QVector<md::WikiLink> linksOf(const NoteId &id) const;
    QVector<md::WikiLink> embedsOf(const NoteId &id) const;

    // Tried in order: an exact vault path, then a title, then an alias.
    Resolution resolve(const QString &target, const NoteId *from = nullptr) const;
    QVector<Backlink> backlinks(const NoteId &id) const;
    // Link targets that resolve to nothing, with the notes that want them.
    QMap<QString, QVector<NoteId>> missing() const { return m_missing; }
    // Every tag, lowercased, sorted, with its note count; nested tags count
    // towards their parents.
    QVector<std::pair<QString, int>> tags() const;
    QVector<NoteId> notesTagged(const QString &tag) const;
    QSet<QString> referencedAttachments() const;

private:
    struct Entry {
        QString title;
        QStringList aliases;
        QStringList tags;
        QVector<md::WikiLink> links;
        QVector<md::WikiLink> embeds;
        QString body;
        QString text;
        QString excerpt;
        static Entry of(const Note &note);
        QString contextAt(int offset) const;
    };
    void rebuildDerived();

    QMap<NoteId, Entry> m_entries;
    QHash<QString, QVector<NoteId>> m_names;   // lowercased title or alias → notes
    QMap<QString, QVector<NoteId>> m_tags;     // lowercased tag → notes
    QHash<NoteId, QVector<Backlink>> m_backlinks;
    QMap<QString, QVector<NoteId>> m_missing;
};

QStringList tagAncestors(const QString &tag);

} // namespace brain
