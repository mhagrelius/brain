#include "sync.h"

#include "jsonutil.h"
#include "semantic.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSaveFile>
#include <QSet>

namespace brain::sync {

Hash hashOf(const QString &text) { return semantic::fnv1a(text); }

NoteId conflictId(const NoteId &id, const QString &from, const QString &date) {
    const QString stem = id.title() + QStringLiteral(" (conflict ") + date + QStringLiteral(" from ") + from + u')';
    const QString folder = id.folder();
    return NoteId::fromRelative(folder.isEmpty() ? stem + QStringLiteral(".md") : folder + u'/' + stem + QStringLiteral(".md"));
}

namespace {
// Notes gone from `after` whose content turned up under a new id.
QVector<std::pair<NoteId, NoteId>> renames(const Snapshot &base, const Snapshot &after) {
    QMap<Hash, QVector<NoteId>> arrived;
    for (auto it = after.cbegin(); it != after.cend(); ++it)
        if (!base.contains(it.key())) arrived[it.value()].append(it.key());
    QVector<std::pair<NoteId, NoteId>> found;
    QSet<NoteId> taken;
    for (auto it = base.cbegin(); it != base.cend(); ++it) {
        if (after.contains(it.key())) continue;
        const auto candidates = arrived.find(it.value());
        if (candidates == arrived.end()) continue;
        for (const NoteId &to : *candidates) {
            if (taken.contains(to)) continue;
            taken.insert(to);
            found.append({it.key(), to});
            break;
        }
    }
    return found;
}
}

Plan plan(const Snapshot &base, const Snapshot &local, const Snapshot &remote, const QString &from, const QString &date) {
    Plan plan;
    const auto localRenames = renames(base, local);
    const auto remoteRenames = renames(base, remote);
    QSet<NoteId> settled;
    for (const auto &[fromId, toId] : remoteRenames) {
        bool alsoHere = false;
        for (const auto &r : localRenames) if (r.first == fromId) { alsoHere = true; break; }
        if (alsoHere) continue;
        plan.renameLocal.append({fromId, toId});
        settled.insert(fromId);
        settled.insert(toId);
    }
    for (const auto &[fromId, toId] : localRenames) {
        settled.insert(fromId);
        settled.insert(toId);
        if (!remote.contains(toId)) plan.push.append(toId);
        if (remote.contains(fromId)) plan.deleteRemote.append(fromId);
    }
    QSet<NoteId> all;
    for (const auto &k : base.keys()) all.insert(k);
    for (const auto &k : local.keys()) all.insert(k);
    for (const auto &k : remote.keys()) all.insert(k);
    QVector<NoteId> ids(all.begin(), all.end());
    std::sort(ids.begin(), ids.end());
    for (const NoteId &id : ids) {
        if (settled.contains(id)) continue;
        const std::optional<Hash> was = base.contains(id) ? std::optional<Hash>(base.value(id)) : std::nullopt;
        const std::optional<Hash> here = local.contains(id) ? std::optional<Hash>(local.value(id)) : std::nullopt;
        const std::optional<Hash> there = remote.contains(id) ? std::optional<Hash>(remote.value(id)) : std::nullopt;
        const bool changedHere = here != was;
        const bool changedThere = there != was;
        if (!changedHere && !changedThere) continue;
        if (changedHere && !changedThere) {
            if (here) plan.push.append(id); else plan.deleteRemote.append(id);
        } else if (!changedHere && changedThere) {
            if (there) plan.pull.append(id); else plan.deleteLocal.append(id);
        } else {
            if (here && there && *here == *there) continue;
            if (!here && there) plan.pull.append(id);
            else if (here && !there) plan.push.append(id);
            else if (!here && !there) continue;
            else plan.conflicts.append(Conflict{id, conflictId(id, from, date)});
        }
    }
    return plan;
}

Snapshot snapshotOf(const Vault &vault) {
    Snapshot out;
    for (const Note &note : vault.scan()) out.insert(note.id, hashOf(note.toText()));
    return out;
}

namespace {
std::optional<QString> readText(const Vault &vault, const NoteId &id) {
    const auto note = vault.read(id);
    return note ? std::optional<QString>(note->toText()) : std::nullopt;
}
}

std::optional<Incoming> gather(const Vault &vault, const Snapshot &base, Remote &remote, const QString &from, const QString &date, QString *error) {
    const Snapshot local = snapshotOf(vault);
    const auto there = remote.list(error);
    if (!there) return std::nullopt;
    const Plan p = plan(base, local, *there, from, date);
    Incoming incoming;
    incoming.agreed = base;
    incoming.rename = p.renameLocal;
    incoming.remove = p.deleteLocal;

    QMap<NoteId, NoteId> landing;
    for (const Conflict &c : p.conflicts) landing.insert(c.id, c.copy);
    QVector<NoteId> pulling = p.pull;
    for (const Conflict &c : p.conflicts) pulling.append(c.id);
    const auto fetched = remote.get(pulling, error);
    if (!fetched) return std::nullopt;
    for (const auto &[id, text] : *fetched) {
        const auto copy = landing.find(id);
        Landing l;
        l.id = copy == landing.end() ? id : copy.value();
        l.text = text;
        if (copy != landing.end()) l.conflictWith = id;
        incoming.land.append(l);
    }
    for (const NoteId &id : p.push) {
        const auto text = readText(vault, id);
        if (!text) { ++incoming.report.failed; continue; }
        const Put put = remote.put(id, *text, base.contains(id) ? std::optional<Hash>(base.value(id)) : std::nullopt);
        if (put.kind == Put::Done && put.hash) { incoming.agreed.insert(id, *put.hash); ++incoming.report.pushed; }
        else ++incoming.report.failed;
    }
    // Deletions last: the only step the next pass cannot undo.
    for (const NoteId &id : p.deleteRemote) {
        const Put put = remote.remove(id, base.contains(id) ? std::optional<Hash>(base.value(id)) : std::nullopt);
        if (put.kind == Put::Done) { incoming.agreed.remove(id); ++incoming.report.deletedThere; }
        else ++incoming.report.failed;
    }
    return incoming;
}

std::pair<Snapshot, Report> apply(const Vault &vault, Incoming incoming, const NoteId *protect, const QString &from, const QString &date) {
    Snapshot agreed = incoming.agreed;
    Report report = incoming.report;
    for (const auto &[oldId, newId] : incoming.rename) {
        if (protect && *protect == oldId) continue;
        const auto text = readText(vault, oldId);
        if (vault.rename(oldId, newId)) {
            agreed.remove(oldId);
            if (text) agreed.insert(newId, hashOf(*text));
            ++report.renamed;
        } else ++report.failed;
    }
    for (const Landing &landing : incoming.land) {
        const bool redirected = !landing.conflictWith && protect && *protect == landing.id;
        NoteId target = landing.id;
        bool conflicted = landing.conflictWith.has_value();
        if (redirected) { target = conflictId(landing.id, from, date); conflicted = true; }
        if (!vault.write(Note::fromText(target, landing.text))) { ++report.failed; continue; }
        if (conflicted) ++report.conflicted;
        else { agreed.insert(target, hashOf(landing.text)); ++report.pulled; }
    }
    for (const NoteId &id : incoming.remove) {
        if (protect && *protect == id) continue;
        if (vault.remove(id)) { agreed.remove(id); ++report.deletedHere; }
        else ++report.failed;
    }
    return {agreed, report};
}

QString defaultBasePath(const QString &vaultRoot) { return vaultRoot + u'/' + QLatin1String(CACHE_DIR) + QStringLiteral("/sync.json"); }

Snapshot loadBase(const QString &path) {
    Snapshot out;
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) return out;
    const QJsonObject root = QJsonDocument::fromJson(json::quoteBigIntegers(file.readAll())).object();
    for (auto it = root.begin(); it != root.end(); ++it) out.insert(NoteId::fromRelative(it.key()), json::toU64(it.value()));
    return out;
}

bool saveBase(const Snapshot &base, const QString &path) {
    QDir().mkpath(QFileInfo(path).absolutePath());
    // Hashes are written as strings: a u64 does not survive a JSON double.
    QJsonObject root;
    for (auto it = base.cbegin(); it != base.cend(); ++it) root.insert(it.key().str(), QString::number(it.value()));
    QSaveFile file(path);
    if (!file.open(QIODevice::WriteOnly)) return false;
    file.write(QJsonDocument(root).toJson(QJsonDocument::Compact));
    return file.commit();
}

} // namespace brain::sync
