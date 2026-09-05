#include "tree.h"

#include <QMap>

#include <algorithm>

namespace brain::tree {

QString sortName(Sort sort) {
    switch (sort) {
    case Sort::Name: return QStringLiteral("name");
    case Sort::Modified: return QStringLiteral("modified");
    case Sort::Created: return QStringLiteral("created");
    }
    return QStringLiteral("name");
}

Sort sortFromName(const QString &name) {
    if (name == u"modified") return Sort::Modified;
    if (name == u"created") return Sort::Created;
    return Sort::Name;
}

QStringList ancestors(const QString &path) {
    QStringList out;
    QString current;
    for (const QString &segment : path.split(u'/', Qt::SkipEmptyParts)) {
        if (!current.isEmpty()) current += u'/';
        current += segment;
        out.append(current);
    }
    return out;
}

bool isWithin(const QString &folder, const QString &into) { return folder == into || into.startsWith(folder + u'/'); }

namespace {

using Children = QMap<QString, QSet<QString>>;
using Contents = QMap<QString, QVector<const Listed *>>;

void registerPath(const QString &path, Children &children) {
    QString parent;
    for (const QString &segment : path.split(u'/', Qt::SkipEmptyParts)) {
        const QString full = parent.isEmpty() ? segment : parent + u'/' + segment;
        children[parent].insert(full);
        children[full];
        parent = full;
    }
}

int count(const QString &folder, const Children &children, const Contents &contents) {
    int here = contents.value(folder).size();
    for (const QString &child : children.value(folder)) here += count(child, children, contents);
    return here;
}

bool byName(const Listed *a, const Listed *b) {
    const int c = a->id.title().toLower().compare(b->id.title().toLower());
    return c != 0 ? c < 0 : a->id < b->id;
}

void emitRows(const QString &folder, int depth, const Children &children, const Contents &contents, const QSet<QString> &expanded, Sort sort, QVector<Row> &out) {
    QStringList subfolders(children.value(folder).begin(), children.value(folder).end());
    subfolders.sort();
    for (const QString &path : subfolders) {
        const bool open = expanded.contains(path);
        Row row;
        row.folder = true;
        row.path = path;
        row.name = path.section(u'/', -1);
        row.depth = depth;
        row.notes = count(path, children, contents);
        row.expanded = open;
        out.append(row);
        if (open) emitRows(path, depth + 1, children, contents, expanded, sort, out);
    }
    QVector<const Listed *> notes = contents.value(folder);
    switch (sort) {
    case Sort::Name: std::sort(notes.begin(), notes.end(), byName); break;
    case Sort::Modified: std::sort(notes.begin(), notes.end(), [](const Listed *a, const Listed *b) { return a->modified != b->modified ? a->modified > b->modified : byName(a, b); }); break;
    case Sort::Created: std::sort(notes.begin(), notes.end(), [](const Listed *a, const Listed *b) { return a->created != b->created ? a->created > b->created : byName(a, b); }); break;
    }
    for (const Listed *note : notes) {
        Row row;
        row.id = note->id;
        row.excerpt = note->excerpt;
        row.depth = depth;
        row.modified = note->modified;
        out.append(row);
    }
}

} // namespace

QVector<Row> rows(const QVector<Listed> &notes, const QStringList &folders, const QSet<QString> &expanded, Sort sort) {
    Children children;
    Contents contents;
    for (const QString &folder : folders) registerPath(folder, children);
    for (const Listed &note : notes) {
        const QString folder = note.id.folder();
        registerPath(folder, children);
        contents[folder].append(&note);
    }
    children[QString()];
    QVector<Row> out;
    emitRows(QString(), 0, children, contents, expanded, sort, out);
    return out;
}

} // namespace brain::tree
