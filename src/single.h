#pragma once

#include <QLocalServer>
#include <QObject>
#include <QStringList>

#include <functional>
#include <optional>

// One running instance. A second launch hands its arguments over a local
// socket: with none, the first raises its window; with a CLI verb, the
// running app answers, so a store held in memory is never written behind its
// back. Scratch runs (--demo, --data, --grab) must not listen.
class SingleInstance : public QObject {
public:
    // $XDG_RUNTIME_DIR/brain.sock; brain_SOCKET_SUFFIX isolates tests.
    static QString socketPath();
    struct Reply { QString output; bool ok; };
    // The reply if an instance answered, nothing if there is none.
    static std::optional<Reply> forward(const QStringList &args, int timeoutMs = 700);

    explicit SingleInstance(std::function<Reply(const QStringList &)> handler, QObject *parent = nullptr);
    bool listen();

private:
    std::function<Reply(const QStringList &)> m_handler;
    QLocalServer m_server;
};
