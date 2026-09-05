#pragma once

// The sidebar's shape: folders, and the notes inside them. The vault is the
// tree; which folders are open is the caller's view state.

#include "note.h"

#include <QSet>
#include <QString>
#include <QStringList>
#include <QVector>

namespace brain::tree {

enum class Sort { Name, Modified, Created };
QString sortName(Sort sort);
Sort sortFromName(const QString &name);

struct Listed {
    NoteId id;
    QString excerpt;
    qint64 modified = 0;
    qint64 created = 0;
};

struct Row {
    bool folder = false;
    QString path;      // folder: vault-relative path
    QString name;      // folder: last segment
    NoteId id;         // note
    QString excerpt;   // note
    int depth = 0;
    int notes = 0;     // folder: notes anywhere beneath
    bool expanded = false;
    qint64 modified = 0;
};

QVector<Row> rows(const QVector<Listed> &notes, const QStringList &folders, const QSet<QString> &expanded, Sort sort);
bool isWithin(const QString &folder, const QString &into);
QStringList ancestors(const QString &path);

} // namespace brain::tree
