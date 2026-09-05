#pragma once

// One note: what it is, and how its text turns into it and back.

#include "frontmatter.h"
#include "scanner.h"

#include <QString>
#include <QStringList>

#include <optional>

namespace brain {

// A note's identity: its path relative to the vault root, with `/` separators.
// The file *is* the note, so renaming a note changes its id.
class NoteId {
public:
    NoteId() = default;
    static NoteId fromRelative(const QString &path);
    const QString &str() const { return m_path; }
    QString title() const;          // the filename without its .md
    QString folder() const;         // the containing folder, "" at the root
    bool isNull() const { return m_path.isEmpty(); }
    bool operator==(const NoteId &o) const { return m_path == o.m_path; }
    bool operator!=(const NoteId &o) const { return m_path != o.m_path; }
    bool operator<(const NoteId &o) const { return m_path < o.m_path; }

private:
    QString m_path;
};

inline size_t qHash(const NoteId &id, size_t seed = 0) { return qHash(id.str(), seed); }

struct Note {
    NoteId id;
    std::optional<Frontmatter> frontmatter;
    QString body;   // everything after the metadata block, verbatim

    static Note fromText(const NoteId &id, const QString &text);
    QString toText() const;
    QString title() const { return id.title(); }
    QStringList aliases() const;
    QStringList tags() const;
    md::Extracted extracted() const { return md::extract(body); }
    QString excerpt(int limit) const;
    bool operator==(const Note &o) const { return id == o.id && frontmatter == o.frontmatter && body == o.body; }
};

// The first line of prose that is not just the note's own title.
QString excerptOf(const QString &strippedText, const QString &title, int limit);

} // namespace brain
