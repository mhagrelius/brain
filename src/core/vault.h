#pragma once

// The folder of notes. Scan, atomic writes, folders, attachments.

#include "note.h"

#include <QString>
#include <QStringList>
#include <QVector>

#include <optional>

namespace brain {

struct VaultError {
    QString message;
};

constexpr const char *CACHE_DIR = ".brain";
constexpr const char *ATTACHMENTS_DIR = "attachments";

class Vault {
public:
    Vault() = default;
    explicit Vault(const QString &root);
    const QString &root() const { return m_root; }
    bool isNull() const { return m_root.isEmpty(); }

    QString pathOf(const NoteId &id) const;
    std::optional<NoteId> idOf(const QString &path) const;

    // Every note in the vault. Unreadable files are reported, not fatal.
    QVector<Note> scan(QVector<VaultError> *problems = nullptr) const;
    std::optional<Note> read(const NoteId &id, VaultError *error = nullptr) const;
    // Write atomically: tmp → fsync → rename. Creates the folder if needed.
    bool write(const Note &note, VaultError *error = nullptr) const;
    bool writeText(const NoteId &id, const QString &text, VaultError *error = nullptr) const;
    std::optional<Note> create(const NoteId &id, const QString &contents, VaultError *error = nullptr) const;
    bool rename(const NoteId &from, const NoteId &to, VaultError *error = nullptr) const;
    bool remove(const NoteId &id, VaultError *error = nullptr) const;
    // (modified, created) in seconds since the epoch; 0 when unknown.
    std::pair<qint64, qint64> times(const NoteId &id) const;

    QStringList folders() const;
    QString folderPath(const QString &folder) const;
    bool createFolder(const QString &folder, VaultError *error = nullptr) const;
    bool moveFolder(const QString &from, const QString &to, VaultError *error = nullptr) const;
    // Only an empty folder is removed: "delete the folder" never means "delete the notes".
    bool deleteFolder(const QString &folder, VaultError *error = nullptr) const;
    QStringList directories() const;   // every directory, for the watcher

    // Copy a file into attachments/, returning the name it landed under.
    std::optional<QString> addAttachment(const QString &sourcePath, VaultError *error = nullptr) const;
    std::optional<QString> addAttachmentBytes(const QString &name, const QByteArray &bytes, VaultError *error = nullptr) const;
    QString attachmentPath(const QString &name) const;

private:
    bool writeBytes(const QString &path, const QByteArray &bytes, VaultError *error) const;
    void walk(const QString &dir, QVector<Note> &notes, QVector<VaultError> *problems) const;
    void walkFolders(const QString &dir, const QString &prefix, QStringList &out) const;
    QString m_root;
};

} // namespace brain
