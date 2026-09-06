#include "backend.h"
#include "config.h"
#include "icons.h"
#include "palette.h"
#include "single.h"
#include "systemtheme.h"
#include "vaultserver.h"

#include <QCommandLineParser>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QKeySequence>
#include <QDir>
#include <QGuiApplication>
#include <QIcon>
#include <QQmlApplicationEngine>
#include <QQmlError>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QScreen>
#include <QTextStream>
#include <QTimer>

#include <functional>
#include <memory>

namespace {

// CLI verbs: `brain capture`, `brain search [titles|text]`, `brain sync`,
// `brain status`, `brain config <key> [value]`. Handled by the running
// instance when there is one, so a config change never gets overwritten by
// the copy the app holds in memory.
bool isVerb(const QString &arg) {
    return arg == u"capture" || arg == u"search" || arg == u"sync" || arg == u"status" || arg == u"config";
}

int runVerbLocally(const QStringList &args) {
    QTextStream out(stdout);
    const QString verb = args.first();
    if (verb == u"config") {
        auto [config, outcome] = brain::Config::load(brain::Config::defaultPath());
        Q_UNUSED(outcome);
        auto field = [&](const QString &key) -> std::optional<QString> * {
            if (key == u"sync_url") return &config.syncUrl;
            if (key == u"sync_token") return &config.syncToken;
            if (key == u"vectors_url") return &config.vectorsUrl;
            if (key == u"vectors_token") return &config.vectorsToken;
            if (key == u"embedding_url") return &config.embeddingUrl;
            if (key == u"vault") return &config.vault;
            return nullptr;
        };
        if (args.size() < 2) { out << "usage: brain config <key> [value]  keys: sync_url sync_token vectors_url vectors_token embedding_url vault\n"; return 2; }
        std::optional<QString> *slot = field(args[1]);
        if (!slot) { out << "unknown key " << args[1] << "\n"; return 2; }
        if (args.size() == 2) { out << (slot->has_value() ? **slot : QStringLiteral("(unset)")) << "\n"; return 0; }
        *slot = args[2];
        QString error;
        if (!config.save(brain::Config::defaultPath(), &error)) { out << "could not save: " << error << "\n"; return 1; }
        out << args[1] << " set\n";
        return 0;
    }
    if (verb == u"status") {
        auto [config, outcome] = brain::Config::load(brain::Config::defaultPath());
        Q_UNUSED(outcome);
        const QString server = config.syncUrl.value_or(QString::fromLatin1(brain::DEFAULT_SERVER_URL));
        out << "vault: " << config.vault.value_or(QStringLiteral("(none)")) << "\n";
        out << "server: " << server << (config.syncToken && !config.syncToken->isEmpty() ? " (token set)" : " (no token: sync off)") << "\n";
        int vectors = 0;
        QString error;
        if (brain::net::VaultServer::health(server, &vectors, &error)) out << "health: ok, " << vectors << " vectors held\n";
        else out << "health: " << error << "\n";
        out << "embedding: " << config.embeddingUrl.value_or(QString::fromLatin1(brain::DEFAULT_EMBEDDING_URL)) << "\n";
        return 0;
    }
    out << "brain is not running; start it first for `" << verb << "`\n";
    return 1;
}

} // namespace

int main(int argc, char *argv[]) {
    // Verbs are decided before anything can show a window.
    QStringList raw;
    for (int i = 1; i < argc; ++i) raw.append(QString::fromLocal8Bit(argv[i]));
    if (!raw.isEmpty() && isVerb(raw.first())) {
        QCoreApplication core(argc, argv);
        core.setApplicationName(QStringLiteral("brain"));
        if (const auto reply = SingleInstance::forward(raw)) {
            QTextStream(stdout) << reply->output << (reply->output.endsWith(u'\n') || reply->output.isEmpty() ? "" : "\n");
            return reply->ok ? 0 : 1;
        }
        return runVerbLocally(raw);
    }

    QGuiApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("brain"));
    // No setApplicationDisplayName() (doubles window titles on Wayland) and
    // no setDesktopFileName() (portal registration warns under the menu's
    // systemd scope). The app_id is the binary name.
    app.setWindowIcon(QIcon::fromTheme(QStringLiteral("brain")));

    QCommandLineParser parser;
    parser.addHelpOption();
    const QCommandLineOption screenOption(QStringLiteral("screen"), QStringLiteral("Open on the given screen id (notes, tags, inbox)."), QStringLiteral("id"));
    const QCommandLineOption grabOption(QStringLiteral("grab"), QStringLiteral("Render one frame to this PNG and exit."), QStringLiteral("file"));
    const QCommandLineOption infoOption(QStringLiteral("info"), QStringLiteral("Print the resolved theme, palette sample and text scale, then exit."));
    const QCommandLineOption actOption(QStringLiteral("act"), QStringLiteral("Enter a state before grabbing (comma-separated)."), QStringLiteral("name"));
    const QCommandLineOption vaultOption(QStringLiteral("vault"), QStringLiteral("Open this folder as the vault for this run only."), QStringLiteral("dir"));
    const QCommandLineOption demoOption(QStringLiteral("demo"), QStringLiteral("Open a throwaway vault of sample notes."));
    parser.addOption(screenOption);
    parser.addOption(grabOption);
    parser.addOption(infoOption);
    parser.addOption(actOption);
    parser.addOption(vaultOption);
    parser.addOption(demoOption);
    parser.process(app);

    QQuickStyle::setStyle(QStringLiteral("Basic"));

    Palette palette(&app);
    if (QScreen *screen = app.primaryScreen())
        palette.setPointsPerPixel(72.0 / screen->logicalDotsPerInch());

    SystemTheme systemTheme(&app);
    palette.setTextScale(systemTheme.textScale());
    QObject::connect(&systemTheme, &SystemTheme::textScaleChanged, &palette, &Palette::setTextScale);

    if (parser.isSet(infoOption)) {
        QTimer::singleShot(400, &app, [&palette]() {
            QTextStream(stdout) << "theme " << palette.themeName() << (palette.dark() ? " dark" : " light")
                << " scale " << palette.textScale() << " window " << palette.window().name()
                << " accent " << palette.accent().name() << " positive " << palette.positive().name()
                << " mono " << palette.monoFamily() << " sans " << palette.sansFamily() << "\n";
            QCoreApplication::exit(0);
        });
        return app.exec();
    }

    // A throwaway vault or a one-off folder: scratch runs neither listen nor
    // forward, and never touch the real config.
    const bool scratch = parser.isSet(grabOption) || parser.isSet(demoOption) || parser.isSet(vaultOption);
    if (scratch) {
        const QString scratchHome = QDir::tempPath() + QStringLiteral("/brain-scratch-") + QString::number(QCoreApplication::applicationPid());
        qputenv("XDG_CONFIG_HOME", (scratchHome + QStringLiteral("/config")).toLocal8Bit());
        qputenv("XDG_CACHE_HOME", (scratchHome + QStringLiteral("/cache")).toLocal8Bit());
        if (!qEnvironmentVariableIsSet("BRAIN_OFFLINE") && parser.isSet(grabOption)) qputenv("BRAIN_OFFLINE", "1");
        brain::Config config;
        // Scratch runs can point at a server through the environment, which
        // is how the sync path is exercised against a throwaway container.
        if (qEnvironmentVariableIsSet("BRAIN_SYNC_URL")) { config.syncUrl = qEnvironmentVariable("BRAIN_SYNC_URL"); config.vectorsUrl = config.syncUrl; }
        if (qEnvironmentVariableIsSet("BRAIN_SYNC_TOKEN")) { config.syncToken = qEnvironmentVariable("BRAIN_SYNC_TOKEN"); config.vectorsToken = config.syncToken; }
        if (qEnvironmentVariableIsSet("BRAIN_EMBEDDING_URL")) config.embeddingUrl = qEnvironmentVariable("BRAIN_EMBEDDING_URL");
        if (parser.isSet(vaultOption)) config.vault = QDir(parser.value(vaultOption)).absolutePath();
        else if (parser.isSet(demoOption) || parser.isSet(grabOption)) {
            const QString demo = scratchHome + QStringLiteral("/vault");
            QDir().mkpath(demo + QStringLiteral("/baking"));
            QDir().mkpath(demo + QStringLiteral("/projects"));
            QDir().mkpath(demo + QStringLiteral("/reading"));
            QDir().mkpath(demo + QStringLiteral("/work"));
            auto write = [&](const QString &rel, const QString &text) { QFile f(demo + u'/' + rel); QDir().mkpath(QFileInfo(f).absolutePath()); f.open(QIODevice::WriteOnly); f.write(text.toUtf8()); };
            write(QStringLiteral("baking/Sourdough.md"), QStringLiteral("---\ntags: [baking, project/brain]\naliases: [starter notes]\ncreated: 2026-08-14\nupdated: 2026-09-02\n---\n# Sourdough\n\nHydration is the whole game. Anything past 80% and the crumb opens up, but the dough stops holding a shape — see [[Bulk Fermentation]] before you push it, and log the result under #baking/notes.\n\n## What went wrong\n\nThe **starter** was cold, so the second rise never came and the bread came out flat. `21°C` is the floor.\n\n- Feed twice before a bake, not once.\n- Watch the dough, not the clock.\n- [x] buy rye flour\n- [ ] proof at 26°C and measure\n\n> A slack dough is not a wet dough.\n\n| Hydration | Crumb  |\n|-----------|--------|\n| 72%       | even   |\n| 84%       | open   |\n\n![[oven-spring.jpg]]\n"));
            write(QStringLiteral("baking/Bulk Fermentation.md"), QStringLiteral("# Bulk Fermentation\n\nGrown by half and holds a dome. Reheat the kitchen before you blame the [[Sourdough]].\n\n#baking\n"));
            write(QStringLiteral("baking/Starter Log.md"), QStringLiteral("# Starter Log\n\nFed 1:2:2 at 21°C, the way [[Sourdough|sourdough]] says to.\n\n- [ ] weigh the discard\n"));
            write(QStringLiteral("baking/Oven Notes.md"), QStringLiteral("# Oven Notes\n\nSteam twelve minutes, then vent. The stone wants forty minutes at full heat first.\n"));
            write(QStringLiteral("baking/Starter Ratios.md"), QStringLiteral("# Starter Ratios\n\n1:1:1 for a fast rise, 1:5:5 overnight.\n"));
            write(QStringLiteral("baking/Flour.md"), QStringLiteral("# Flour\n\nStrong white for structure, a fifth rye for flavour.\n"));
            write(QStringLiteral("projects/Brain.md"), QStringLiteral("# Brain\n\nA notebook that is a folder of Markdown. See [[Vault Conventions]].\n\n#project/brain\n"));
            write(QStringLiteral("reading/Salt Fat Acid Heat.md"), QStringLiteral("# Salt Fat Acid Heat\n\nThe four things every dish balances.\n\n#reading\n"));
            write(QStringLiteral("work/Standup.md"), QStringLiteral("# Standup\n\n1. Hold the release until the deadline.\n2. Renew the passport.\n"));
            write(QStringLiteral("Vault Conventions.md"), QStringLiteral("# Vault Conventions\n\nTitles are filenames. Tags nest with a slash. Attachments live in `attachments/`.\n"));
            write(QStringLiteral("Inbox.md"), QStringLiteral("# Inbox\n\n- call the mill about rye\n- [[Starter Log]] needs dates\n"));
            config.vault = demo;
            config.lastNote = QStringLiteral("baking/Sourdough.md");
        }
        QDir().mkpath(scratchHome + QStringLiteral("/config/brain"));
        config.save(scratchHome + QStringLiteral("/config/brain/config.json"));
    }

    SingleInstance single([](const QStringList &args) -> SingleInstance::Reply {
        Backend *backend = Backend::instance();
        if (!backend) return {QStringLiteral("not ready"), false};
        return {backend->handleVerb(args), true};
    });
    if (!scratch) {
        if (SingleInstance::forward({})) return 0;
        single.listen();
    }

    Backend backend(&palette, &app);
    if (parser.isSet(screenOption)) backend.go(parser.value(screenOption));
    // `type:text` and `key:Ctrl+Z` steps are real key events posted to the
    // window once it is up, so a grab can show the editor after typing.
    QStringList steps;
    for (const QString &act : parser.value(actOption).split(u',', Qt::SkipEmptyParts)) {
        if (act.startsWith(QStringLiteral("type:")) || act.startsWith(QStringLiteral("key:")) || act.startsWith(QStringLiteral("click:"))) steps.append(act);
        else backend.act(act);
    }

    QQmlApplicationEngine engine;
    engine.addImageProvider(QStringLiteral("icon"), new IconProvider);
    QObject::connect(&engine, &QQmlApplicationEngine::warnings, &app, [](const QList<QQmlError> &warnings) {
        for (const QQmlError &warning : warnings) qWarning().noquote() << warning.toString();
    });
    engine.loadFromModule(QStringLiteral("Brain"), QStringLiteral("Main"));
    if (engine.rootObjects().isEmpty()) {
        qCritical() << "Could not load the interface";
        return -1;
    }
    auto *window = qobject_cast<QQuickWindow *>(engine.rootObjects().first());
    QObject::connect(&backend, &Backend::windowRequested, &app, [window]() {
        if (window) { window->show(); window->requestActivate(); }
    });
    QObject::connect(&app, &QGuiApplication::aboutToQuit, &backend, &Backend::flushAndSave);
    auto runSteps = std::make_shared<std::function<void(int)>>();
    *runSteps = [window, steps, runSteps, &backend](int i) {
        if (i == 0) emit backend.editorFocusRequested();
        if (i >= steps.size()) return;
        const QString step = steps.at(i);
        if (step.startsWith(QStringLiteral("click:"))) {
            // click:x;y[;right][;double] — window coordinates at design scale.
            const QStringList parts = step.mid(6).split(u';');
            const QPointF at(parts.value(0).toDouble(), parts.value(1).toDouble());
            const Qt::MouseButton button = parts.contains(QStringLiteral("right")) ? Qt::RightButton : Qt::LeftButton;
            const int times = parts.contains(QStringLiteral("double")) ? 2 : 1;
            for (int n = 0; n < times; ++n) {
                QCoreApplication::postEvent(window, new QMouseEvent(n == 1 ? QEvent::MouseButtonDblClick : QEvent::MouseButtonPress, at, at, window->mapToGlobal(at.toPoint()), button, button, Qt::NoModifier));
                QCoreApplication::postEvent(window, new QMouseEvent(QEvent::MouseButtonRelease, at, at, window->mapToGlobal(at.toPoint()), button, Qt::NoButton, Qt::NoModifier));
            }
        } else if (step.startsWith(QStringLiteral("type:"))) {
            for (const QChar c : step.mid(5)) {
                QCoreApplication::postEvent(window, new QKeyEvent(QEvent::KeyPress, 0, Qt::NoModifier, QString(c)));
                QCoreApplication::postEvent(window, new QKeyEvent(QEvent::KeyRelease, 0, Qt::NoModifier, QString(c)));
            }
        } else {
            const QKeySequence sequence = QKeySequence::fromString(step.mid(4));
            const QKeyCombination combo = sequence.isEmpty() ? QKeyCombination(Qt::Key_unknown) : sequence[0];
            QCoreApplication::postEvent(window, new QKeyEvent(QEvent::KeyPress, combo.key(), combo.keyboardModifiers()));
            QCoreApplication::postEvent(window, new QKeyEvent(QEvent::KeyRelease, combo.key(), combo.keyboardModifiers()));
        }
        QTimer::singleShot(60, window, [runSteps, i]() { (*runSteps)(i + 1); });
    };
    if (!steps.isEmpty()) QTimer::singleShot(250, window, [runSteps]() { (*runSteps)(0); });
    if (parser.isSet(grabOption)) {
        const QString file = parser.value(grabOption);
        const int grabDelay = qEnvironmentVariableIsSet("BRAIN_GRAB_DELAY_MS") ? qEnvironmentVariableIntValue("BRAIN_GRAB_DELAY_MS") : 600;
        QTimer::singleShot(grabDelay + 80 * steps.size(), &app, [window, file, &backend]() {
            const bool ok = window && window->grabWindow().save(file);
            if (!ok) qCritical() << "grab failed:" << file;
            if (qEnvironmentVariableIsSet("BRAIN_DEBUG_EDITOR")) QTextStream(stdout) << "--- source ---\n" << backend.editor()->source() << "\n--- caret " << backend.editor()->caret() << " display " << (backend.editor()->textEdit() ? backend.editor()->textEdit()->property("cursorPosition").toInt() : -1) << " expected " << backend.editor()->displayOffset(backend.editor()->caret()) << " ---\n";
            QCoreApplication::exit(ok ? 0 : 2);
        });
    }
    return app.exec();
}
