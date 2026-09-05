#include "semantic.h"

#include "jsonutil.h"

#include <QDir>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSaveFile>
#include <QStandardPaths>

#include <algorithm>
#include <cmath>

namespace brain::semantic {

quint64 fnv1a(const QString &text) {
    quint64 hash = 0xcbf29ce484222325ULL;
    for (unsigned char byte : text.toUtf8()) {
        hash ^= quint64(byte);
        hash *= 0x100000001b3ULL;
    }
    return hash;
}

namespace {
// Cut on a character boundary, never through a surrogate pair.
int safeCut(const QString &s, int at) {
    if (at > 0 && at < s.size() && s[at - 1].isHighSurrogate()) return at - 1;
    return at;
}
}

QVector<QString> chunks(const QString &title, const QString &text) {
    QVector<QString> out;
    QString current;
    auto flush = [&]() {
        const QString trimmed = current.trimmed();
        if (!trimmed.isEmpty()) out.append(title + QStringLiteral("\n\n") + trimmed);
        current.clear();
    };
    for (const QString &raw : text.split(QStringLiteral("\n\n"))) {
        const QString block = raw.trimmed();
        if (block.isEmpty()) continue;
        const bool heading = block.startsWith(u'#');
        if ((heading || current.size() + block.size() > CHUNK_CHARS) && !current.isEmpty()) flush();
        if (block.size() > CHUNK_CHARS) {
            QString rest = block;
            while (!rest.isEmpty()) {
                const int take = safeCut(rest, std::min(int(rest.size()), CHUNK_CHARS));
                current.append(rest.left(take));
                rest = rest.mid(take);
                flush();
            }
            continue;
        }
        if (!current.isEmpty()) current.append(QStringLiteral("\n\n"));
        current.append(block);
    }
    flush();
    if (out.isEmpty() && !title.trimmed().isEmpty()) out.append(title);
    if (out.size() > MAX_CHUNKS) out.resize(MAX_CHUNKS);
    return out;
}

Digest digestOf(const QString &title, const QString &text) {
    const QVector<QString> pieces = chunks(title, text);
    QString joined;
    for (int i = 0; i < pieces.size(); ++i) {
        if (i) joined.append(QChar(1));
        joined.append(pieces[i]);
    }
    return fnv1a(joined);
}

QVector<float> normalise(const QVector<float> &vector) {
    double sum = 0;
    for (float v : vector) sum += double(v) * double(v);
    const double length = std::sqrt(sum);
    if (length == 0 || !std::isfinite(length)) return vector;
    QVector<float> out(vector.size());
    for (int i = 0; i < vector.size(); ++i) out[i] = float(vector[i] / length);
    return out;
}

namespace {
float dot(const QVector<float> &a, const QVector<float> &b) {
    if (a.size() != b.size()) return -1.0f;
    double s = 0;
    for (int i = 0; i < a.size(); ++i) s += double(a[i]) * double(b[i]);
    return float(s);
}
}

const Embedded *Store::get(const NoteId &id) const {
    const auto it = m_notes.find(id);
    return it == m_notes.end() ? nullptr : &it.value();
}

bool Store::setModel(const QString &name) {
    if (model == name) return false;
    const bool had = !m_notes.isEmpty();
    model = name;
    m_notes.clear();
    return had;
}

void Store::insert(const NoteId &id, Digest digest, const Chunks &chunks) {
    Embedded e;
    e.digest = digest;
    for (const QVector<float> &chunk : chunks) e.chunks.append(normalise(chunk));
    m_notes.insert(id, e);
}

void Store::apply(const Plan &plan) {
    for (const Moved &moved : plan.moved) {
        if (!m_notes.contains(moved.from)) continue;
        m_notes.insert(moved.to, m_notes.take(moved.from));
    }
    for (const NoteId &id : plan.drop) m_notes.remove(id);
}

QVector<std::pair<NoteId, float>> Store::nearest(const QVector<float> &rawQuery, float floor, int limit) const {
    const QVector<float> query = normalise(rawQuery);
    QVector<std::pair<NoteId, float>> scored;
    for (auto it = m_notes.cbegin(); it != m_notes.cend(); ++it) {
        float best = -INFINITY;
        for (const QVector<float> &chunk : it->chunks) best = std::max(best, dot(query, chunk));
        if (best >= floor) scored.append({it.key(), best});
    }
    std::sort(scored.begin(), scored.end(), [](const auto &a, const auto &b) { return a.second != b.second ? a.second > b.second : a.first < b.first; });
    if (scored.size() > limit) scored.resize(limit);
    return scored;
}

QString defaultStorePath(const QString &vaultRoot) {
    const QString base = QStandardPaths::writableLocation(QStandardPaths::GenericCacheLocation);
    return base + QStringLiteral("/brain/vectors-") + QStringLiteral("%1").arg(fnv1a(vaultRoot), 16, 16, QLatin1Char('0')) + QStringLiteral(".json");
}

QByteArray Store::toJson() const {
    QJsonObject notes;
    for (auto it = m_notes.cbegin(); it != m_notes.cend(); ++it) {
        QJsonArray chunksJson;
        for (const QVector<float> &chunk : it->chunks) {
            QJsonArray row;
            for (float v : chunk) row.append(double(v));
            chunksJson.append(row);
        }
        notes.insert(it.key().str(), QJsonObject{{QStringLiteral("digest"), QString::number(it->digest)}, {QStringLiteral("chunks"), chunksJson}});
    }
    return QJsonDocument(QJsonObject{{QStringLiteral("model"), model}, {QStringLiteral("notes"), notes}}).toJson(QJsonDocument::Compact);
}

Store Store::fromJson(const QByteArray &json) {
    Store store;
    const QJsonObject root = QJsonDocument::fromJson(json).object();
    store.model = root.value(QStringLiteral("model")).toString();
    const QJsonObject notes = root.value(QStringLiteral("notes")).toObject();
    for (auto it = notes.begin(); it != notes.end(); ++it) {
        const QJsonObject o = it.value().toObject();
        Embedded e;
        e.digest = json::toU64(o.value(QStringLiteral("digest")));
        for (const QJsonValue &row : o.value(QStringLiteral("chunks")).toArray()) {
            QVector<float> chunk;
            for (const QJsonValue &v : row.toArray()) chunk.append(float(v.toDouble()));
            e.chunks.append(chunk);
        }
        store.m_notes.insert(NoteId::fromRelative(it.key()), e);
    }
    return store;
}

Store Store::load(const QString &path) {
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) return Store();
    return fromJson(json::quoteBigIntegers(file.readAll()));
}

bool Store::save(const QString &path) const {
    QDir().mkpath(QFileInfo(path).absolutePath());
    QSaveFile file(path);
    if (!file.open(QIODevice::WriteOnly)) return false;
    file.write(toJson());
    return file.commit();
}

Plan plan(const Store &store, const QVector<Wanted> &wanted) {
    Plan plan;
    QSet<NoteId> live;
    for (const Wanted &w : wanted) live.insert(w.id);
    QMap<Digest, QVector<NoteId>> orphans;
    for (auto it = store.m_notes.cbegin(); it != store.m_notes.cend(); ++it)
        if (!live.contains(it.key())) orphans[it->digest].append(it.key());
    for (const Wanted &want : wanted) {
        const auto it = store.m_notes.find(want.id);
        if (it != store.m_notes.end()) {
            if (it->digest != want.digest) plan.embed.append(want.id);
            continue;
        }
        auto o = orphans.find(want.digest);
        if (o != orphans.end() && !o->isEmpty()) {
            plan.moved.append(Moved{o->takeLast(), want.id});
        } else {
            plan.embed.append(want.id);
        }
    }
    for (const QVector<NoteId> &ids : orphans) for (const NoteId &id : ids) plan.drop.append(id);
    std::sort(plan.drop.begin(), plan.drop.end());
    return plan;
}

QVector<Wanted> wanted(const Index &index) {
    QVector<Wanted> out;
    for (const NoteId &id : index.ids()) out.append(Wanted{id, digestOf(id.title(), index.text(id))});
    return out;
}

std::optional<QVector<float>> Embedder::embedQuery(const QString &query, EmbedError *error) {
    const auto vectors = embed({query}, error);
    if (!vectors || vectors->isEmpty()) {
        if (error && error->message.isEmpty()) error->message = QStringLiteral("no vector came back for the query");
        return std::nullopt;
    }
    return vectors->first();
}

Report catchUp(Store &store, const Index &index, Embedder &embedder, Shared *shared, const Progress &progress) {
    Report report;
    report.reset = store.setModel(embedder.model());
    const QVector<Wanted> wants = wanted(index);
    const Plan p = plan(store, wants);
    report.moved = p.moved.size();
    report.dropped = p.drop.size();
    store.apply(p);

    QVector<std::pair<NoteId, Digest>> digests;
    for (const NoteId &id : p.embed) digests.append({id, digestOf(id.title(), index.text(id))});

    QMap<Digest, Chunks> known;
    if (shared) {
        QVector<Digest> asking;
        for (const auto &d : digests) asking.append(d.second);
        if (const auto found = shared->fetch(asking)) for (const auto &f : *found) known.insert(f.first, f.second);
    }

    QVector<std::pair<Digest, Chunks>> publishing;
    int done = 0;
    for (const auto &[id, digest] : digests) {
        if (progress) progress(done, digests.size());
        const auto k = known.find(digest);
        if (k != known.end()) { store.insert(id, digest, k.value()); ++report.fetched; ++done; continue; }
        const QVector<QString> pieces = chunks(id.title(), index.text(id));
        if (pieces.isEmpty()) { ++done; continue; }
        EmbedError error;
        const auto vectors = embedder.embed(pieces, &error);
        if (vectors && vectors->size() == pieces.size()) {
            store.insert(id, digest, *vectors);
            known.insert(digest, *vectors);
            publishing.append({digest, *vectors});
            ++report.embedded;
            ++done;
        } else {
            report.pending = digests.size() - report.embedded - report.fetched;
            break;
        }
    }
    if (progress) progress(done, digests.size());
    if (shared && !publishing.isEmpty()) shared->publish(publishing);
    return report;
}

} // namespace brain::semantic
