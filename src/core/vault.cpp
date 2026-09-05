#include "vault.h"

#include <QDir>
#include <QDirIterator>
#include <QFile>
#include <QFileInfo>
#include <QSaveFile>

namespace brain {

namespace {
void fail(VaultError *error, const QString &message) { if (error) error->message = message; }
}

Vault::Vault(const QString &root) : m_root(QDir::cleanPath(root)) {}

QString Vault::pathOf(const NoteId &id) const { return m_root + u'/' + id.str(); }

std::optional<NoteId> Vault::idOf(const QString &path) const {
    const QString clean = QDir::cleanPath(path);
    if (!clean.startsWith(m_root + u'/')) return std::nullopt;
    const QString relative = clean.mid(m_root.size() + 1);
    if (!relative.endsWith(QStringLiteral(".md"))) return std::nullopt;
    for (const QString &part : relative.split(u'/')) if (part.startsWith(u'.')) return std::nullopt;
    return NoteId::fromRelative(relative);
}

QVector<Note> Vault::scan(QVector<VaultError> *problems) const {
    QVector<Note> notes;
    walk(m_root, notes, problems);
    return notes;
}

void Vault::walk(const QString &dir, QVector<Note> &notes, QVector<VaultError> *problems) const {
    QDir d(dir);
    if (!d.exists()) {
        if (problems) problems->append(VaultError{dir + QStringLiteral(": not a directory")});
        return;
    }
    const QFileInfoList entries = d.entryInfoList(QDir::Dirs | QDir::Files | QDir::NoDotAndDotDot, QDir::Name);
    for (const QFileInfo &entry : entries) {
        const QString name = entry.fileName();
        // Dotfiles, .brain/ and .git/ are not notes.
        if (name.startsWith(u'.')) continue;
        if (entry.isDir()) { walk(entry.filePath(), notes, problems); continue; }
        const std::optional<NoteId> id = idOf(entry.filePath());
        if (!id) continue;
        VaultError error;
        if (const auto note = read(*id, &error)) notes.append(*note);
        else if (problems) problems->append(error);
    }
}

std::optional<Note> Vault::read(const NoteId &id, VaultError *error) const {
    QFile file(pathOf(id));
    if (!file.open(QIODevice::ReadOnly)) { fail(error, pathOf(id) + QStringLiteral(": ") + file.errorString()); return std::nullopt; }
    const QByteArray bytes = file.readAll();
    return Note::fromText(id, QString::fromUtf8(bytes));
}

bool Vault::writeBytes(const QString &path, const QByteArray &bytes, VaultError *error) const {
    const QString parent = QFileInfo(path).absolutePath();
    if (!QDir().mkpath(parent)) { fail(error, parent + QStringLiteral(": could not create")); return false; }
    // QSaveFile writes a sibling temporary and renames it over the target,
    // with a flush and fsync on commit.
    QSaveFile file(path);
    if (!file.open(QIODevice::WriteOnly)) { fail(error, path + QStringLiteral(": ") + file.errorString()); return false; }
    if (file.write(bytes) != bytes.size() || !file.commit()) { fail(error, path + QStringLiteral(": ") + file.errorString()); return false; }
    return true;
}

bool Vault::write(const Note &note, VaultError *error) const {
    return writeBytes(pathOf(note.id), note.toText().toUtf8(), error);
}

bool Vault::writeText(const NoteId &id, const QString &text, VaultError *error) const {
    return writeBytes(pathOf(id), text.toUtf8(), error);
}

std::optional<Note> Vault::create(const NoteId &id, const QString &contents, VaultError *error) const {
    if (QFileInfo::exists(pathOf(id))) { fail(error, id.str() + QStringLiteral(" already exists")); return std::nullopt; }
    const Note note = Note::fromText(id, contents);
    if (!write(note, error)) return std::nullopt;
    return note;
}

bool Vault::rename(const NoteId &from, const NoteId &to, VaultError *error) const {
    const QString target = pathOf(to);
    if (QFileInfo::exists(target)) { fail(error, to.str() + QStringLiteral(" already exists")); return false; }
    QDir().mkpath(QFileInfo(target).absolutePath());
    if (!QFile::rename(pathOf(from), target)) { fail(error, QStringLiteral("could not move ") + from.str()); return false; }
    return true;
}

bool Vault::remove(const NoteId &id, VaultError *error) const {
    QFile file(pathOf(id));
    if (!file.remove()) { fail(error, pathOf(id) + QStringLiteral(": ") + file.errorString()); return false; }
    return true;
}

std::pair<qint64, qint64> Vault::times(const NoteId &id) const {
    const QFileInfo info(pathOf(id));
    if (!info.exists()) return {0, 0};
    const qint64 modified = info.lastModified().toSecsSinceEpoch();
    const QDateTime birth = info.birthTime();
    return {modified, birth.isValid() ? birth.toSecsSinceEpoch() : modified};
}

QStringList Vault::folders() const {
    QStringList out;
    walkFolders(m_root, QString(), out);
    out.sort();
    return out;
}

void Vault::walkFolders(const QString &dir, const QString &prefix, QStringList &out) const {
    const QFileInfoList entries = QDir(dir).entryInfoList(QDir::Dirs | QDir::NoDotAndDotDot, QDir::Name);
    for (const QFileInfo &entry : entries) {
        const QString name = entry.fileName();
        if (name.startsWith(u'.') || (prefix.isEmpty() && name == QLatin1String(ATTACHMENTS_DIR))) continue;
        const QString path = prefix.isEmpty() ? name : prefix + u'/' + name;
        out.append(path);
        walkFolders(entry.filePath(), path, out);
    }
}

QStringList Vault::directories() const {
    QStringList out{m_root};
    for (const QString &folder : folders()) out.append(folderPath(folder));
    return out;
}

QString Vault::folderPath(const QString &folder) const { return folder.isEmpty() ? m_root : m_root + u'/' + folder; }

bool Vault::createFolder(const QString &folder, VaultError *error) const {
    if (folder.isEmpty() || QFileInfo::exists(folderPath(folder))) { fail(error, folder + QStringLiteral(" already exists")); return false; }
    if (!QDir().mkpath(folderPath(folder))) { fail(error, QStringLiteral("could not create ") + folder); return false; }
    return true;
}

bool Vault::moveFolder(const QString &from, const QString &to, VaultError *error) const {
    if (from.isEmpty() || to.isEmpty()) { fail(error, QStringLiteral("the vault root cannot move")); return false; }
    if (to == from || to.startsWith(from + u'/')) { fail(error, QStringLiteral("a folder cannot move inside itself")); return false; }
    if (QFileInfo::exists(folderPath(to))) { fail(error, to + QStringLiteral(" already exists")); return false; }
    QDir().mkpath(QFileInfo(folderPath(to)).absolutePath());
    if (!QDir().rename(folderPath(from), folderPath(to))) { fail(error, QStringLiteral("could not move ") + from); return false; }
    return true;
}

bool Vault::deleteFolder(const QString &folder, VaultError *error) const {
    if (folder.isEmpty()) { fail(error, QStringLiteral("the vault root cannot be removed")); return false; }
    QDir d(folderPath(folder));
    if (!d.exists()) { fail(error, folder + QStringLiteral(" does not exist")); return false; }
    // Only an empty folder goes. Dotfiles count as contents too: an rmdir
    // fails on them, which is the right answer.
    if (!d.entryList(QDir::AllEntries | QDir::NoDotAndDotDot | QDir::Hidden).isEmpty()) { fail(error, folder + QStringLiteral(" is not empty")); return false; }
    if (!QDir().rmdir(d.absolutePath())) { fail(error, QStringLiteral("could not remove ") + folder); return false; }
    return true;
}

QString Vault::attachmentPath(const QString &name) const { return m_root + u'/' + QLatin1String(ATTACHMENTS_DIR) + u'/' + name; }

std::optional<QString> Vault::addAttachment(const QString &sourcePath, VaultError *error) const {
    QFile source(sourcePath);
    if (!source.open(QIODevice::ReadOnly)) { fail(error, sourcePath + QStringLiteral(": ") + source.errorString()); return std::nullopt; }
    return addAttachmentBytes(QFileInfo(sourcePath).fileName(), source.readAll(), error);
}

std::optional<QString> Vault::addAttachmentBytes(const QString &name, const QByteArray &bytes, VaultError *error) const {
    if (name.isEmpty()) { fail(error, QStringLiteral("an attachment needs a name")); return std::nullopt; }
    const int dot = name.lastIndexOf(u'.');
    const QString stem = dot > 0 ? name.left(dot) : name;
    const QString extension = dot > 0 ? name.mid(dot) : QString();
    for (int attempt = 0; attempt < 10000; ++attempt) {
        const QString candidate = attempt == 0 ? name : stem + u'-' + QString::number(attempt) + extension;
        const QString path = attachmentPath(candidate);
        QFile existing(path);
        if (existing.open(QIODevice::ReadOnly)) {
            if (existing.readAll() == bytes) return candidate;   // same name, same bytes: already here
            continue;
        }
        if (!writeBytes(path, bytes, error)) return std::nullopt;
        return candidate;
    }
    fail(error, QStringLiteral("no free name for ") + name);
    return std::nullopt;
}

} // namespace brain
