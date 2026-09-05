#pragma once

// Deciding what to do when a vault exists in more than one place. A pure plan
// over three snapshots (base, local, remote), then a network half that reads
// local files but writes none (`gather`, on a worker) and a filesystem half
// that does every local write (`apply`, on the thread that owns the notebook).
// A deletion never wins over an edit; a conflict is a note beside the original.

#include "vault.h"

#include <QMap>
#include <QString>
#include <QVector>

#include <optional>

namespace brain::sync {

// FNV-1a over the file's bytes. Over the file, not the stripped text, so a
// change to frontmatter that no model would notice still syncs.
using Hash = quint64;
Hash hashOf(const QString &text);

using Snapshot = QMap<NoteId, Hash>;

struct Conflict {
    NoteId id;
    NoteId copy;
};

struct Plan {
    QVector<NoteId> push;
    QVector<NoteId> pull;
    QVector<NoteId> deleteLocal;
    QVector<NoteId> deleteRemote;
    QVector<std::pair<NoteId, NoteId>> renameLocal;
    QVector<Conflict> conflicts;
    bool isEmpty() const { return push.isEmpty() && pull.isEmpty() && deleteLocal.isEmpty() && deleteRemote.isEmpty() && renameLocal.isEmpty() && conflicts.isEmpty(); }
};

NoteId conflictId(const NoteId &id, const QString &from, const QString &date);
Plan plan(const Snapshot &base, const Snapshot &local, const Snapshot &remote, const QString &from, const QString &date);

struct Put {
    enum Kind { Done, Stale, Failed } kind = Failed;
    std::optional<Hash> hash;   // Done: the hash now; Stale: what the server holds (absent when deleted)
    QString error;
};

class Remote {
public:
    virtual ~Remote() = default;
    virtual std::optional<Snapshot> list(QString *error) = 0;
    virtual std::optional<QVector<std::pair<NoteId, QString>>> get(const QVector<NoteId> &ids, QString *error) = 0;
    virtual Put put(const NoteId &id, const QString &text, std::optional<Hash> base) = 0;
    virtual Put remove(const NoteId &id, std::optional<Hash> base) = 0;
};

struct Report {
    int pushed = 0, pulled = 0, deletedHere = 0, deletedThere = 0, renamed = 0, conflicted = 0, failed = 0;
    bool isQuiet() const { return pushed == 0 && pulled == 0 && deletedHere == 0 && deletedThere == 0 && renamed == 0 && conflicted == 0 && failed == 0; }
};

struct Landing {
    NoteId id;
    QString text;
    std::optional<NoteId> conflictWith;
};

struct Incoming {
    QVector<Landing> land;
    QVector<std::pair<NoteId, NoteId>> rename;
    QVector<NoteId> remove;
    Snapshot agreed;
    Report report;
};

Snapshot snapshotOf(const Vault &vault);
std::optional<Incoming> gather(const Vault &vault, const Snapshot &base, Remote &remote, const QString &from, const QString &date, QString *error);
std::pair<Snapshot, Report> apply(const Vault &vault, Incoming incoming, const NoteId *protect, const QString &from, const QString &date);

QString defaultBasePath(const QString &vaultRoot);
Snapshot loadBase(const QString &path);
bool saveBase(const Snapshot &base, const QString &path);

} // namespace brain::sync
