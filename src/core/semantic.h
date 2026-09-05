#pragma once

// Vectors over the vault: the chunking, the fingerprint, the store, the plan
// that keeps the store level with the vault, and the seams a model server and
// a shared store plug into. QtCore only; the sockets live in src/net.

#include "index.h"

#include <QMap>
#include <QString>
#include <QVector>

#include <functional>
#include <optional>

namespace brain::semantic {

constexpr int CHUNK_CHARS = 2000;
constexpr int MAX_CHUNKS = 16;

// FNV-1a over the chunk text: stable across runs and machines, and the same
// function the Rust client used, so the shared store's keys agree.
quint64 fnv1a(const QString &text);
using Digest = quint64;
using Chunks = QVector<QVector<float>>;

struct Embedded {
    Digest digest = 0;
    Chunks chunks;
};

struct Wanted {
    NoteId id;
    Digest digest = 0;
};

struct Moved {
    NoteId from;
    NoteId to;
};

struct Plan {
    QVector<NoteId> embed;
    QVector<Moved> moved;
    QVector<NoteId> drop;
    bool isEmpty() const { return embed.isEmpty() && moved.isEmpty() && drop.isEmpty(); }
};

QVector<QString> chunks(const QString &title, const QString &text);
Digest digestOf(const QString &title, const QString &text);
QVector<float> normalise(const QVector<float> &vector);

class Store {
public:
    QString model;
    int size() const { return m_notes.size(); }
    bool isEmpty() const { return m_notes.isEmpty(); }
    bool contains(const NoteId &id) const { return m_notes.contains(id); }
    const Embedded *get(const NoteId &id) const;
    QVector<NoteId> ids() const { return QVector<NoteId>(m_notes.keyBegin(), m_notes.keyEnd()); }
    bool setModel(const QString &model);   // true when vectors were thrown away
    void insert(const NoteId &id, Digest digest, const Chunks &chunks);
    void apply(const Plan &plan);
    QVector<std::pair<NoteId, float>> nearest(const QVector<float> &query, float floor, int limit) const;

    static Store load(const QString &path);
    bool save(const QString &path) const;
    QByteArray toJson() const;
    static Store fromJson(const QByteArray &json);

    friend Plan plan(const Store &store, const QVector<Wanted> &wanted);

private:
    QMap<NoteId, Embedded> m_notes;
};

QString defaultStorePath(const QString &vaultRoot);
Plan plan(const Store &store, const QVector<Wanted> &wanted);
QVector<Wanted> wanted(const Index &index);

struct EmbedError {
    QString message;
};

class Embedder {
public:
    virtual ~Embedder() = default;
    virtual QString model() const = 0;
    virtual std::optional<Chunks> embed(const QVector<QString> &texts, EmbedError *error) = 0;
    virtual std::optional<QVector<float>> embedQuery(const QString &query, EmbedError *error);
};

class Shared {
public:
    virtual ~Shared() = default;
    virtual std::optional<QVector<std::pair<Digest, Chunks>>> fetch(const QVector<Digest> &digests) = 0;
    virtual bool publish(const QVector<std::pair<Digest, Chunks>> &entries) = 0;
};

struct Report {
    bool reset = false;
    int moved = 0;
    int dropped = 0;
    int fetched = 0;
    int embedded = 0;
    int pending = 0;
    bool isQuiet() const { return !reset && moved == 0 && dropped == 0 && fetched == 0 && embedded == 0 && pending == 0; }
};

using Progress = std::function<void(int done, int total)>;
Report catchUp(Store &store, const Index &index, Embedder &embedder, Shared *shared = nullptr, const Progress &progress = {});

} // namespace brain::semantic
