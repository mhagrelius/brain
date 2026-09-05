#pragma once

// The only thing that writes a file or mutates the index. The shell emits
// intent and changes nothing itself, so there is exactly one place a note can
// be lost. A method returns what happened, not what to display.

#include "config.h"
#include "index.h"
#include "search.h"
#include "semantic.h"
#include "sync.h"
#include "tree.h"
#include "vault.h"

#include <QDateTime>
#include <QSet>
#include <QString>
#include <QStringList>
#include <QVector>

#include <optional>

namespace brain {

struct Alert {
    enum Kind { None, NotSaving, Diverged, Vanished, Conflicts } kind = None;
    QString message;
    NoteId id;
    int count = 0;
};

struct External {
    enum Kind { Quiet, Reloaded, Diverged, Vanished } kind = Quiet;
    NoteId id;
    QString onDisk;
};

struct Hit {
    QString id;
    QString title;
    QString detail;
    std::optional<std::pair<int, int>> highlight;
    QString score;
};

struct SyncStatus {
    std::optional<QString> server;
    std::optional<QString> vault;
    int notesHere = 0;
    int notesAgreed = 0;
    std::optional<QDateTime> lastPass;
    std::optional<sync::Report> lastChange;
    std::optional<QString> lastError;
    int vectors = 0;
    std::optional<QString> embeddingServer;
};

class Notebook {
public:
    enum class Mode { Title, Text };

    // ---- config ----
    void loadConfig(const QString &path);
    bool saveConfig(QString *error = nullptr);
    Config &config() { return m_config; }
    const Config &config() const { return m_config; }
    // Absent config = the default server; an empty string = off.
    std::optional<QString> embeddingUrl() const;
    std::optional<std::pair<QString, QString>> sharedVectors() const;
    std::optional<std::pair<QString, QString>> syncServer() const;

    // ---- vault ----
    const Vault *vault() const { return m_vault.isNull() ? nullptr : &m_vault; }
    QString vaultRoot() const { return m_vault.root(); }
    bool hasVault() const { return !m_vault.isNull(); }
    QVector<VaultError> setVault(const QString &root);
    QVector<VaultError> rescan();
    QVector<VaultError> reloadVault();
    const Index &index() const { return m_index; }

    // ---- alerts ----
    Alert alert() const;
    void dismissConflicts() { m_conflicts = 0; }
    bool takeDiskVersion();
    bool restoreOpenNote();
    External absorbExternalChanges();

    // ---- vectors ----
    std::pair<Index, semantic::Store> catchUpInput() const { return {m_index, m_vectors}; }
    void absorbVectors(const semantic::Store &store);
    const semantic::Store &vectors() const { return m_vectors; }
    bool hasVectors() const { return !m_vectors.isEmpty(); }
    void setQueryVector(const QString &query, const QVector<float> &vector) { m_queryVector = {query, vector}; }

    // ---- listing ----
    bool isSearching() const { return !m_query.trimmed().isEmpty(); }
    QVector<std::pair<NoteId, QString>> listedNotes() const;
    QVector<std::pair<NoteId, QString>> searchResults() const;
    QVector<tree::Row> sidebarRows() const;
    QVector<std::pair<QString, int>> tags() const { return m_index.tags(); }
    std::optional<QString> activeTag() const { return m_filter; }
    bool filterByTag(const std::optional<QString> &tag);
    void toggleFolder(const QString &path);
    const QSet<QString> &expanded() const { return m_expanded; }
    tree::Sort sort() const { return m_sort; }
    void setSort(tree::Sort sort) { m_sort = sort; }
    const QString &query() const { return m_query; }
    bool setQuery(const QString &query);
    QVector<NoteId> taggedIds() const;

    // ---- the open note ----
    std::optional<NoteId> openNoteId() const { return m_open; }
    const Note *openNote() const { return m_buffer ? &*m_buffer : nullptr; }
    bool loadNote(const NoteId &id, VaultError *error = nullptr);
    std::optional<NoteId> restoreLastNote();
    void markEdited() { m_dirty = true; }
    bool isDirty() const { return m_dirty; }
    // Copy the editor's body into the open note. Does not write to disk.
    void flushBody(const QString &body);
    void setOpenTags(const QStringList &tags);
    void setOpenAliases(const QStringList &aliases);
    enum class Saved { Clean, Written, Failed };
    Saved saveNow(QString *error = nullptr);
    QString onDiskText() const { return m_onDisk.value_or(QString()); }

    // ---- links and search ----
    QStringList linkCandidates(const QString &query) const;
    Resolution resolveLink(const QString &target) const;
    QVector<Backlink> backlinksOfOpenNote() const;
    // In text mode: hybrid. `wantsEmbedding` says the shell should fetch the query's vector.
    QVector<Hit> search(const QString &query, Mode mode, bool *wantsEmbedding) const;

    // ---- attachments ----
    QStringList attachFiles(const QStringList &paths, QStringList *failed = nullptr);
    std::optional<QString> attachBytes(const QString &name, const QByteArray &bytes);
    QStringList unusedAttachments() const;

    // ---- notes and folders ----
    QString currentFolder() const;
    std::optional<NoteId> createNoteIn(const QString &folder, const QString &title, QString *error = nullptr);
    struct Renamed { enum Kind { Unchanged, Done, Failed } kind = Unchanged; NoteId to; int links = 0; QString error; };
    Renamed renameNote(const QString &title);
    std::optional<NoteId> deleteOpenNote(QString *error = nullptr);
    QStringList folders() const { return m_vault.isNull() ? QStringList() : m_vault.folders(); }
    std::optional<QString> createFolder(const QString &parent, const QString &name, QString *error = nullptr);
    bool deleteFolder(const QString &path, QString *error = nullptr);
    bool renameFolder(const QString &path, const QString &name, QString *error = nullptr);
    bool relocateFolder(const QString &from, const QString &to, QString *error = nullptr);
    struct Moved { enum Kind { Unchanged, Done, Failed } kind = Unchanged; NoteId to; QString destination; QString error; };
    Moved moveNote(const NoteId &id, const QString &destination);
    // Append a captured line to a note, creating it if needed.
    bool appendTo(const NoteId &id, const QString &text, QString *error = nullptr);

    // ---- sync ----
    std::optional<std::pair<Vault, sync::Snapshot>> syncInput() const;
    sync::Report absorbSync(sync::Incoming incoming, const QString &from, const QString &date);
    void recordSyncFailure(const QString &error);
    SyncStatus syncStatus() const;
    std::optional<QDateTime> lastPass() const { return m_lastPass; }

private:
    void loadVectors();
    void clearAlerts();
    void expandTo(const NoteId &id);
    NoteId uniqueId(const QString &folder, const QString &title) const;

    Config m_config;
    QString m_configPath;
    Vault m_vault;
    Index m_index;
    search::Bm25 m_lexical;
    semantic::Store m_vectors;
    std::optional<QString> m_vectorsPath;
    std::optional<std::pair<QString, QVector<float>>> m_queryVector;

    std::optional<NoteId> m_open;
    std::optional<Note> m_buffer;
    std::optional<QString> m_onDisk;
    bool m_dirty = false;
    std::optional<QString> m_notSaving;
    std::optional<NoteId> m_diverged;
    std::optional<NoteId> m_vanished;
    int m_conflicts = 0;

    QSet<QString> m_expanded;
    std::optional<QString> m_targetFolder;
    std::optional<QString> m_filter;
    QString m_query;
    tree::Sort m_sort = tree::Sort::Name;

    std::optional<QDateTime> m_lastPass;
    std::optional<sync::Report> m_lastChange;
    std::optional<QString> m_lastSyncError;
};

} // namespace brain
