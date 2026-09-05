#pragma once

// The one piece of state that cannot live in the vault: which vault, and
// where the servers are. Same file and schema as the GTK app:
// ~/.config/brain/config.json.

#include <QString>
#include <QStringList>

#include <optional>

namespace brain {

struct Config {
    static constexpr int SCHEMA_VERSION = 1;
    int version = SCHEMA_VERSION;
    std::optional<QString> vault;
    std::optional<QString> lastNote;
    std::optional<int> windowWidth;
    std::optional<int> windowHeight;
    bool windowMaximized = false;
    bool readingMode = false;
    QString sort;
    // Absent = the default (mattnas); empty string = off.
    std::optional<QString> embeddingUrl;
    std::optional<QString> vectorsUrl;
    std::optional<QString> vectorsToken;
    std::optional<QString> syncUrl;
    std::optional<QString> syncToken;
    QStringList expandedFolders;

    enum class Outcome { Loaded, Fresh, Recovered };
    static QString defaultPath();
    static std::pair<Config, Outcome> load(const QString &path);
    bool save(const QString &path, QString *error = nullptr) const;
    QByteArray toJson() const;
    static Config fromJson(const QByteArray &json, bool *ok);
    bool operator==(const Config &) const = default;
};

// Where the servers are when nothing says otherwise: the brain-server
// container and the embedding model on mattnas, over the tailnet.
constexpr const char *DEFAULT_SERVER_URL = "http://mattnas:8082";
constexpr const char *DEFAULT_EMBEDDING_URL = "http://mattnas:8081";

} // namespace brain
