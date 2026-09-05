#pragma once

// The `---` metadata block at the top of a note. Four keys are understood —
// tags, aliases, created, updated — and everything else is preserved verbatim.
// Read a note and write it back without editing it and the bytes are
// identical: a line is re-rendered canonically only when its value changed.

#include <QDate>
#include <QString>
#include <QStringList>
#include <QVector>

#include <optional>

namespace brain {

class Frontmatter {
public:
    QStringList tags;
    QStringList aliases;
    std::optional<QDate> created;
    std::optional<QDate> updated;

    // Split a note into its metadata block and its body. The body starts at
    // the first character after the closing delimiter's newline.
    static std::pair<std::optional<Frontmatter>, QString> split(const QString &text);

    // The block as text, delimiters included, ending in a newline. Empty
    // frontmatter renders as the empty string.
    QString render() const;
    bool isEmpty() const;
    bool operator==(const Frontmatter &other) const;

private:
    enum class Key { Tags, Aliases, Created, Updated };
    struct Entry {
        bool known = false;
        Key key = Key::Tags;
        QStringList lines;
    };
    struct Original {
        QStringList tags, aliases;
        std::optional<QDate> created, updated;
    };

    static std::optional<Key> parseKey(const QString &name);
    static QString keyName(Key key);
    static QVector<Entry> group(const QStringList &raw);
    static std::optional<QString> keyNameOf(const QString &line);
    static bool isContinuation(const QString &line);
    static QStringList listValue(const QStringList &lines);
    static std::optional<QDate> dateValue(const QStringList &lines);
    static QString clean(const QString &value);

    void readValues();
    bool changed(Key key) const;
    bool isSet(Key key) const;
    std::optional<QString> lineFor(Key key) const;

    QVector<Entry> m_entries;
    std::optional<Original> m_original;
    QString m_open = QStringLiteral("---");
    std::optional<QString> m_close;
    bool m_closeNewline = true;
};

} // namespace brain
