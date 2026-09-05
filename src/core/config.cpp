#include "config.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSaveFile>
#include <QStandardPaths>

namespace brain {

QString Config::defaultPath() {
    return QStandardPaths::writableLocation(QStandardPaths::GenericConfigLocation) + QStringLiteral("/brain/config.json");
}

namespace {
std::optional<QString> optString(const QJsonObject &o, const char *key) {
    const QJsonValue v = o.value(QLatin1String(key));
    if (v.isString()) return v.toString();
    return std::nullopt;
}
std::optional<int> optInt(const QJsonObject &o, const char *key) {
    const QJsonValue v = o.value(QLatin1String(key));
    if (v.isDouble()) return v.toInt();
    return std::nullopt;
}
void put(QJsonObject &o, const char *key, const std::optional<QString> &v) { if (v) o.insert(QLatin1String(key), *v); else o.insert(QLatin1String(key), QJsonValue::Null); }
void put(QJsonObject &o, const char *key, const std::optional<int> &v) { if (v) o.insert(QLatin1String(key), *v); else o.insert(QLatin1String(key), QJsonValue::Null); }
}

Config Config::fromJson(const QByteArray &json, bool *ok) {
    QJsonParseError error;
    const QJsonDocument doc = QJsonDocument::fromJson(json, &error);
    if (error.error != QJsonParseError::NoError || !doc.isObject()) { if (ok) *ok = false; return Config(); }
    if (ok) *ok = true;
    const QJsonObject o = doc.object();
    Config c;
    if (const auto v = optInt(o, "version")) c.version = *v;
    c.vault = optString(o, "vault");
    c.lastNote = optString(o, "last_note");
    c.windowWidth = optInt(o, "window_width");
    c.windowHeight = optInt(o, "window_height");
    c.windowMaximized = o.value(QStringLiteral("window_maximized")).toBool(false);
    c.readingMode = o.value(QStringLiteral("reading_mode")).toBool(false);
    c.sort = o.value(QStringLiteral("sort")).toString();
    c.embeddingUrl = optString(o, "embedding_url");
    c.vectorsUrl = optString(o, "vectors_url");
    c.vectorsToken = optString(o, "vectors_token");
    c.syncUrl = optString(o, "sync_url");
    c.syncToken = optString(o, "sync_token");
    for (const QJsonValue &v : o.value(QStringLiteral("expanded_folders")).toArray()) c.expandedFolders.append(v.toString());
    return c;
}

QByteArray Config::toJson() const {
    QJsonObject o;
    o.insert(QStringLiteral("version"), version);
    put(o, "vault", vault);
    put(o, "last_note", lastNote);
    put(o, "window_width", windowWidth);
    put(o, "window_height", windowHeight);
    o.insert(QStringLiteral("window_maximized"), windowMaximized);
    o.insert(QStringLiteral("reading_mode"), readingMode);
    o.insert(QStringLiteral("sort"), sort);
    put(o, "embedding_url", embeddingUrl);
    put(o, "vectors_url", vectorsUrl);
    put(o, "vectors_token", vectorsToken);
    put(o, "sync_url", syncUrl);
    put(o, "sync_token", syncToken);
    o.insert(QStringLiteral("expanded_folders"), QJsonArray::fromStringList(expandedFolders));
    return QJsonDocument(o).toJson(QJsonDocument::Indented);
}

std::pair<Config, Config::Outcome> Config::load(const QString &path) {
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) return {Config(), Outcome::Fresh};
    bool ok = false;
    Config c = fromJson(file.readAll(), &ok);
    file.close();
    if (ok) return {c, Outcome::Loaded};
    // Keep the unreadable file rather than deleting it.
    QFile::rename(path, path + QStringLiteral(".corrupt"));
    return {Config(), Outcome::Recovered};
}

bool Config::save(const QString &path, QString *error) const {
    QDir().mkpath(QFileInfo(path).absolutePath());
    QSaveFile file(path);
    if (!file.open(QIODevice::WriteOnly)) { if (error) *error = file.errorString(); return false; }
    file.write(toJson());
    if (!file.commit()) { if (error) *error = file.errorString(); return false; }
    return true;
}

} // namespace brain
