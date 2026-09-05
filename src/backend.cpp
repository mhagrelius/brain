#include "backend.h"

#include "embedder.h"
#include "palette.h"
#include "vaultserver.h"

#include <QBuffer>
#include <QClipboard>
#include <QDateTime>
#include <QDesktopServices>
#include <QDir>
#include <QGuiApplication>
#include <QImage>
#include <QMimeData>
#include <QMutex>
#include <QMutexLocker>
#include <QStandardPaths>
#include <QSysInfo>
#include <QUrl>

#include <thread>

using namespace brain;

Backend *Backend::s_instance = nullptr;

namespace {
QMutex g_instanceMutex;
constexpr int TICK_MS = 2000;
constexpr int CATCH_UP_DELAY_MS = 5000;
constexpr int SYNC_EVERY_MS = 60000;

QString machineName() {
    QString cleaned;
    for (QChar c : QSysInfo::machineHostName()) if (c.isLetterOrNumber() || c == u'-' || c == u'_') cleaned.append(c);
    return cleaned.isEmpty() ? QStringLiteral("another machine") : cleaned;
}

QString today() { return QDate::currentDate().toString(QStringLiteral("yyyy-MM-dd")); }

QString tilde(const QString &path) {
    const QString home = QDir::homePath();
    if (path == home) return QStringLiteral("~");
    if (path.startsWith(home + u'/')) return u'~' + path.mid(home.size());
    return path;
}

QString initialOf(const QString &title) {
    for (QChar c : title) if (c.isLetterOrNumber()) return QString(c).toUpper();
    return title.isEmpty() ? QStringLiteral("·") : title.left(1);
}
}

Backend::Backend(Palette *palette, QObject *parent) : QObject(parent), m_palette(palette), m_editor(new Editor(palette, this)) {
    {
        QMutexLocker lock(&g_instanceMutex);
        s_instance = this;
    }
    m_machine = machineName();
    m_notebook.loadConfig(Config::defaultPath());
    m_reading = m_notebook.config().readingMode;
    m_editor->setReading(m_reading);
    if (m_notebook.config().vault && QDir(*m_notebook.config().vault).exists()) {
        m_notebook.setVault(*m_notebook.config().vault);
        // setVault forgets the last note on purpose (a vault switch); a
        // launch is not a switch, so put it back.
        m_notebook.config() = Config::load(Config::defaultPath()).first;
        m_notebook.restoreLastNote();
    }
    m_editor->setVaultRoot(m_notebook.vaultRoot());
    loadOpenNoteIntoEditor();
    watchVault();

    connect(m_editor, &Editor::edited, this, &Backend::onEdited);
    connect(m_editor, &Editor::sourceChanged, this, [this]() { recompute(); });
    connect(palette, &Palette::changed, this, [this]() { m_editor->refreshStyles(); recompute(); });
    connect(palette, &Palette::textScaleChanged, this, [this]() { m_editor->refreshStyles(); recompute(); });

    m_tick.setInterval(TICK_MS);
    connect(&m_tick, &QTimer::timeout, this, &Backend::onTick);
    m_tick.start();
    m_watchDebounce.setSingleShot(true);
    m_watchDebounce.setInterval(500);
    connect(&m_watchDebounce, &QTimer::timeout, this, &Backend::onVaultChanged);
    connect(&m_watcher, &QFileSystemWatcher::directoryChanged, this, [this](const QString &) { m_watchDebounce.start(); });
    connect(&m_watcher, &QFileSystemWatcher::fileChanged, this, [this](const QString &) { m_watchDebounce.start(); });
    m_catchUpTimer.setSingleShot(true);
    connect(&m_catchUpTimer, &QTimer::timeout, this, &Backend::runCatchUp);
    m_syncTimer.setInterval(SYNC_EVERY_MS);
    connect(&m_syncTimer, &QTimer::timeout, this, &Backend::runSync);
    m_toastTimer.setSingleShot(true);
    connect(&m_toastTimer, &QTimer::timeout, this, [this]() {
        const qint64 now = QDateTime::currentMSecsSinceEpoch();
        QVariantList kept;
        for (const QVariant &v : m_toasts) {
            const QVariantMap t = v.toMap();
            const qint64 until = t.value(QStringLiteral("until")).toLongLong();
            if (until == 0 || until > now) kept.append(t);
        }
        if (kept.size() != m_toasts.size()) { m_toasts = kept; emit changed(); }
        refreshToastsSoon();
    });
    m_queryTimer.setSingleShot(true);
    m_queryTimer.setInterval(250);
    connect(&m_queryTimer, &QTimer::timeout, this, [this]() { if (!m_pendingQuery.isEmpty()) runQueryEmbedding(m_pendingQuery); });

    recompute();
    if (qEnvironmentVariable("BRAIN_OFFLINE") != QLatin1String("1")) {
        scheduleCatchUp(CATCH_UP_DELAY_MS);
        m_syncTimer.start();
        QTimer::singleShot(3000, this, &Backend::runSync);
    }
}

Backend::~Backend() {
    QMutexLocker lock(&g_instanceMutex);
    s_instance = nullptr;
}

Backend *Backend::create(QQmlEngine *, QJSEngine *) {
    Q_ASSERT(s_instance);
    QJSEngine::setObjectOwnership(s_instance, QJSEngine::CppOwnership);
    return s_instance;
}

void Backend::runOnMain(std::function<void()> fn) {
    QMutexLocker lock(&g_instanceMutex);
    if (!s_instance) return;
    QMetaObject::invokeMethod(s_instance, [fn = std::move(fn)]() { fn(); }, Qt::QueuedConnection);
}

// ---- persistence and the tick -------------------------------------------------

void Backend::flushEditor() {
    if (!m_notebook.openNoteId()) return;
    m_notebook.flushBody(m_editor->source());
}

void Backend::flushAndSave() {
    flushEditor();
    QString error;
    if (m_notebook.saveNow(&error) == Notebook::Saved::Failed) toast(ToastKind::Info, QStringLiteral("Could not save"), error, QString(), QString(), 8000);
    m_notebook.config().readingMode = m_reading;
    m_notebook.saveConfig();
}

void Backend::onTick() {
    flushEditor();
    if (!m_notebook.isDirty()) return;
    QString error;
    const Notebook::Saved saved = m_notebook.saveNow(&error);
    if (saved == Notebook::Saved::Failed) {
        bool shown = false;
        for (const QVariant &v : m_toasts) if (v.toMap().value(QStringLiteral("key")).toString() == u"notsaving") shown = true;
        if (!shown) {
            const int id = toast(ToastKind::Info, QStringLiteral("Not saving"), error, QString(), QString(), 0);
            QVariantMap t = m_toasts.last().toMap();
            t.insert(QStringLiteral("key"), QStringLiteral("notsaving"));
            m_toasts[m_toasts.size() - 1] = t;
            Q_UNUSED(id);
        }
    } else if (saved == Notebook::Saved::Written) {
        QVariantList kept;
        for (const QVariant &v : m_toasts) if (v.toMap().value(QStringLiteral("key")).toString() != u"notsaving") kept.append(v);
        m_toasts = kept;
        scheduleCatchUp(CATCH_UP_DELAY_MS);
        recompute();
    }
}

void Backend::onEdited() {
    m_notebook.markEdited();
}

void Backend::saveNow() {
    flushAndSave();
    setOsd(QStringLiteral("saved"), QStringLiteral("Saved"), QString(), -1, 1200);
    recompute();
}

// ---- the vault and the watcher ------------------------------------------------

void Backend::watchVault() {
    if (!m_watcher.directories().isEmpty()) m_watcher.removePaths(m_watcher.directories());
    if (!m_watcher.files().isEmpty()) m_watcher.removePaths(m_watcher.files());
    if (const Vault *vault = m_notebook.vault()) m_watcher.addPaths(vault->directories());
}

void Backend::onVaultChanged() {
    flushEditor();
    const External e = m_notebook.absorbExternalChanges();
    watchVault();
    switch (e.kind) {
    case External::Reloaded:
        loadOpenNoteIntoEditor(m_editor->caret());
        break;
    case External::Diverged:
        toast(ToastKind::Conflict, e.id.title() + QStringLiteral(".md changed on disk"), QStringLiteral("Your edits are still here. Keep them, or take the file."), QStringLiteral("Take the file"), QStringLiteral("Keep mine"), 0);
        break;
    case External::Vanished:
        toast(ToastKind::Vanished, e.id.title() + QStringLiteral(".md was deleted on disk"), QStringLiteral("The editor still holds it."), QStringLiteral("Put it back"), QStringLiteral("Let it go"), 0);
        break;
    case External::Quiet:
        break;
    }
    scheduleCatchUp(CATCH_UP_DELAY_MS);
    recompute();
}

void Backend::chooseVault(const QString &raw) {
    QString path = raw.trimmed();
    if (path.startsWith(QStringLiteral("file://"))) path = QUrl(path).toLocalFile();
    if (path.startsWith(u'~')) path = QDir::homePath() + path.mid(1);
    if (path.isEmpty()) return;
    if (!QDir().mkpath(path)) { toast(ToastKind::Info, QStringLiteral("Could not create ") + path, QString()); return; }
    flushAndSave();
    ++m_generation;
    m_notebook.setVault(path);
    m_notebook.saveConfig();
    m_editor->setVaultRoot(m_notebook.vaultRoot());
    loadOpenNoteIntoEditor();
    watchVault();
    scheduleCatchUp(1500);
    QTimer::singleShot(1500, this, &Backend::runSync);
    recompute();
}

QString Backend::suggestedVault() const {
    const QString notes = QDir::homePath() + QStringLiteral("/notes");
    return tilde(notes);
}

// ---- navigation ---------------------------------------------------------------

void Backend::setScreen(const QString &screen) {
    if (m_screen == screen) return;
    m_screen = screen;
    if (screen != u"tags") m_notebook.filterByTag(std::nullopt);
    recompute();
}

void Backend::goToKey(int key) {
    static const QStringList ids = {QStringLiteral("notes"), QStringLiteral("tags"), QStringLiteral("inbox")};
    if (key >= 1 && key <= ids.size()) setScreen(ids.at(key - 1));
}

void Backend::openNote(const QString &id) {
    if (id.isEmpty()) return;
    const NoteId target = NoteId::fromRelative(id);
    if (m_notebook.openNoteId() && *m_notebook.openNoteId() == target) return;
    flushAndSave();
    VaultError error;
    if (!m_notebook.loadNote(target, &error)) { toast(ToastKind::Info, QStringLiteral("Could not open ") + target.title(), error.message); recompute(); return; }
    loadOpenNoteIntoEditor();
    recompute();
    emit editorFocusRequested();
}

void Backend::loadOpenNoteIntoEditor(int caret) {
    const Note *note = m_notebook.openNote();
    m_editor->load(note ? note->body : QString(), caret);
}

void Backend::toggleFolder(const QString &path) {
    m_notebook.toggleFolder(path);
    m_notebook.saveConfig();
    recompute();
}

void Backend::setQuery(const QString &text) {
    if (m_notebook.setQuery(text)) recompute();
}

void Backend::cycleSort() {
    switch (m_notebook.sort()) {
    case tree::Sort::Name: m_notebook.setSort(tree::Sort::Modified); break;
    case tree::Sort::Modified: m_notebook.setSort(tree::Sort::Created); break;
    case tree::Sort::Created: m_notebook.setSort(tree::Sort::Name); break;
    }
    m_notebook.saveConfig();
    recompute();
}

QString Backend::sortLabel() const {
    switch (m_notebook.sort()) {
    case tree::Sort::Name: return QStringLiteral("name ↓");
    case tree::Sort::Modified: return QStringLiteral("written ↓");
    case tree::Sort::Created: return QStringLiteral("made ↓");
    }
    return QString();
}

void Backend::filterTag(const QString &tag) {
    if (tag.isEmpty() || (m_notebook.activeTag() && *m_notebook.activeTag() == tag.toLower())) m_notebook.filterByTag(std::nullopt);
    else { m_notebook.filterByTag(tag); if (m_screen != u"tags") m_screen = QStringLiteral("tags"); }
    recompute();
}

// ---- notes and folders --------------------------------------------------------

void Backend::newNote() { newNoteIn(m_notebook.currentFolder()); }

void Backend::newNoteIn(const QString &folder) {
    if (!m_notebook.hasVault()) return;
    openPrompt(QStringLiteral("new-note"), folder.isEmpty() ? QStringLiteral("New note") : QStringLiteral("New note in ") + folder, QString(), QStringLiteral("Write it"), folder);
}

void Backend::newFolderIn(const QString &parent) {
    if (!m_notebook.hasVault()) return;
    openPrompt(QStringLiteral("new-folder"), parent.isEmpty() ? QStringLiteral("New folder") : QStringLiteral("New folder in ") + parent, QString(), QStringLiteral("Make it"), parent);
}

void Backend::askRenameNote(const QString &id) {
    const NoteId note = NoteId::fromRelative(id);
    openNote(id);
    openPrompt(QStringLiteral("rename-note"), QStringLiteral("Rename note"), note.title(), QStringLiteral("Rename"), id);
}

void Backend::askDeleteNote(const QString &id) {
    const NoteId note = NoteId::fromRelative(id);
    openPrompt(QStringLiteral("delete-note"), QStringLiteral("Delete “") + note.title() + QStringLiteral("”?"), QString(), QStringLiteral("Delete"), id, true);
}

void Backend::askRenameFolder(const QString &path) {
    openPrompt(QStringLiteral("rename-folder"), QStringLiteral("Rename folder"), path.section(u'/', -1), QStringLiteral("Rename"), path);
}

void Backend::askDeleteFolder(const QString &path) {
    openPrompt(QStringLiteral("delete-folder"), QStringLiteral("Remove folder “") + path + QStringLiteral("”?"), QString(), QStringLiteral("Remove"), path, true);
}

void Backend::openPrompt(const QString &kind, const QString &title, const QString &value, const QString &primary, const QString &target, bool danger) {
    m_prompt = QVariantMap{{QStringLiteral("kind"), kind}, {QStringLiteral("title"), title}, {QStringLiteral("value"), value}, {QStringLiteral("primary"), primary}, {QStringLiteral("target"), target}, {QStringLiteral("danger"), danger}, {QStringLiteral("hasField"), !kind.startsWith(QStringLiteral("delete"))}};
    emit changed();
}

void Backend::promptCancel() {
    m_prompt.clear();
    emit changed();
    emit editorFocusRequested();
}

void Backend::promptAccept(const QString &rawValue) {
    const QVariantMap prompt = m_prompt;
    m_prompt.clear();
    const QString kind = prompt.value(QStringLiteral("kind")).toString();
    const QString target = prompt.value(QStringLiteral("target")).toString();
    QString value = rawValue.trimmed();
    value.replace(u'/', u'-');
    QString error;
    if (kind == u"new-note") {
        if (value.isEmpty()) value = QStringLiteral("Untitled");
        flushAndSave();
        if (m_notebook.createNoteIn(target, value, &error)) { loadOpenNoteIntoEditor(); emit editorFocusRequested(); }
        else toast(ToastKind::Info, QStringLiteral("Could not create the note"), error);
    } else if (kind == u"write-link") {
        if (value.isEmpty()) value = prompt.value(QStringLiteral("value")).toString().trimmed();
        if (!value.isEmpty()) {
            flushAndSave();
            if (m_notebook.createNoteIn(target, value, &error)) { loadOpenNoteIntoEditor(); emit editorFocusRequested(); }
            else toast(ToastKind::Info, QStringLiteral("Could not create the note"), error);
        }
    } else if (kind == u"new-folder") {
        if (!value.isEmpty() && !m_notebook.createFolder(target, value, &error)) toast(ToastKind::Info, QStringLiteral("Could not make the folder"), error);
        watchVault();
    } else if (kind == u"rename-note") {
        if (!value.isEmpty()) {
            flushAndSave();
            const Notebook::Renamed r = m_notebook.renameNote(value);
            if (r.kind == Notebook::Renamed::Failed) toast(ToastKind::Info, QStringLiteral("Could not rename"), r.error);
            else if (r.kind == Notebook::Renamed::Done && r.links > 0) toast(ToastKind::Success, QStringLiteral("Renamed"), QString::number(r.links) + (r.links == 1 ? QStringLiteral(" link repointed.") : QStringLiteral(" links repointed.")));
        }
    } else if (kind == u"delete-note") {
        const NoteId id = NoteId::fromRelative(target);
        if (m_notebook.openNoteId() && *m_notebook.openNoteId() == id) {
            const auto gone = m_notebook.deleteOpenNote(&error);
            if (!gone) toast(ToastKind::Info, QStringLiteral("Could not delete"), error);
            loadOpenNoteIntoEditor();
        } else if (const Vault *vault = m_notebook.vault()) {
            VaultError problem;
            if (vault->remove(id, &problem)) m_notebook.rescan();
            else toast(ToastKind::Info, QStringLiteral("Could not delete"), problem.message);
        }
        scheduleCatchUp(CATCH_UP_DELAY_MS);
    } else if (kind == u"rename-folder") {
        if (!value.isEmpty() && !m_notebook.renameFolder(target, value, &error)) toast(ToastKind::Info, QStringLiteral("Could not rename the folder"), error);
        else loadOpenNoteIntoEditor(m_editor->caret());
        watchVault();
    } else if (kind == u"delete-folder") {
        if (!m_notebook.deleteFolder(target, &error)) toast(ToastKind::Info, QStringLiteral("The folder stays"), error, QString(), QString(), 6000);
        watchVault();
    }
    m_notebook.saveConfig();
    recompute();
    if (kind != u"new-note") emit editorFocusRequested();
}

void Backend::moveNote(const QString &id, const QString &folder) {
    flushAndSave();
    const Notebook::Moved m = m_notebook.moveNote(NoteId::fromRelative(id), folder);
    if (m.kind == Notebook::Moved::Failed) toast(ToastKind::Info, QStringLiteral("Could not move"), m.error);
    m_notebook.saveConfig();
    recompute();
}

void Backend::moveFolder(const QString &from, const QString &into) {
    if (from.isEmpty() || tree::isWithin(from, into)) return;
    flushAndSave();
    const QString to = into.isEmpty() ? from.section(u'/', -1) : into + u'/' + from.section(u'/', -1);
    QString error;
    if (!m_notebook.relocateFolder(from, to, &error)) toast(ToastKind::Info, QStringLiteral("Could not move the folder"), error);
    else loadOpenNoteIntoEditor(m_editor->caret());
    watchVault();
    m_notebook.saveConfig();
    recompute();
}

// ---- the editor ---------------------------------------------------------------

void Backend::toggleReading() {
    m_reading = !m_reading;
    m_editor->setReading(m_reading);
    m_notebook.config().readingMode = m_reading;
    m_notebook.saveConfig();
    if (m_reading && m_insertOpen) m_insertOpen = false;
    setOsd(QStringLiteral("reading"), m_reading ? QStringLiteral("Reading mode") : QStringLiteral("Editing"), QStringLiteral("ctrl+e"), -1, 1600);
    recompute();
}

void Backend::followLink(const QString &target) {
    const Resolution r = m_notebook.resolveLink(target);
    if (r.kind == Resolution::NoteFound) { openNote(r.note.str()); return; }
    if (r.kind == Resolution::Ambiguous) {
        QStringList names;
        for (const NoteId &id : r.candidates) names.append(id.str());
        toast(ToastKind::Info, QStringLiteral("“") + target + QStringLiteral("” could mean ") + QString::number(r.candidates.size()) + QStringLiteral(" notes"), names.join(QStringLiteral(" · ")), QString(), QString(), 6000);
        return;
    }
    openPrompt(QStringLiteral("write-link"), QStringLiteral("“") + target + QStringLiteral("” does not exist yet"), target, QStringLiteral("Write it"), m_notebook.currentFolder());
}

void Backend::openUrl(const QString &url) { QDesktopServices::openUrl(QUrl(url)); }

void Backend::attachUrls(const QStringList &urls) {
    QStringList paths;
    for (const QString &u : urls) { const QUrl url(u); paths.append(url.isLocalFile() ? url.toLocalFile() : u); }
    QStringList failed;
    const QStringList names = m_notebook.attachFiles(paths, &failed);
    for (const QString &name : names) m_editor->insertEmbed(name);
    if (!failed.isEmpty()) toast(ToastKind::Info, QStringLiteral("Could not attach"), failed.join(QStringLiteral(", ")));
}

bool Backend::pasteImage() {
    const QMimeData *mime = QGuiApplication::clipboard()->mimeData();
    if (!mime || !mime->hasImage()) return false;
    const QImage image = qvariant_cast<QImage>(mime->imageData());
    if (image.isNull()) return false;
    QByteArray bytes;
    QBuffer buffer(&bytes);
    buffer.open(QIODevice::WriteOnly);
    image.save(&buffer, "PNG");
    const QString name = QStringLiteral("pasted-") + QDateTime::currentDateTime().toString(QStringLiteral("yyyyMMdd-HHmmss")) + QStringLiteral(".png");
    const auto stored = m_notebook.attachBytes(name, bytes);
    if (!stored) return false;
    m_editor->insertEmbed(*stored);
    return true;
}

namespace {
QStringList splitList(const QString &text) {
    QStringList out;
    for (const QString &part : text.split(u',', Qt::SkipEmptyParts)) {
        QString t = part.trimmed();
        while (t.startsWith(u'#')) t.remove(0, 1);
        if (!t.isEmpty() && !out.contains(t)) out.append(t);
    }
    return out;
}
}

void Backend::setNoteTags(const QString &text) {
    m_notebook.setOpenTags(splitList(text));
    recompute();
}

void Backend::setNoteAliases(const QString &text) {
    m_notebook.setOpenAliases(splitList(text));
    recompute();
}

// ---- capture ------------------------------------------------------------------

void Backend::openCapture() {
    if (!m_notebook.hasVault()) return;
    m_captureOpen = true;
    m_captureTarget = 0;
    emit changed();
    emit captureRequested();
}

void Backend::closeCapture() {
    m_captureOpen = false;
    emit changed();
    emit editorFocusRequested();
}

void Backend::captureCycleDestination() {
    m_captureTarget = (m_captureTarget + 1) % 2;
    if (m_captureTarget == 1 && !hasNote()) m_captureTarget = 0;
    emit changed();
}

QString Backend::captureDestination() const {
    if (m_captureTarget == 1 && m_notebook.openNoteId()) return m_notebook.openNoteId()->str();
    return QStringLiteral("Inbox.md");
}

void Backend::capture(const QString &rawText, bool openAfter) {
    const QString text = rawText.trimmed();
    m_captureOpen = false;
    if (text.isEmpty()) { emit changed(); return; }
    const NoteId id = NoteId::fromRelative(captureDestination());
    const QString line = text.startsWith(QStringLiteral("- ")) ? text : QStringLiteral("- ") + text;
    QString error;
    if (!m_notebook.appendTo(id, line, &error)) toast(ToastKind::Info, QStringLiteral("Could not capture"), error);
    else if (m_notebook.openNoteId() && *m_notebook.openNoteId() == id) loadOpenNoteIntoEditor(m_editor->caret());
    if (openAfter) openNote(id.str());
    scheduleCatchUp(CATCH_UP_DELAY_MS);
    recompute();
    if (!openAfter) emit editorFocusRequested();
}

// ---- the insert menu ----------------------------------------------------------

void Backend::openInsert() {
    if (m_reading || !hasNote()) return;
    m_insertOpen = true;
    m_insertQuery.clear();
    refreshInsert();
    emit changed();
}

void Backend::closeInsert() {
    m_insertOpen = false;
    emit changed();
    emit editorFocusRequested();
}

void Backend::insertFilter(const QString &text) {
    m_insertQuery = text;
    refreshInsert();
    emit changed();
}

void Backend::refreshInsert() {
    m_insertRows.clear();
    m_recentRows.clear();
    const QString q = m_insertQuery.trimmed().toLower();
    auto rowFor = [&](md::Format f, const QVector<int> &positions) {
        return QVariantMap{{QStringLiteral("label"), md::labelOf(f)}, {QStringLiteral("syntax"), md::syntaxOf(f)}, {QStringLiteral("key"), md::keybindOf(f)}, {QStringLiteral("positions"), QVariant::fromValue(QVariantList(positions.begin(), positions.end()))}};
    };
    struct Scored { int score; md::Format format; QVector<int> positions; };
    QVector<Scored> scored;
    for (md::Format f : md::allFormats()) {
        const QString label = md::labelOf(f);
        if (q.isEmpty()) { scored.append({0, f, {}}); continue; }
        // The same subsequence scoring the title search uses.
        QVector<int> positions;
        int at = 0, score = 0;
        bool ok = true;
        for (int i = 0; i < q.size() && ok; ++i) {
            int found = -1;
            for (int j = at; j < label.size(); ++j) if (label[j].toLower() == q[i]) { found = j; break; }
            if (found < 0) { ok = false; break; }
            if (found == 0) score += 15; else if (!label[found - 1].isLetterOrNumber()) score += 10;
            if (i > 0 && positions.last() == found - 1) score += 8;
            score -= found - at;
            positions.append(found);
            at = found + 1;
        }
        if (ok) scored.append({score, f, positions});
    }
    std::stable_sort(scored.begin(), scored.end(), [](const Scored &a, const Scored &b) { return a.score > b.score; });
    for (const Scored &s : scored) m_insertRows.append(rowFor(s.format, s.positions));
    if (q.isEmpty()) {
        for (const QString &label : m_recent)
            for (md::Format f : md::allFormats()) if (md::labelOf(f) == label) m_recentRows.append(rowFor(f, {}));
    }
}

void Backend::applyInsert(const QString &label, int selectionStart, int selectionEnd) {
    m_insertOpen = false;
    m_editor->applyFormat(label, selectionStart, selectionEnd);
    m_recent.removeAll(label);
    m_recent.prepend(label);
    while (m_recent.size() > 3) m_recent.removeLast();
    recompute();
    emit editorFocusRequested();
}

// ---- search -------------------------------------------------------------------

void Backend::openSearch(const QString &mode) {
    m_searchOpen = true;
    m_searchMode = mode == u"text" ? Notebook::Mode::Text : Notebook::Mode::Title;
    m_searchQuery.clear();
    refreshHits();
    emit changed();
}

void Backend::closeSearch() {
    m_searchOpen = false;
    emit changed();
    emit editorFocusRequested();
}

void Backend::searchToggleMode() {
    m_searchMode = m_searchMode == Notebook::Mode::Title ? Notebook::Mode::Text : Notebook::Mode::Title;
    refreshHits();
    emit changed();
}

void Backend::setSearchQuery(const QString &text) {
    m_searchQuery = text;
    refreshHits();
    emit changed();
}

void Backend::refreshHits() {
    bool wants = false;
    const QVector<Hit> hits = m_notebook.search(m_searchQuery, m_searchMode, &wants);
    m_hits.clear();
    for (const Hit &h : hits) {
        QVariantMap row{{QStringLiteral("id"), h.id}, {QStringLiteral("title"), h.title}, {QStringLiteral("initial"), initialOf(h.title)}, {QStringLiteral("score"), h.score}};
        const NoteId id = NoteId::fromRelative(h.id);
        QString detail = h.detail;
        if (m_searchMode == Notebook::Mode::Title) {
            detail = id.folder().isEmpty() ? m_notebook.index().excerpt(id) : id.folder() + QStringLiteral(" · ") + m_notebook.index().excerpt(id);
            row.insert(QStringLiteral("hlStart"), -1);
            row.insert(QStringLiteral("hlEnd"), -1);
        } else {
            const QString folder = id.folder();
            const int shift = folder.isEmpty() ? 0 : folder.size() + 3;
            detail = folder.isEmpty() ? h.detail : folder + QStringLiteral(" · ") + h.detail;
            row.insert(QStringLiteral("hlStart"), h.highlight ? h.highlight->first + shift : -1);
            row.insert(QStringLiteral("hlEnd"), h.highlight ? h.highlight->second + shift : -1);
        }
        row.insert(QStringLiteral("detail"), detail);
        m_hits.append(row);
    }
    if (m_searchMode == Notebook::Mode::Title) m_searchRanking = QStringLiteral("titles and aliases");
    else if (!m_notebook.embeddingUrl()) m_searchRanking = QStringLiteral("bm25, words alone");
    else if (m_notebook.hasVectors()) m_searchRanking = wants ? QStringLiteral("bm25 · vectors pending") : QStringLiteral("bm25 + vectors, fused");
    else m_searchRanking = QStringLiteral("bm25, no vectors yet");
    if (wants && !m_searchQuery.trimmed().isEmpty() && m_notebook.embeddingUrl() && m_notebook.hasVectors()) {
        m_pendingQuery = m_searchQuery.trimmed();
        m_queryTimer.start();
    }
}

QString Backend::createDestination() const {
    const QString folder = m_notebook.currentFolder();
    return folder.isEmpty() ? QStringLiteral("vault root") : folder + u'/';
}

void Backend::openHit(const QString &id) {
    m_searchOpen = false;
    openNote(id);
    recompute();
}

void Backend::createFromSearch() {
    const QString title = m_searchQuery.trimmed();
    if (title.isEmpty()) return;
    m_searchOpen = false;
    flushAndSave();
    QString error;
    if (m_notebook.createNoteIn(m_notebook.currentFolder(), title, &error)) { loadOpenNoteIntoEditor(); emit editorFocusRequested(); }
    else toast(ToastKind::Info, QStringLiteral("Could not create the note"), error);
    recompute();
}

// ---- toasts -------------------------------------------------------------------

int Backend::toast(ToastKind kind, const QString &title, const QString &body, const QString &primary, const QString &secondary, int ttlMs) {
    const int id = m_nextToast++;
    QString kindName;
    switch (kind) {
    case ToastKind::Success: kindName = QStringLiteral("success"); break;
    case ToastKind::Conflict: kindName = QStringLiteral("conflict"); break;
    case ToastKind::Vanished: kindName = QStringLiteral("vanished"); break;
    case ToastKind::Osd: kindName = QStringLiteral("osd"); break;
    case ToastKind::Info: kindName = QStringLiteral("info"); break;
    }
    m_toasts.append(QVariantMap{{QStringLiteral("id"), id}, {QStringLiteral("kind"), kindName}, {QStringLiteral("title"), title}, {QStringLiteral("body"), body}, {QStringLiteral("meta"), QStringLiteral("brain · now")}, {QStringLiteral("primary"), primary}, {QStringLiteral("secondary"), secondary}, {QStringLiteral("until"), ttlMs > 0 ? QDateTime::currentMSecsSinceEpoch() + ttlMs : 0}, {QStringLiteral("progress"), -1.0}});
    while (m_toasts.size() > 4) m_toasts.removeFirst();
    emit changed();
    refreshToastsSoon();
    return id;
}

void Backend::refreshToastsSoon() {
    qint64 soonest = 0;
    for (const QVariant &v : m_toasts) {
        const qint64 until = v.toMap().value(QStringLiteral("until")).toLongLong();
        if (until > 0 && (soonest == 0 || until < soonest)) soonest = until;
    }
    if (soonest == 0) return;
    m_toastTimer.start(std::max<qint64>(50, soonest - QDateTime::currentMSecsSinceEpoch()));
}

void Backend::setOsd(const QString &key, const QString &title, const QString &detail, double progress, int ttlMs) {
    for (int i = 0; i < m_toasts.size(); ++i) {
        QVariantMap t = m_toasts[i].toMap();
        if (t.value(QStringLiteral("key")).toString() != key) continue;
        t.insert(QStringLiteral("title"), title);
        t.insert(QStringLiteral("body"), detail);
        t.insert(QStringLiteral("progress"), progress);
        t.insert(QStringLiteral("until"), ttlMs > 0 ? QDateTime::currentMSecsSinceEpoch() + ttlMs : 0);
        m_toasts[i] = t;
        emit changed();
        refreshToastsSoon();
        return;
    }
    toast(ToastKind::Osd, title, detail, QString(), QString(), ttlMs);
    QVariantMap t = m_toasts.last().toMap();
    t.insert(QStringLiteral("key"), key);
    t.insert(QStringLiteral("progress"), progress);
    m_toasts[m_toasts.size() - 1] = t;
    emit changed();
}

void Backend::removeToast(int id) {
    for (int i = 0; i < m_toasts.size(); ++i) if (m_toasts[i].toMap().value(QStringLiteral("id")).toInt() == id) { m_toasts.removeAt(i); break; }
}

void Backend::dismissToast(int id) {
    removeToast(id);
    emit changed();
}

void Backend::toastAction(int id, const QString &which) {
    QVariantMap t;
    for (const QVariant &v : m_toasts) if (v.toMap().value(QStringLiteral("id")).toInt() == id) t = v.toMap();
    removeToast(id);
    const QString kind = t.value(QStringLiteral("kind")).toString();
    if (kind == u"conflict") {
        if (which == u"primary") { m_notebook.takeDiskVersion(); loadOpenNoteIntoEditor(m_editor->caret()); }
        else { m_notebook.markEdited(); }   // keep mine: the next tick writes the editor's version
    } else if (kind == u"vanished") {
        if (which == u"primary") m_notebook.restoreOpenNote();
        else { flushEditor(); m_notebook.reloadVault(); loadOpenNoteIntoEditor(); }
    }
    recompute();
}

// ---- status -------------------------------------------------------------------

void Backend::toggleStatus() {
    m_statusOpen = !m_statusOpen;
    refreshStatusInfo();
    emit changed();
}

void Backend::refreshStatusInfo() {
    const SyncStatus s = m_notebook.syncStatus();
    QVariantList rows;
    auto add = [&](const QString &k, const QString &v, const QString &c = QString()) { rows.append(QVariantMap{{QStringLiteral("k"), k}, {QStringLiteral("v"), v}, {QStringLiteral("c"), c}}); };
    add(QStringLiteral("vault"), s.vault ? tilde(*s.vault) : QStringLiteral("none"));
    add(QStringLiteral("server"), s.server.value_or(QStringLiteral("not set up — this vault stays on this machine")), s.server ? QString() : QStringLiteral("warning"));
    add(QStringLiteral("notes"), QString::number(s.notesHere) + QStringLiteral(" here · ") + QString::number(s.notesAgreed) + QStringLiteral(" agreed"));
    add(QStringLiteral("last pass"), s.lastPass ? relative(s.lastPass->secsTo(QDateTime::currentDateTime())) : QStringLiteral("not yet"));
    if (s.lastError) add(QStringLiteral("problem"), *s.lastError, QStringLiteral("negative"));
    if (s.lastChange) {
        QStringList parts;
        auto say = [&](int n, const QString &one, const QString &many) { if (n == 1) parts.append(QStringLiteral("1 ") + one); else if (n > 1) parts.append(QString::number(n) + u' ' + many); };
        say(s.lastChange->pushed, QStringLiteral("note sent"), QStringLiteral("notes sent"));
        say(s.lastChange->pulled, QStringLiteral("note received"), QStringLiteral("notes received"));
        say(s.lastChange->renamed, QStringLiteral("rename"), QStringLiteral("renames"));
        say(s.lastChange->deletedHere, QStringLiteral("removed here"), QStringLiteral("removed here"));
        say(s.lastChange->deletedThere, QStringLiteral("removed there"), QStringLiteral("removed there"));
        say(s.lastChange->conflicted, QStringLiteral("conflict copy"), QStringLiteral("conflict copies"));
        say(s.lastChange->failed, QStringLiteral("transfer failed"), QStringLiteral("transfers failed"));
        add(QStringLiteral("last change"), parts.isEmpty() ? QStringLiteral("nothing") : parts.join(QStringLiteral(", ")));
    }
    add(QStringLiteral("vectors"), QString::number(s.vectors) + QStringLiteral(" of ") + QString::number(s.notesHere) + (m_notebook.vectors().model.isEmpty() ? QString() : QStringLiteral(" · ") + m_notebook.vectors().model));
    add(QStringLiteral("embedding"), s.embeddingServer.value_or(QStringLiteral("off")), s.embeddingServer ? QString() : QStringLiteral("warning"));
    if (!m_lastEmbeddingError.isEmpty()) add(QStringLiteral("model"), m_lastEmbeddingError, QStringLiteral("warning"));
    add(QStringLiteral("machine"), m_machine);
    add(QStringLiteral("config"), tilde(Config::defaultPath()));
    m_statusInfo = QVariantMap{{QStringLiteral("rows"), rows}, {QStringLiteral("syncing"), m_syncing}, {QStringLiteral("hasServer"), s.server.has_value()}};
}

QString Backend::relative(qint64 seconds) const {
    if (seconds < 0) seconds = 0;
    if (seconds <= 90) return QStringLiteral("just now");
    if (seconds <= 5400) return QString::number((seconds + 30) / 60) + QStringLiteral(" minutes ago");
    const qint64 hours = (seconds + 1800) / 3600;
    return hours == 1 ? QStringLiteral("an hour ago") : QString::number(hours) + QStringLiteral(" hours ago");
}

// ---- workers ------------------------------------------------------------------

void Backend::scheduleCatchUp(int delayMs) {
    if (!m_notebook.hasVault()) return;
    if (m_catchingUp) { m_catchUpAgain = true; return; }
    m_catchUpTimer.start(delayMs);
}

void Backend::runCatchUp() {
    if (m_catchingUp || !m_notebook.hasVault()) return;
    const auto url = m_notebook.embeddingUrl();
    if (!url) return;
    m_catchingUp = true;
    const int generation = m_generation;
    auto [index, store] = m_notebook.catchUpInput();
    const auto shared = m_notebook.sharedVectors();
    const QString embeddingUrl = *url;
    std::thread([index = std::move(index), store = std::move(store), shared, embeddingUrl, generation]() mutable {
        QString error;
        auto llama = brain::net::Llama::connect(embeddingUrl, &error);
        if (!llama) {
            runOnMain([error]() {
                Backend *b = s_instance;
                b->m_catchingUp = false;
                b->m_lastEmbeddingError = QStringLiteral("no embedding server: ") + error;
                if (b->m_catchUpAgain) { b->m_catchUpAgain = false; b->scheduleCatchUp(CATCH_UP_DELAY_MS * 6); }
                b->recompute();
            });
            return;
        }
        std::unique_ptr<brain::net::VaultServer> server;
        if (shared) server = std::make_unique<brain::net::VaultServer>(shared->first, shared->second, llama->model());
        const int total = brain::semantic::plan(store, brain::semantic::wanted(index)).embed.size();
        int lastShown = -1;
        const auto progress = [&](int done, int all) {
            if (all < 3 || done == lastShown) return;
            lastShown = done;
            runOnMain([done, all]() { s_instance->setOsd(QStringLiteral("embedding"), QStringLiteral("Embedding"), QString::number(done) + u'/' + QString::number(all), double(done) / std::max(1, all), done >= all ? 1500 : 0); });
        };
        brain::semantic::Store next = store;
        const brain::semantic::Report report = brain::semantic::catchUp(next, index, *llama, server.get(), total >= 3 ? progress : brain::semantic::Progress());
        runOnMain([next = std::move(next), report, generation]() mutable {
            Backend *b = s_instance;
            b->m_catchingUp = false;
            b->m_lastEmbeddingError.clear();
            if (generation == b->m_generation) b->m_notebook.absorbVectors(next);
            if (report.pending > 0) b->scheduleCatchUp(CATCH_UP_DELAY_MS * 6);
            else if (b->m_catchUpAgain) { b->m_catchUpAgain = false; b->scheduleCatchUp(CATCH_UP_DELAY_MS); }
            if (b->m_searchOpen) b->refreshHits();
            b->recompute();
        });
    }).detach();
}

void Backend::runQueryEmbedding(const QString &query) {
    const auto url = m_notebook.embeddingUrl();
    if (!url) return;
    const QString embeddingUrl = *url;
    std::thread([query, embeddingUrl]() {
        QString error;
        auto llama = brain::net::Llama::connect(embeddingUrl, &error);
        if (!llama) return;
        brain::semantic::EmbedError problem;
        const auto vector = llama->embedQuery(query, &problem);
        if (!vector) return;
        runOnMain([query, vector = *vector]() {
            Backend *b = s_instance;
            b->m_notebook.setQueryVector(query, vector);
            if (b->m_searchOpen && b->m_searchQuery.trimmed() == query) { b->refreshHits(); emit b->changed(); }
        });
    }).detach();
}

void Backend::syncNow() {
    runSync();
    refreshStatusInfo();
    emit changed();
}

void Backend::runSync() {
    if (m_syncing) return;
    const auto server = m_notebook.syncServer();
    if (!server) return;
    const auto input = m_notebook.syncInput();
    if (!input) return;
    flushEditor();
    m_notebook.saveNow();
    m_syncing = true;
    const int generation = m_generation;
    const QString from = m_machine, date = today();
    std::thread([vault = input->first, base = input->second, server = *server, from, date, generation]() {
        brain::net::VaultServer remote(server.first, server.second);
        QString error;
        auto incoming = brain::sync::gather(vault, base, remote, from, date, &error);
        runOnMain([incoming = std::move(incoming), error, from, date, generation]() mutable {
            Backend *b = s_instance;
            b->m_syncing = false;
            if (generation != b->m_generation) return;
            if (!incoming) { b->m_notebook.recordSyncFailure(error); b->recompute(); return; }
            b->flushEditor();
            const brain::sync::Report report = b->m_notebook.absorbSync(std::move(*incoming), from, date);
            b->m_syncedAt = QTime::currentTime().toString(QStringLiteral("HH:mm"));
            if (!report.isQuiet()) {
                QStringList parts;
                auto say = [&](int n, const QString &what) { if (n > 0) parts.append(QString::number(n) + u' ' + what); };
                say(report.pushed, QStringLiteral("pushed"));
                say(report.pulled, QStringLiteral("pulled"));
                say(report.renamed, QStringLiteral("renamed"));
                say(report.deletedHere + report.deletedThere, QStringLiteral("removed"));
                QString body = parts.isEmpty() ? QStringLiteral("Nothing moved.") : parts.join(QStringLiteral(", ")) + u'.';
                body += report.conflicted > 0 ? QStringLiteral(" %1 conflict %2 beside the original.").arg(report.conflicted).arg(report.conflicted == 1 ? QStringLiteral("copy landed") : QStringLiteral("copies landed")) : QStringLiteral(" Nothing conflicted.");
                b->toast(report.conflicted > 0 ? ToastKind::Conflict : ToastKind::Success, QStringLiteral("Vault synced"), body, QString(), QString(), report.conflicted > 0 ? 12000 : 5000);
                b->loadOpenNoteIntoEditor(b->m_editor->caret());
                b->watchVault();
                b->scheduleCatchUp(CATCH_UP_DELAY_MS);
            }
            b->recompute();
        });
    }).detach();
}

// ---- CLI verbs ----------------------------------------------------------------

QString Backend::handleVerb(const QStringList &args) {
    if (args.isEmpty()) { emit windowRequested(); return QString(); }
    const QString verb = args.first();
    if (verb == u"capture") { emit windowRequested(); openCapture(); return QStringLiteral("capture opened"); }
    if (verb == u"search") { emit windowRequested(); openSearch(args.size() > 1 ? args[1] : QStringLiteral("text")); return QStringLiteral("search opened"); }
    if (verb == u"sync") { runSync(); return QStringLiteral("sync pass started"); }
    if (verb == u"status") {
        refreshStatusInfo();
        QStringList lines;
        for (const QVariant &v : m_statusInfo.value(QStringLiteral("rows")).toList()) { const QVariantMap r = v.toMap(); lines.append(r.value(QStringLiteral("k")).toString() + QStringLiteral(": ") + r.value(QStringLiteral("v")).toString()); }
        return lines.join(u'\n');
    }
    if (verb == u"config") {
        Config &c = m_notebook.config();
        auto field = [&](const QString &key) -> std::optional<QString> * {
            if (key == u"sync_url") return &c.syncUrl;
            if (key == u"sync_token") return &c.syncToken;
            if (key == u"vectors_url") return &c.vectorsUrl;
            if (key == u"vectors_token") return &c.vectorsToken;
            if (key == u"embedding_url") return &c.embeddingUrl;
            if (key == u"vault") return &c.vault;
            return nullptr;
        };
        if (args.size() < 2) return QStringLiteral("usage: brain config <key> [value]  (sync_url, sync_token, vectors_url, vectors_token, embedding_url, vault)");
        std::optional<QString> *slot = field(args[1]);
        if (!slot) return QStringLiteral("unknown key ") + args[1];
        if (args.size() == 2) return slot->has_value() ? **slot : QStringLiteral("(unset)");
        if (args[2] == u"token" && args.size() > 3) *slot = args[3]; else *slot = args[2];
        m_notebook.saveConfig();
        if (args[1] == u"vault") chooseVault(**slot);
        scheduleCatchUp(1000);
        QTimer::singleShot(1000, this, &Backend::runSync);
        recompute();
        return args[1] + QStringLiteral(" set");
    }
    emit windowRequested();
    return QStringLiteral("unknown verb ") + verb;
}

void Backend::act(const QString &name) {
    if (name == u"capture") openCapture();
    else if (name == u"insert") { m_insertOpen = true; m_insertQuery = QStringLiteral("ta"); refreshInsert(); emit changed(); }
    else if (name == u"search") { openSearch(QStringLiteral("text")); setSearchQuery(QStringLiteral("why is my bread so flat")); }
    else if (name == u"titles") { openSearch(QStringLiteral("titles")); setSearchQuery(QStringLiteral("so")); }
    else if (name == u"reading") { if (!m_reading) toggleReading(); }
    else if (name == u"conflict") toast(ToastKind::Conflict, QStringLiteral("Sourdough.md changed on disk"), QStringLiteral("Your edits are still here. Keep them, or take the file."), QStringLiteral("Take the file"), QStringLiteral("Keep mine"), 0);
    else if (name == u"synced") toast(ToastKind::Success, QStringLiteral("Vault synced"), QStringLiteral("4 notes pushed, 1 pulled. Nothing conflicted."), QString(), QString(), 0);
    else if (name == u"embedding") setOsd(QStringLiteral("embedding"), QStringLiteral("Embedding"), QStringLiteral("128/500"), 0.256, 0);
    else if (name == u"status") { m_statusOpen = true; refreshStatusInfo(); emit changed(); }
    else if (name == u"prompt") openPrompt(QStringLiteral("rename-note"), QStringLiteral("Rename note"), QStringLiteral("Sourdough"), QStringLiteral("Rename"), QString());
    else if (name == u"tags") setScreen(QStringLiteral("tags"));
    else if (name == u"inbox") setScreen(QStringLiteral("inbox"));
    else if (name.startsWith(QStringLiteral("open:"))) openNote(name.mid(5));
}

// ---- recompute ----------------------------------------------------------------

void Backend::recompute() {
    const Index &index = m_notebook.index();
    const int total = index.size();
    int inbox = 0;
    for (const NoteId &id : index.ids()) if (id.folder().isEmpty()) ++inbox;
    const auto tags = index.tags();

    struct Screen { const char *id; const char *label; int count; const char *tone; };
    const Screen screens[] = {{"notes", "Notes", total, "muted"}, {"tags", "Tags", int(tags.size()), "muted"}, {"inbox", "Inbox", inbox, inbox > 0 ? "warning" : "muted"}};
    QVariantList nav;
    int i = 0;
    for (const Screen &s : screens) {
        ++i;
        nav.append(QVariantMap{{QStringLiteral("id"), QLatin1String(s.id)}, {QStringLiteral("label"), QLatin1String(s.label)}, {QStringLiteral("key"), QString::number(i)}, {QStringLiteral("count"), QString::number(s.count)}, {QStringLiteral("tone"), QLatin1String(s.tone)}, {QStringLiteral("active"), m_screen == QLatin1String(s.id)}});
    }
    m_nav = nav;
    m_vaultLabel = m_notebook.hasVault() ? tilde(m_notebook.vaultRoot()) + QStringLiteral(" · ") + QString::number(total) + (total == 1 ? QStringLiteral(" note") : QStringLiteral(" notes")) : QStringLiteral("no vault yet");

    refreshTree();

    // The open note.
    const Note *note = m_notebook.openNote();
    const std::optional<NoteId> openId = m_notebook.openNoteId();
    m_backlinks.clear();
    if (note && openId) {
        m_noteTitle = openId->title();
        const int words = m_editor->wordCount();
        const auto [modified, created] = m_notebook.vault()->times(*openId);
        const QString folder = openId->folder();
        QStringList sub;
        sub.append(folder.isEmpty() ? QStringLiteral("vault") : folder);
        if (modified) sub.append(QStringLiteral("updated ") + QDateTime::fromSecsSinceEpoch(modified).toString(QStringLiteral("yyyy-MM-dd")));
        sub.append(QString::number(words) + (words == 1 ? QStringLiteral(" word") : QStringLiteral(" words")));
        m_noteSubtitle = sub.join(QStringLiteral(" · "));
        const bool dirty = m_notebook.isDirty() || m_editor->source() != note->body;
        const Alert alert = m_notebook.alert();
        QString onDisk = dirty ? QStringLiteral("unsaved") : QStringLiteral("clean");
        QString onDiskTone = dirty ? QStringLiteral("warning") : QStringLiteral("positive");
        if (alert.kind == Alert::Diverged) { onDisk = QStringLiteral("changed outside"); onDiskTone = QStringLiteral("negative"); }
        else if (alert.kind == Alert::Vanished) { onDisk = QStringLiteral("gone"); onDiskTone = QStringLiteral("negative"); }
        else if (alert.kind == Alert::NotSaving) { onDisk = QStringLiteral("not saving"); onDiskTone = QStringLiteral("negative"); }
        QVariantList tagChips;
        for (const QString &tag : index.tagsOf(*openId)) tagChips.append(tag);
        QStringList fmTags = note->frontmatter ? note->frontmatter->tags : QStringList();
        QStringList aliases = note->aliases();
        const QString createdText = note->frontmatter && note->frontmatter->created ? note->frontmatter->created->toString(QStringLiteral("yyyy-MM-dd")) : (created ? QDateTime::fromSecsSinceEpoch(created).toString(QStringLiteral("yyyy-MM-dd")) : QStringLiteral("—"));
        const QString updatedText = note->frontmatter && note->frontmatter->updated ? note->frontmatter->updated->toString(QStringLiteral("yyyy-MM-dd")) : (modified ? QDateTime::fromSecsSinceEpoch(modified).toString(QStringLiteral("yyyy-MM-dd")) : QStringLiteral("—"));
        m_rail = QVariantMap{{QStringLiteral("folder"), folder.isEmpty() ? QStringLiteral("—") : folder}, {QStringLiteral("words"), QString::number(words)}, {QStringLiteral("created"), createdText}, {QStringLiteral("updated"), updatedText}, {QStringLiteral("aliases"), aliases.join(QStringLiteral(", "))}, {QStringLiteral("fmTags"), fmTags.join(QStringLiteral(", "))}, {QStringLiteral("onDisk"), onDisk}, {QStringLiteral("onDiskTone"), onDiskTone}, {QStringLiteral("tags"), tagChips}};
        for (const Backlink &b : m_notebook.backlinksOfOpenNote())
            m_backlinks.append(QVariantMap{{QStringLiteral("id"), b.from.str()}, {QStringLiteral("title"), b.from.title()}, {QStringLiteral("initial"), initialOf(b.from.title())}, {QStringLiteral("context"), b.context}});
        QStringList left;
        left.append(m_reading ? QStringLiteral("reading") : QStringLiteral("editing"));
        left.append(QString::number(words) + QStringLiteral(" words"));
        left.append(QString::number(m_backlinks.size()) + (m_backlinks.size() == 1 ? QStringLiteral(" backlink") : QStringLiteral(" backlinks")));
        if (m_notebook.embeddingUrl()) left.append(QStringLiteral("vectors ") + QString::number(m_notebook.vectors().size()) + u'/' + QString::number(total));
        else left.append(QStringLiteral("words alone"));
        m_statusLeft = left.join(QStringLiteral(" · "));
    } else {
        m_noteTitle = m_notebook.hasVault() ? QStringLiteral("No note open") : QStringLiteral("Brain");
        m_noteSubtitle = m_notebook.hasVault() ? QStringLiteral("pick one on the left, or Ctrl+N") : QStringLiteral("choose a folder to keep notes in");
        m_rail.clear();
        m_statusLeft = QString::number(total) + QStringLiteral(" notes") + (m_notebook.embeddingUrl() ? QStringLiteral(" · vectors ") + QString::number(m_notebook.vectors().size()) + u'/' + QString::number(total) : QString());
    }
    const auto server = m_notebook.syncServer();
    const SyncStatus status = m_notebook.syncStatus();
    if (!server) { m_statusRight = QStringLiteral("not synced"); m_statusDot = QStringLiteral("faint"); }
    else if (m_syncing) { m_statusRight = QStringLiteral("syncing"); m_statusDot = QStringLiteral("warning"); }
    else if (status.lastError) { m_statusRight = QStringLiteral("sync failed"); m_statusDot = QStringLiteral("negative"); }
    else if (!m_syncedAt.isEmpty()) { m_statusRight = QStringLiteral("synced ") + m_syncedAt; m_statusDot = QStringLiteral("positive"); }
    else { m_statusRight = QStringLiteral("sync pending"); m_statusDot = QStringLiteral("faint"); }
    if (m_statusOpen) refreshStatusInfo();
    emit changed();
}

void Backend::refreshTree() {
    m_rows.clear();
    m_tagRows.clear();
    const std::optional<NoteId> openId = m_notebook.openNoteId();
    const qint64 now = QDateTime::currentSecsSinceEpoch();
    auto age = [&](qint64 modified) -> QString {
        if (modified <= 0) return QString();
        const qint64 s = std::max<qint64>(0, now - modified);
        if (s < 3600) return QString::number(std::max<qint64>(1, s / 60)) + u'm';
        if (s < 86400) return QString::number(s / 3600) + u'h';
        if (s < 7 * 86400) return QString::number(s / 86400) + u'd';
        if (s < 30 * 86400) return QString::number(s / (7 * 86400)) + u'w';
        if (s < 365 * 86400) return QString::number(s / (30 * 86400)) + QStringLiteral("mo");
        return QString::number(s / (365 * 86400)) + u'y';
    };
    auto noteRow = [&](const NoteId &id, const QString &excerpt, int depth, qint64 modified) {
        return QVariantMap{{QStringLiteral("kind"), QStringLiteral("note")}, {QStringLiteral("id"), id.str()}, {QStringLiteral("name"), id.title()}, {QStringLiteral("excerpt"), excerpt}, {QStringLiteral("depth"), depth}, {QStringLiteral("meta"), age(modified)}, {QStringLiteral("selected"), openId && *openId == id}, {QStringLiteral("folder"), id.folder()}};
    };
    if (m_screen == u"notes" || (m_screen == u"tags" && m_notebook.activeTag())) {
        if (m_notebook.isSearching()) {
            m_sidebarLabel = QStringLiteral("matches");
            for (const auto &[id, excerpt] : m_notebook.searchResults()) {
                const auto [modified, created] = m_notebook.vault() ? m_notebook.vault()->times(id) : std::pair<qint64, qint64>{0, 0};
                QVariantMap row = noteRow(id, excerpt, 0, modified);
                row.insert(QStringLiteral("showExcerpt"), true);
                m_rows.append(row);
            }
        } else {
            m_sidebarLabel = m_notebook.activeTag() ? u'#' + *m_notebook.activeTag() : QStringLiteral("Vault");
            bool rootNotesStarted = false;
            for (const tree::Row &row : m_notebook.sidebarRows()) {
                if (row.folder) {
                    m_rows.append(QVariantMap{{QStringLiteral("kind"), QStringLiteral("folder")}, {QStringLiteral("path"), row.path}, {QStringLiteral("name"), row.name}, {QStringLiteral("depth"), row.depth}, {QStringLiteral("meta"), QString::number(row.notes)}, {QStringLiteral("expanded"), row.expanded}, {QStringLiteral("gap"), false}});
                } else {
                    QVariantMap r = noteRow(row.id, row.excerpt, row.depth, row.modified);
                    // Root-level notes sit after the folders with a small gap.
                    const bool gap = row.depth == 0 && !rootNotesStarted && !m_rows.isEmpty();
                    if (row.depth == 0) rootNotesStarted = true;
                    r.insert(QStringLiteral("gap"), gap);
                    m_rows.append(r);
                }
            }
        }
    } else if (m_screen == u"inbox") {
        m_sidebarLabel = QStringLiteral("Unfiled");
        for (const auto &[id, excerpt] : m_notebook.listedNotes()) {
            if (!id.folder().isEmpty()) continue;
            const auto [modified, created] = m_notebook.vault() ? m_notebook.vault()->times(id) : std::pair<qint64, qint64>{0, 0};
            m_rows.append(noteRow(id, excerpt, 0, modified));
        }
    } else {
        m_sidebarLabel = QStringLiteral("Tags");
    }
    const QString active = m_notebook.activeTag().value_or(QString());
    for (const auto &[tag, count] : m_notebook.tags()) {
        const int depth = tag.count(u'/');
        m_tagRows.append(QVariantMap{{QStringLiteral("name"), tag}, {QStringLiteral("leaf"), tag.section(u'/', -1)}, {QStringLiteral("depth"), depth}, {QStringLiteral("count"), QString::number(count)}, {QStringLiteral("active"), tag == active}});
    }
}
